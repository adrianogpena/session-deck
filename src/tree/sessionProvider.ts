import * as vscode from 'vscode';
import * as fs from 'fs';
import * as path from 'path';
import {
  decodeProjectPath,
  getClaudeProjectsDir,
  listProjectDirNames,
  listSessionFiles,
  readSessionMeta,
} from '../discovery/claudeStorage';
import { resolveProjectRoot } from '../discovery/gitProject';
import { normalizeFsPath } from '../discovery/pathUtils';
import { ensureSessionStatusDir, getSessionStatusDir, readEffectiveSessionStatus, SessionStatusRecord } from '../status/sessionStatus';
import { sessionStatusUri } from '../status/sessionStatusDecorationProvider';
import { DeckState } from '../config/state';
import { readWorkspaceProjectEntries } from '../config/workspaceConfig';

/** How many of a project's most recent sessions the main tree shows — see `getChildren`. */
const MAX_SESSIONS_PER_PROJECT_VIEW = 5;

interface ProjectMember {
  dirName: string;
  cwd: string;
}

interface DiscoveredGroup {
  /** The root exactly as git (or the raw cwd, for non-git folders) resolved it — nicely cased, used for display defaults. */
  canonicalRoot: string;
  members: ProjectMember[];
}

export class ProjectGroupNode {
  readonly kind = 'project' as const;
  constructor(
    /** Exactly as written in `.vscode/session-deck.json` — the stable identity used for renames/removal. */
    public readonly rootPath: string,
    public readonly displayName: string,
    public readonly members: ProjectMember[]
  ) {}
}

export class SessionNode {
  readonly kind = 'session' as const;
  constructor(
    public readonly projectDirName: string,
    public readonly sessionId: string,
    public readonly filePath: string,
    public readonly displayName: string,
    public readonly lastModified: Date,
    public readonly cwd: string,
    public readonly projectRoot: string
  ) {}
}

export type ClaudeDeckNode = ProjectGroupNode | SessionNode;

export interface SessionWithProject {
  session: SessionNode;
  projectDisplayName: string;
}

export class SessionTreeProvider implements vscode.TreeDataProvider<ClaudeDeckNode> {
  private readonly _onDidChangeTreeData = new vscode.EventEmitter<ClaudeDeckNode | undefined | void>();
  readonly onDidChangeTreeData = this._onDidChangeTreeData.event;

  constructor(private readonly state: DeckState, private readonly extensionUri: vscode.Uri) {}

  refresh(): void {
    this._onDidChangeTreeData.fire();
  }

  /** Best-effort live refresh. Recursive fs.watch isn't supported on every
   * platform (it is on Windows and macOS, which covers this project's target),
   * so this is a nice-to-have on top of the manual refresh button.
   *
   * `onStatusChanged` additionally repaints the status decorations
   * (`sessionStatusDecorationProvider.ts`) — a separate rendering layer from
   * the tree itself, so it needs its own explicit nudge. */
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

    // Repaints session status dots live as reportStatus.ts (a Claude Code hook,
    // enabled via "Session Deck: Enable Live Status Tracking") writes them.
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
    if (element.kind === 'project') {
      const item = new vscode.TreeItem(element.displayName, vscode.TreeItemCollapsibleState.Collapsed);
      item.contextValue = 'sessionDeckProject';
      item.iconPath = new vscode.ThemeIcon('briefcase');
      const tooltipLines = [
        element.rootPath,
        element.members.length > 1
          ? `(${element.members.length} linked working directories, incl. worktrees)`
          : '',
      ];
      const totalSessions = countSessionsInGroup(element);
      if (totalSessions > MAX_SESSIONS_PER_PROJECT_VIEW) {
        tooltipLines.push(`Showing the ${MAX_SESSIONS_PER_PROJECT_VIEW} most recent of ${totalSessions} sessions.`);
      }
      item.tooltip = tooltipLines.filter(Boolean).join('\n');
      return item;
    }

    const item = new vscode.TreeItem(element.displayName, vscode.TreeItemCollapsibleState.None);
    const relSubpath = relativeSubpath(element.projectRoot, element.cwd);
    item.description = relSubpath ? `${relSubpath} · ${timeAgo(element.lastModified)}` : timeAgo(element.lastModified);
    const status = readEffectiveSessionStatus(element.sessionId);
    item.tooltip = [`${element.cwd}`, element.sessionId, element.filePath, statusTooltipLine(status)]
      .filter(Boolean)
      .join('\n');
    item.iconPath = vscode.Uri.joinPath(this.extensionUri, 'resources', 'claude-mark.svg');
    // Status color lives on a FileDecoration (see sessionStatusDecorationProvider.ts), not
    // this icon: a ThemeIcon's ThemeColor gets washed out to the row's plain foreground when
    // the row is selected, but a decoration is a separate layer that keeps its color regardless.
    item.resourceUri = sessionStatusUri(element.sessionId);
    item.contextValue = 'sessionDeckSession';
    item.command = {
      command: 'sessionDeck.openSession',
      title: 'Open Claude Session',
      arguments: [element],
    };
    return item;
  }

  async getChildren(element?: ClaudeDeckNode): Promise<ClaudeDeckNode[]> {
    if (!element) {
      return this.getProjectGroups();
    }
    if (element.kind === 'project') {
      // Capped for readability — a project can easily accumulate dozens of
      // sessions over time. Search and the "Active Claude Session" Explorer
      // view both go through listAllSessions() instead, which is uncapped, so
      // an older session is still findable/still shows as active if it's the
      // one actually running.
      return this.getSessionsForGroup(element, MAX_SESSIONS_PER_PROJECT_VIEW);
    }
    return [];
  }

  /**
   * Every project Session Deck can find under `~/.claude/projects`, regardless of
   * whether it's on this workspace's `session-deck.json` list. Used to populate
   * the "Add Project" picker and to bootstrap the file the first time it's
   * created — never used directly to decide what the tree shows.
   *
   * Keyed case-insensitively on Windows (`normalizeFsPath`) so a project isn't
   * split into two groups just because git and a hand-typed workspace-list entry
   * happen to differ in drive-letter casing.
   */
  private async discoverGroups(): Promise<Map<string, DiscoveredGroup>> {
    const dirNames = listProjectDirNames();
    const members = await Promise.all(
      dirNames.map(async (dirName) => ({ dirName, cwd: await resolveDirCwd(dirName) }))
    );
    const roots = await Promise.all(members.map((m) => resolveProjectRoot(m.cwd)));

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

  /** Every discovered project with a default display name, for the "Add Project" picker and to seed a fresh `session-deck.json`. */
  async listKnownProjects(): Promise<{ rootPath: string; displayName: string }[]> {
    const groups = await this.discoverGroups();
    return [...groups.values()]
      .map((g) => ({ rootPath: g.canonicalRoot, displayName: path.basename(g.canonicalRoot) || g.canonicalRoot }))
      .sort((a, b) => a.displayName.localeCompare(b.displayName));
  }

  /**
   * `.vscode/session-deck.json` is the sole source of truth for what the tree
   * shows: no file, or `{ "projects": [] }`, means an empty tree — never "show
   * everything discovered". This keeps exactly one mental model instead of a
   * "configured" vs. "legacy default" mode to reason about.
   */
  private async getProjectGroups(): Promise<ProjectGroupNode[]> {
    const groups = await this.discoverGroups();
    const workspaceEntries = readWorkspaceProjectEntries() ?? [];

    const nodes: ProjectGroupNode[] = [];
    for (const entry of workspaceEntries) {
      const groupMembers = groups.get(normalizeFsPath(entry.root))?.members ?? [];
      const displayName = entry.name ?? (path.basename(entry.root) || entry.root);
      nodes.push(new ProjectGroupNode(entry.root, displayName, groupMembers));
    }

    return nodes.sort((a, b) => a.displayName.localeCompare(b.displayName));
  }

  /**
   * `limit`, when given, keeps only the N most recently modified sessions —
   * cheaply, by sorting on `fs.statSync` mtime *before* ever parsing a
   * transcript for its title, so a project with dozens of old sessions doesn't
   * pay to read all of them just to display the newest few.
   */
  private async getSessionsForGroup(group: ProjectGroupNode, limit?: number): Promise<SessionNode[]> {
    interface Candidate {
      dirName: string;
      cwd: string;
      filePath: string;
      sessionId: string;
      mtime: Date;
    }

    const candidates: Candidate[] = [];
    for (const { dirName, cwd } of group.members) {
      for (const file of listSessionFiles(dirName)) {
        const filePath = path.join(getClaudeProjectsDir(), dirName, file);
        candidates.push({
          dirName,
          cwd,
          filePath,
          sessionId: path.basename(file, '.jsonl'),
          mtime: fs.statSync(filePath).mtime,
        });
      }
    }
    candidates.sort((a, b) => b.mtime.getTime() - a.mtime.getTime());
    const selected = limit !== undefined ? candidates.slice(0, limit) : candidates;

    return Promise.all(
      selected.map(async (c) => {
        const meta = await readSessionMeta(c.filePath);
        const displayName = this.state.getSessionName(c.sessionId) ?? meta.firstPrompt ?? '(empty session)';
        return new SessionNode(c.dirName, c.sessionId, c.filePath, displayName, c.mtime, c.cwd, group.rootPath);
      })
    );
  }

  /** Every session across every configured project, for the "Search Sessions" command. */
  async listAllSessions(): Promise<SessionWithProject[]> {
    const groups = await this.getProjectGroups();
    const result: SessionWithProject[] = [];
    for (const group of groups) {
      const sessions = await this.getSessionsForGroup(group);
      for (const session of sessions) {
        result.push({ session, projectDisplayName: group.displayName });
      }
    }
    return result;
  }
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

function countSessionsInGroup(group: ProjectGroupNode): number {
  return group.members.reduce((sum, member) => sum + listSessionFiles(member.dirName).length, 0);
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
