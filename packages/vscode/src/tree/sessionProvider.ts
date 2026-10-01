import * as vscode from 'vscode';
import * as fs from 'fs';
import * as path from 'path';
import {
  decodeProjectPath,
  getClaudeProjectsDir,
  listProjectDirNames,
  listSessionFiles,
  readSessionMeta,
} from '@session-deck/core';
import { CopilotSessionRow, getCopilotHomeDir, listCopilotSessions } from '@session-deck/core';
import { arrangeProjects, folderNodeKey, FolderPrefs, projectNodeKey, SessionCategory, sortSessions } from '@session-deck/core';
import { resolveProjectRoot } from '@session-deck/core';
import { normalizeFsPath, isInside } from '@session-deck/core';
import { mapWithConcurrency } from '@session-deck/core';
import { ensureSessionStatusDir, getSessionStatusDir, readEffectiveSessionStatus, SessionStatusRecord } from '@session-deck/core';
import { readDeckConfig, readLatestRateLimitUsage, readSessionUsage, readSevenDayDailySpend } from '@session-deck/core';
import { DailySpend, formatResetTime, renderUsageBar, UsageMetric, usageSeverity, USAGE_METRIC_LABELS, UsageSeverity } from '@session-deck/core';
import { sessionStatusUri } from '../status/sessionStatusDecorationProvider';
import { DeckState } from '../config/state';
import { readWorkspaceProjectEntries } from '../config/workspaceConfig';
import { selectSessionsToArchive, selectSessionsToStop } from '@session-deck/core';
import { agentIconPath } from './agentIcons';

export type AgentType = 'claude' | 'copilot';

/** Fixed render order when both agents have sessions — see `getRootNodes`. */
const AGENTS: readonly AgentType[] = ['claude', 'copilot'];

/** How many of a project's most recent sessions the main tree shows by default. Overridable via `maxSessionsShown`. */
const MAX_SESSIONS_PER_PROJECT_VIEW = 5;

/** Idle threshold (in minutes) past which `stopIdleSessions` closes a session's terminal. Overridable via `stopIdleMinutes`. */
const DEFAULT_STOP_IDLE_MINUTES = 30;

/** Caps concurrent `git` processes `discoverGroups` spawns when resolving cwds to project roots. */
const GIT_RESOLVE_CONCURRENCY = 8;

/** A named swatch prefixed onto the label as plain text (a `ThemeColor`'d icon washes out when selected). `emoji` wins if both are set. */
const COLOR_SWATCH_EMOJI: Record<string, string> = {
  red: '🔴',
  orange: '🟠',
  yellow: '🟡',
  green: '🟢',
  blue: '🔵',
  purple: '🟣',
  brown: '🟤',
  black: '⚫',
  white: '⚪',
};

interface ProjectMember {
  dirName: string;
  cwd: string;
}

interface DiscoveredGroup {
  /** The root exactly as git (or the raw cwd, for non-git folders) resolved it — nicely cased, used for display defaults. */
  canonicalRoot: string;
  members: ProjectMember[];
}

/** Only present when more than one agent has any sessions — wraps that agent's Project → Session subtree. */
export class AgentFolderNode {
  readonly kind = 'agentFolder' as const;
  constructor(public readonly agent: AgentType) {}
}

/** A Session Deck folder (shared with the terminal UI) holding some of this agent's projects. */
export class FolderNode {
  readonly kind = 'folder' as const;
  constructor(
    public readonly agent: AgentType,
    public readonly folder: FolderPrefs,
    public readonly projects: ProjectGroupNode[]
  ) {}
}

export class ProjectGroupNode {
  readonly kind = 'project' as const;
  constructor(
    /** The same `rootPath` can have a separate node per agent, each with its own cap/archive bucket. */
    public readonly agent: AgentType,
    /** Exactly as written in `.vscode/session-deck.json` — the stable identity used for renames/removal. */
    public readonly rootPath: string,
    public readonly displayName: string,
    /** Distinct working-directory variants for this (agent, project); >1 usually means worktrees. */
    public readonly members: ProjectMember[],
    /** Total sessions for this agent, before the `maxSessionsShown` cap. */
    public readonly totalSessionCount: number,
    public readonly maxSessionsShown: number = MAX_SESSIONS_PER_PROJECT_VIEW,
    public readonly emoji?: string,
    public readonly color?: string,
    /** Opt-in: auto-close a session's terminal once idle past `stopIdleMinutes`. See `WorkspaceProjectEntry.stopIdleSessions`. */
    public readonly stopIdleSessions: boolean = false,
    public readonly stopIdleMinutes: number = DEFAULT_STOP_IDLE_MINUTES
  ) {}
}

export class SessionNode {
  readonly kind = 'session' as const;
  constructor(
    public readonly agent: AgentType,
    public readonly sessionId: string,
    public readonly displayName: string,
    public readonly lastModified: Date,
    public readonly cwd: string,
    public readonly projectRoot: string,
    /** Claude-only: the `~/.claude/projects/<dirName>` folder this session's `.jsonl` lives in. */
    public readonly projectDirName?: string,
    /** Claude-only: Copilot sessions live in the shared `session-store.db`, read by id instead. */
    public readonly filePath?: string
  ) {}
}

/** A project's "Archived" bucket — only ever shown as a child of a project that has at least one archived session. */
export class ArchiveFolderNode {
  readonly kind = 'archiveFolder' as const;
  constructor(
    public readonly project: ProjectGroupNode,
    public readonly archivedCount: number
  ) {}
}

/** One toggleable usage row (Context / 5h / 7d), pinned to the bottom of the tree — see `UsageViewController`. */
export class UsageNode {
  readonly kind = 'usage' as const;
  constructor(
    public readonly metric: UsageMetric,
    public readonly percent: number | undefined,
    public readonly updatedAt: number | undefined,
    /** `fiveHour` only. */
    public readonly resetsAt: number | undefined,
    public readonly hasFocusedSession: boolean
  ) {}
}

/** The "Rest of week" row below 7d — this week's day-by-day spend against the 7d quota, not a usage reading of its own. */
export class UsageBudgetNode {
  readonly kind = 'usageBudget' as const;
  constructor(
    /** Monday through today, weekends included; empty when nothing's been recorded yet. */
    public readonly days: DailySpend[]
  ) {}
}

/** Who's "focused" (for the Context % row) and whether the usage row should show at all — supplied by `extension.ts`, which owns both the active terminal and the show/hide toggle. */
export interface UsageViewController {
  isVisible(): boolean;
  getFocusedSessionId(): string | undefined;
}

export type ClaudeDeckNode =
  | AgentFolderNode
  | FolderNode
  | ProjectGroupNode
  | SessionNode
  | ArchiveFolderNode
  | UsageNode
  | UsageBudgetNode;

/** The project's key in the shared tree prefs, same as the terminal UI's (`normalizeFsPath` of the root). */
export function projectKeyOf(node: ProjectGroupNode): string {
  return normalizeFsPath(node.rootPath);
}

/** Maps the extension's status files onto the categories shared with the terminal UI (for the "actionable" sort). */
function categoryOf(sessionId: string): SessionCategory {
  const status = readEffectiveSessionStatus(sessionId)?.status;
  return status === 'running' || status === 'waiting' || status === 'error' ? status : status === 'done' ? 'idle' : 'stopped';
}

export interface SessionWithProject {
  session: SessionNode;
  projectDisplayName: string;
}

export class SessionTreeProvider implements vscode.TreeDataProvider<ClaudeDeckNode> {
  private readonly _onDidChangeTreeData = new vscode.EventEmitter<ClaudeDeckNode | undefined | void>();
  readonly onDidChangeTreeData = this._onDidChangeTreeData.event;

  /** Session ids seen for each (agent, project) pair on the previous pass, keyed by `` `${agent}:${normalizeFsPath(rootPath)}` `` — see `enforceArchiveCap`. */
  private readonly knownSessionIdsByProject = new Map<string, Set<string>>();

  constructor(
    private readonly state: DeckState,
    private readonly extensionUri: vscode.Uri,
    /** Called with the ids of any sessions `enforceArchiveCap` just auto-archived, so their terminals (if open) can be closed. */
    private readonly onSessionsArchived?: (sessionIds: string[]) => void,
    /** Called with the ids of any sessions `enforceIdleStop` just found idle past their project's threshold, so their terminals can be closed. Independent of archiving/pinning. */
    private readonly onSessionsIdleStopped?: (sessionIds: string[]) => void,
    /** Supplies the bottom usage row's visibility and focused-session id. Omitted entirely in tests that don't care about it. */
    private readonly usageView?: UsageViewController
  ) {}

  refresh(): void {
    this._onDidChangeTreeData.fire();
  }

  /** Best-effort live refresh — recursive fs.watch isn't supported on every platform. */
  watch(context: vscode.ExtensionContext, onStatusChanged?: () => void): void {
    const root = getClaudeProjectsDir();
    if (fs.existsSync(root)) {
      try {
        const watcher = fs.watch(root, { recursive: true }, () => this.refresh());
        context.subscriptions.push({ dispose: () => watcher.close() });
      } catch {
        // Silently fall back to manual refresh.
      }
    }

    // Copilot writes new sessions to session-store.db-wal (WAL mode) before it's checkpointed
    // into the main file, so both need watching, not just session-store.db.
    for (const filename of ['session-store.db', 'session-store.db-wal']) {
      const filePath = path.join(getCopilotHomeDir(), filename);
      if (fs.existsSync(filePath)) {
        try {
          const watcher = fs.watch(filePath, () => this.refresh());
          context.subscriptions.push({ dispose: () => watcher.close() });
        } catch {
          // Silently fall back to manual refresh.
        }
      }
    }

    // Repaints session status dots live as claudeProcessWatcher.ts/copilotStatusWatcher.ts write them.
    ensureSessionStatusDir();
    try {
      const statusWatcher = fs.watch(getSessionStatusDir(), () => {
        this.refresh();
        onStatusChanged?.();
      });
      context.subscriptions.push({ dispose: () => statusWatcher.close() });
    } catch {
      // Silently fall back to manual refresh.
    }
  }

  getTreeItem(element: ClaudeDeckNode): vscode.TreeItem {
    if (element.kind === 'agentFolder') {
      const item = new vscode.TreeItem(agentLabel(element.agent), vscode.TreeItemCollapsibleState.Expanded);
      item.contextValue = 'sessionDeckAgentFolder';
      item.iconPath = agentIconPath(this.extensionUri, element.agent);
      return item;
    }

    if (element.kind === 'folder') {
      const item = new vscode.TreeItem(element.folder.name, this.collapsibleState(folderNodeKey(element.folder.id)));
      item.contextValue = 'sessionDeckFolder';
      item.iconPath = new vscode.ThemeIcon('folder');
      item.description = `${element.projects.length} project${element.projects.length === 1 ? '' : 's'}`;
      item.tooltip = 'Session Deck folder (shared with the terminal UI)';
      return item;
    }

    if (element.kind === 'project') {
      const swatch = element.emoji ?? (element.color ? COLOR_SWATCH_EMOJI[element.color] : undefined);
      const item = new vscode.TreeItem(
        swatch ? `${swatch} ${element.displayName}` : element.displayName,
        this.collapsibleState(projectNodeKey(projectKeyOf(element)))
      );
      item.contextValue = 'sessionDeckProject';
      item.iconPath = new vscode.ThemeIcon('briefcase');
      const tooltipLines = [
        element.rootPath,
        element.members.length > 1
          ? `(${element.members.length} linked working directories, incl. worktrees)`
          : '',
      ];
      if (element.totalSessionCount > element.maxSessionsShown) {
        tooltipLines.push(`Showing the ${element.maxSessionsShown} most recent of ${element.totalSessionCount} sessions.`);
      }
      item.tooltip = tooltipLines.filter(Boolean).join('\n');
      return item;
    }

    if (element.kind === 'archiveFolder') {
      const item = new vscode.TreeItem(`Archived (${element.archivedCount})`, vscode.TreeItemCollapsibleState.Collapsed);
      item.contextValue = 'sessionDeckArchiveFolder';
      item.iconPath = new vscode.ThemeIcon('file-zip');
      item.tooltip = `Sessions auto-archived once "${element.project.displayName}" had more than ${element.project.maxSessionsShown} active sessions, or archived by hand.`;
      return item;
    }

    if (element.kind === 'usage') {
      return this.usageTreeItem(element);
    }

    if (element.kind === 'usageBudget') {
      return this.usageBudgetTreeItem(element);
    }

    const archived = this.state.isSessionArchived(element.sessionId);
    const pin = archived ? undefined : this.state.getSessionPin(element.sessionId);
    const item = new vscode.TreeItem(pin ? `📌 ${element.displayName}` : element.displayName, vscode.TreeItemCollapsibleState.None);
    const relSubpath = relativeSubpath(element.projectRoot, element.cwd);
    item.description = relSubpath ? `${relSubpath} · ${timeAgo(element.lastModified)}` : timeAgo(element.lastModified);
    const status = readEffectiveSessionStatus(element.sessionId);
    item.tooltip = [`${element.cwd}`, element.sessionId, element.filePath ?? '', statusTooltipLine(status), pin ? `Pinned to the ${pin} of the project` : '']
      .filter(Boolean)
      .join('\n');
    item.iconPath = agentIconPath(this.extensionUri, element.agent);
    // Status color lives on a FileDecoration, not this icon — a ThemeColor'd icon washes out when selected.
    item.resourceUri = sessionStatusUri(element.sessionId);
    item.contextValue = archived ? 'sessionDeckArchivedSession' : 'sessionDeckSession';
    item.command = {
      command: 'sessionDeck.openSession',
      title: 'Open Session',
      arguments: [element],
    };
    return item;
  }

  async getChildren(element?: ClaudeDeckNode): Promise<ClaudeDeckNode[]> {
    if (!element) {
      return this.getRootNodes();
    }
    if (element.kind === 'agentFolder') {
      return this.getTopLevel(element.agent);
    }
    if (element.kind === 'folder') {
      return element.projects;
    }
    if (element.kind === 'project') {
      const candidates = await this.gatherCandidates(element);
      const active = candidates.filter((c) => !this.state.isSessionArchived(c.sessionId));
      const archivedCount = candidates.length - active.length;
      // Anything beyond the cap moves to "Archived" (see enforceArchiveCap); listAllSessions()
      // (Search, Open Sessions) is uncapped, so nothing is ever unfindable. Pinned sessions don't
      // count toward the cap.
      const pinned = active.filter((c) => this.state.getSessionPin(c.sessionId));
      const unpinned = active.filter((c) => !this.state.getSessionPin(c.sessionId)).slice(0, element.maxSessionsShown);
      const nodes = await this.toSessionNodes([...pinned, ...unpinned], element);
      const sessions = sortSessions(
        nodes,
        this.state.getTree().sort,
        (n) => this.state.getSessionPin(n.sessionId),
        (n) => n.lastModified.getTime(),
        (n) => categoryOf(n.sessionId)
      );
      return archivedCount > 0 ? [...sessions, new ArchiveFolderNode(element, archivedCount)] : sessions;
    }
    if (element.kind === 'archiveFolder') {
      const candidates = await this.gatherCandidates(element.project);
      const archived = candidates.filter((c) => this.state.isSessionArchived(c.sessionId));
      return this.toSessionNodes(archived, element.project);
    }
    return [];
  }

  /** An `AgentFolderNode` per agent that has any sessions at all, or straight to the flat project list when only one agent qualifies — plus the usage row, pinned last, when toggled on. */
  private async getRootNodes(): Promise<ClaudeDeckNode[]> {
    const agentsInUse = this.detectAgentsInUse();
    const nodes: ClaudeDeckNode[] =
      agentsInUse.length <= 1
        ? await this.getTopLevel(agentsInUse[0] ?? 'claude')
        : AGENTS.filter((a) => agentsInUse.includes(a)).map((a) => new AgentFolderNode(a));
    if (this.usageView?.isVisible()) {
      const focusedSessionId = this.usageView.getFocusedSessionId();
      const hasFocusedSession = focusedSessionId !== undefined;
      // Context is specific to the focused session's own transcript; 5h/7d are account-wide, so they're
      // read from whichever session's statusLine reported most recently, not tied to what's focused.
      const sessionUsage = focusedSessionId ? readSessionUsage(focusedSessionId) : undefined;
      const rateLimit = readLatestRateLimitUsage();
      nodes.push(new UsageNode('context', sessionUsage?.contextPercent, sessionUsage?.updatedAt, undefined, hasFocusedSession));
      nodes.push(new UsageNode('fiveHour', rateLimit?.fiveHourPercent, rateLimit?.updatedAt, rateLimit?.fiveHourResetsAt, hasFocusedSession));
      nodes.push(new UsageNode('sevenDay', rateLimit?.sevenDayPercent, rateLimit?.updatedAt, rateLimit?.sevenDayResetsAt, hasFocusedSession));
      nodes.push(new UsageBudgetNode(readSevenDayDailySpend()));
    }
    return nodes;
  }

  /** `—` for a field with no data yet (never resumed since `statusline-command.sh` started recording it, or no session focused). */
  private usageTreeItem(element: UsageNode): vscode.TreeItem {
    const item = new vscode.TreeItem(USAGE_METRIC_LABELS[element.metric], vscode.TreeItemCollapsibleState.None);
    item.contextValue = 'sessionDeckUsage';

    // `ui.use24HourClock` is shared with the terminal UI's config popup (`C`), there's no VS Code setting of its own.
    // sevenDay's reset can be days out, so its label is prefixed with the weekday — fiveHour's never needs it.
    const resetLabel = element.resetsAt
      ? formatResetTime(element.resetsAt, readDeckConfig().ui.use24HourClock, element.metric === 'sevenDay')
      : undefined;

    if (element.percent === undefined) {
      item.description = '—';
      item.iconPath = new vscode.ThemeIcon('dash');
    } else {
      item.description = `${renderUsageBar(element.percent)} ${element.percent}%${resetLabel ? ` (${resetLabel})` : ''}`;
      item.iconPath = new vscode.ThemeIcon('circle-large-filled', usageColor(element.metric, element.percent));
    }

    const tooltipLines = [
      element.metric === 'context' && !element.hasFocusedSession
        ? 'No session focused — open a Claude session and select its terminal to see its usage.'
        : undefined,
      resetLabel ? `Resets ${resetLabel}` : undefined,
      element.updatedAt
        ? `Updated ${timeAgo(new Date(element.updatedAt))}`
        : element.metric === 'context'
          ? element.hasFocusedSession
            ? 'No usage data recorded yet for this session.'
            : undefined
          // 5h/7d are account-wide — no session needs to be focused for these to have data.
          : 'No usage data recorded yet — open any Claude session.',
    ];
    item.tooltip = tooltipLines.filter((line): line is string => Boolean(line)).join('\n');
    return item;
  }

  /** This week's (Monday through today, weekends included) day-by-day spend against the 7d quota — not a budget, a record, so you can do the "how much is left" math yourself. */
  private usageBudgetTreeItem(element: UsageBudgetNode): vscode.TreeItem {
    const item = new vscode.TreeItem('This week', vscode.TreeItemCollapsibleState.None);
    item.contextValue = 'sessionDeckUsage';
    item.iconPath = new vscode.ThemeIcon('calendar');

    item.description =
      element.days.length === 0 ? '—' : element.days.map((d) => `${d.label} ${Math.round(d.percent)}%`).join(' · ');

    const tooltipLines = [
      element.days.length === 0 ? 'No usage data recorded yet — open any Claude session.' : undefined,
      ...element.days.map((d) => `${d.label}: ${Math.round(d.percent)}% of the 7d quota`),
      'A day with no reading (Session Deck wasn’t running) is folded into the next recorded day.',
    ];
    item.tooltip = tooltipLines.filter((line): line is string => Boolean(line)).join('\n');
    return item;
  }

  /**
   * One agent's projects, placed by the shared tree prefs: folders first (those holding at least one of
   * this workspace's projects), then top-level projects in their manual order, the rest by name.
   */
  private async getTopLevel(agent: AgentType): Promise<(FolderNode | ProjectGroupNode)[]> {
    const projects = await this.getProjectGroups(agent);
    const byKey = new Map(projects.map((p) => [projectKeyOf(p), p]));
    const arranged = arrangeProjects(this.state.getTree(), [...byKey.keys()]);
    const folders = arranged.folders
      .filter((f) => f.projects.length > 0)
      .map((f) => new FolderNode(agent, f.folder, f.projects.map((k) => byKey.get(k)!)));
    return [...folders, ...arranged.root.map((k) => byKey.get(k)!)];
  }

  /** Project keys in the order shown inside `folderId` (or at the top level for `null`), for Move Up/Down. */
  async displayedProjectOrder(agent: AgentType, folderId: string | null): Promise<string[]> {
    const top = await this.getTopLevel(agent);
    const nodes = folderId === null ? top.filter((n) => n.kind === 'project') : (top.find((n) => n.kind === 'folder' && n.folder.id === folderId) as FolderNode | undefined)?.projects ?? [];
    return nodes.map((n) => projectKeyOf(n as ProjectGroupNode));
  }

  /** Folders and projects start expanded unless collapsed here or in the terminal UI. */
  private collapsibleState(nodeKey: string): vscode.TreeItemCollapsibleState {
    return this.state.getTree().collapsed.includes(nodeKey)
      ? vscode.TreeItemCollapsibleState.Collapsed
      : vscode.TreeItemCollapsibleState.Expanded;
  }

  private detectAgentsInUse(): AgentType[] {
    const result: AgentType[] = [];
    if (listProjectDirNames().length > 0) {
      result.push('claude');
    }
    if (listCopilotSessions().length > 0) {
      result.push('copilot');
    }
    return result;
  }

  /** Every Claude project found under `~/.claude/projects`, whether or not it's on this workspace's list. */
  private async discoverGroups(): Promise<Map<string, DiscoveredGroup>> {
    const dirNames = listProjectDirNames();
    const members = await Promise.all(
      dirNames.map(async (dirName) => ({ dirName, cwd: await resolveDirCwd(dirName) }))
    );
    const roots = await mapWithConcurrency(members, GIT_RESOLVE_CONCURRENCY, (m) => resolveProjectRoot(m.cwd));

    const groups = new Map<string, DiscoveredGroup>();
    members.forEach((member, i) => {
      const canonicalRoot = roots[i].root;
      const key = normalizeFsPath(canonicalRoot);
      const group = groups.get(key) ?? { canonicalRoot, members: [] };
      group.members.push(member);
      groups.set(key, group);
    });
    return groups;
  }

  /**
   * Every discovered project, for the "Add Project" picker. `SESSION_DECK_DISCOVERY_ROOT`, if set,
   * narrows this to one root — a dev-only escape hatch for demoing the picker without every real
   * project on the machine showing up. Doesn't affect `getProjectGroups()` / what the tree renders.
   */
  async listKnownProjects(): Promise<{ rootPath: string; displayName: string }[]> {
    const groups = await this.discoverGroups();
    const discoveryRoot = process.env.SESSION_DECK_DISCOVERY_ROOT;
    return [...groups.values()]
      .filter((g) => !discoveryRoot || isInside(discoveryRoot, g.canonicalRoot))
      .map((g) => ({ rootPath: g.canonicalRoot, displayName: path.basename(g.canonicalRoot) || g.canonicalRoot }))
      .sort((a, b) => a.displayName.localeCompare(b.displayName));
  }

  /** `.vscode/session-deck.json` is the sole source of truth for the tree — no file means no projects shown, never "show everything discovered". A `hidden: true` entry is skipped here. */
  private async getProjectGroups(agent: AgentType): Promise<ProjectGroupNode[]> {
    const workspaceEntries = (readWorkspaceProjectEntries() ?? []).filter((e) => !e.hidden);
    const nodes: ProjectGroupNode[] = [];

    if (agent === 'claude') {
      const groups = await this.discoverGroups();
      for (const entry of workspaceEntries) {
        const groupMembers = groups.get(normalizeFsPath(entry.root))?.members ?? [];
        const displayName = entry.name ?? (path.basename(entry.root) || entry.root);
        const totalSessionCount = groupMembers.reduce((sum, m) => sum + listSessionFiles(m.dirName).length, 0);
        nodes.push(
          new ProjectGroupNode(
            'claude',
            entry.root,
            displayName,
            groupMembers,
            totalSessionCount,
            entry.maxSessionsShown ?? MAX_SESSIONS_PER_PROJECT_VIEW,
            entry.emoji,
            entry.color,
            entry.stopIdleSessions ?? false,
            entry.stopIdleMinutes ?? DEFAULT_STOP_IDLE_MINUTES
          )
        );
      }
    } else {
      for (const entry of workspaceEntries) {
        const rows = await this.copilotSessionsForRoot(entry.root);
        const members = [...new Set(rows.map((r) => r.cwd))].map((cwd) => ({ dirName: cwd, cwd }));
        const displayName = entry.name ?? (path.basename(entry.root) || entry.root);
        nodes.push(
          new ProjectGroupNode(
            'copilot',
            entry.root,
            displayName,
            members,
            rows.length,
            entry.maxSessionsShown ?? MAX_SESSIONS_PER_PROJECT_VIEW,
            entry.emoji,
            entry.color,
            entry.stopIdleSessions ?? false,
            entry.stopIdleMinutes ?? DEFAULT_STOP_IDLE_MINUTES
          )
        );
      }
    }

    return nodes.sort((a, b) => a.displayName.localeCompare(b.displayName));
  }

  /** Every session for a group's agent, newest first, archived and active alike — also where `enforceArchiveCap` runs from. */
  private async gatherCandidates(group: ProjectGroupNode): Promise<SessionCandidate[]> {
    const candidates =
      group.agent === 'claude' ? this.gatherClaudeCandidates(group) : await this.gatherCopilotCandidates(group);
    candidates.sort((a, b) => b.mtime.getTime() - a.mtime.getTime());
    await this.enforceArchiveCap(group, candidates);
    this.enforceIdleStop(group, candidates);
    return candidates;
  }

  /** Cheap: sorts on mtime only, never parses a transcript for its title (that's lazy, in `toSessionNodes`). */
  private gatherClaudeCandidates(group: ProjectGroupNode): SessionCandidate[] {
    const candidates: SessionCandidate[] = [];
    for (const { dirName, cwd } of group.members) {
      for (const file of listSessionFiles(dirName)) {
        const filePath = path.join(getClaudeProjectsDir(), dirName, file);
        candidates.push({
          agent: 'claude',
          sessionId: path.basename(file, '.jsonl'),
          cwd,
          mtime: fs.statSync(filePath).mtime,
          dirName,
          filePath,
        });
      }
    }
    return candidates;
  }

  private async gatherCopilotCandidates(group: ProjectGroupNode): Promise<SessionCandidate[]> {
    const rows = await this.copilotSessionsForRoot(group.rootPath);
    return rows.map((row) => ({
      agent: 'copilot' as const,
      sessionId: row.id,
      cwd: row.cwd,
      mtime: new Date(row.updatedAt),
      summary: row.summary ?? undefined,
    }));
  }

  /** Every Copilot session whose `cwd` resolves (git-aware) to `rootPath` — re-queries and re-resolves each call, no caching. */
  private async copilotSessionsForRoot(rootPath: string): Promise<CopilotSessionRow[]> {
    const targetKey = normalizeFsPath(rootPath);
    const sessions = listCopilotSessions();
    const roots = await mapWithConcurrency(sessions, GIT_RESOLVE_CONCURRENCY, (row) => resolveProjectRoot(row.cwd));
    return sessions.filter((_, i) => normalizeFsPath(roots[i].root) === targetKey);
  }

  private async toSessionNodes(candidates: SessionCandidate[], group: ProjectGroupNode): Promise<SessionNode[]> {
    return Promise.all(
      candidates.map(async (c) => {
        if (c.agent === 'claude') {
          const meta = await readSessionMeta(c.filePath as string);
          const displayName = this.state.getSessionName(c.sessionId) ?? meta.title ?? meta.firstPrompt ?? '(empty session)';
          return new SessionNode('claude', c.sessionId, displayName, c.mtime, c.cwd, group.rootPath, c.dirName, c.filePath);
        }
        const displayName = this.state.getSessionName(c.sessionId) ?? c.summary ?? '(empty session)';
        return new SessionNode('copilot', c.sessionId, displayName, c.mtime, c.cwd, group.rootPath);
      })
    );
  }

  /**
   * Keeps at most `maxSessionsShown` active sessions per (agent, project), auto-archiving the oldest
   * excess — but only in response to a genuinely new session appearing, never on the first pass a
   * project is seen (which would otherwise mass-archive every pre-existing project's history).
   */
  private async enforceArchiveCap(group: ProjectGroupNode, candidates: SessionCandidate[]): Promise<void> {
    const key = `${group.agent}:${normalizeFsPath(group.rootPath)}`;
    const currentIds = new Set(candidates.map((c) => c.sessionId));
    const known = this.knownSessionIdsByProject.get(key);
    this.knownSessionIdsByProject.set(key, currentIds);

    if (!known) {
      return;
    }

    const newlyDiscovered = new Set([...currentIds].filter((id) => !known.has(id)));
    // Pinned sessions are never auto-archived, and don't count toward the cap.
    const activeIdsByRecency = candidates
      .filter((c) => !this.state.isSessionArchived(c.sessionId) && !this.state.getSessionPin(c.sessionId))
      .map((c) => c.sessionId);
    const toArchive = selectSessionsToArchive(activeIdsByRecency, group.maxSessionsShown, newlyDiscovered);
    if (toArchive.length === 0) {
      return;
    }

    for (const sessionId of toArchive) {
      await this.state.setSessionArchived(sessionId, true);
    }
    this.onSessionsArchived?.(toArchive);
  }

  /**
   * Opt-in per project (`stopIdleSessions`): closes the terminal of any of this group's sessions
   * that have sat idle for at least `stopIdleMinutes`, to free the PTY/memory — but never archives
   * them, and runs regardless of archive/pin state, since an idle terminal still costs resources
   * whether or not it's shown in the tree.
   */
  private enforceIdleStop(group: ProjectGroupNode, candidates: SessionCandidate[]): void {
    if (!group.stopIdleSessions) {
      return;
    }
    const toStop = selectSessionsToStop(
      candidates.map((c) => ({ sessionId: c.sessionId, lastActivity: c.mtime })),
      group.stopIdleMinutes
    );
    if (toStop.length === 0) {
      return;
    }
    this.onSessionsIdleStopped?.(toStop);
  }

  /** Every session across every project and both agents, archived included — used by Search, uncapped unlike the tree. */
  async listAllSessions(): Promise<SessionWithProject[]> {
    const result: SessionWithProject[] = [];
    for (const agent of AGENTS) {
      const groups = await this.getProjectGroups(agent);
      for (const group of groups) {
        const candidates = await this.gatherCandidates(group);
        const sessions = await this.toSessionNodes(candidates, group);
        for (const session of sessions) {
          result.push({ session, projectDisplayName: group.displayName });
        }
      }
    }
    return result;
  }

}

interface SessionCandidate {
  agent: AgentType;
  sessionId: string;
  cwd: string;
  mtime: Date;
  /** Claude-only. */
  dirName?: string;
  /** Claude-only. */
  filePath?: string;
  /** Copilot-only: its `sessions.summary` column, already fetched — no extra read needed to title the row (unlike Claude, which parses the transcript's first prompt lazily). */
  summary?: string;
}

function agentLabel(agent: AgentType): string {
  return agent === 'claude' ? 'Claude' : 'Copilot';
}

/** Thresholds and bar rendering live in `@session-deck/core` (`usageDisplay.ts`) so the terminal UI matches exactly. */
const USAGE_SEVERITY_COLOR: Record<UsageSeverity, string> = { ok: 'charts.green', warning: 'charts.yellow', critical: 'charts.red' };

function usageColor(metric: UsageMetric, percent: number): vscode.ThemeColor {
  return new vscode.ThemeColor(USAGE_SEVERITY_COLOR[usageSeverity(metric, percent)]);
}

/** The cwd a ~/.claude/projects/<dirName> folder was created for, read from its first session. */
async function resolveDirCwd(dirName: string): Promise<string> {
  for (const file of listSessionFiles(dirName)) {
    const { cwd } = await readSessionMeta(path.join(getClaudeProjectsDir(), dirName, file));
    if (cwd) {
      return cwd;
    }
  }
  // Last resort: no session in this folder recorded a cwd (e.g. an empty/corrupt folder).
  return decodeProjectPath(dirName);
}

function relativeSubpath(root: string, cwd: string): string {
  if (normalizeFsPath(root) === normalizeFsPath(cwd)) {
    return '';
  }
  const rel = path.relative(root, cwd);
  return rel && !rel.startsWith('..') ? rel : '';
}

function statusTooltipLine(status: SessionStatusRecord | undefined): string {
  if (!status) {
    return '';
  }
  return `Status: ${status.status} (${timeAgo(new Date(status.updatedAt))})`;
}

function timeAgo(date: Date): string {
  const seconds = Math.floor((Date.now() - date.getTime()) / 1000);
  const units: [number, string][] = [
    [60, 's'],
    [60, 'm'],
    [24, 'h'],
    [7, 'd'],
    [4.345, 'w'],
  ];
  let value = seconds;
  let unit = 's';
  for (const [size, label] of units) {
    if (value < size) {
      break;
    }
    value = Math.floor(value / size);
    unit = label;
  }
  return `${value}${unit} ago`;
}
