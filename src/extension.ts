import * as path from 'path';
import * as vscode from 'vscode';
import { AgentType, SessionTreeProvider, ProjectGroupNode, SessionNode, SessionWithProject } from './tree/sessionProvider';
import { ActiveSessionProvider } from './tree/activeSessionProvider';
import { SessionContentProvider, SESSION_SCHEME } from './content/sessionContentProvider';
import {
  clearSearchTextCache,
  clearSessionMetaCache,
  readLastAssistantResponse,
  readSessionSearchText,
} from './discovery/claudeStorage';
import { copilotSessionSearchText, lastCopilotAssistantResponse } from './discovery/copilotStorage';
import {
  disableStatusTracking,
  enableStatusTracking,
  getClaudeSettingsPath,
  isStatusTrackingEnabled,
  isStatusTrackingPresent,
} from './status/claudeSettings';
import { resolveProjectRoot, clearProjectRootCache } from './discovery/gitProject';
import { acknowledgeSessionStatus, readEffectiveSessionStatus, SessionStatus } from './status/sessionStatus';
import { SessionStatusDecorationProvider } from './status/sessionStatusDecorationProvider';
import { WaitingNotifier } from './status/waitingNotifier';
import { DeckState } from './config/state';
import { AgentTerminalService } from './terminal/terminalService';
import {
  addProjectToWorkspaceList,
  getConfigFsPath,
  getConfigRelativePattern,
  getWorkspaceProjectEntry,
  readWorkspaceProjectEntries,
  removeProjectFromWorkspaceList,
  setWorkspaceProjectHidden,
  setWorkspaceProjectName,
  writeWorkspaceProjectEntries,
} from './config/workspaceConfig';

export function activate(context: vscode.ExtensionContext) {
  const state = new DeckState(context.globalState);
  const contentProvider = new SessionContentProvider();
  const statusDecorationProvider = new SessionStatusDecorationProvider();
  const outputChannel = vscode.window.createOutputChannel('Session Deck');
  const terminalService = new AgentTerminalService(outputChannel, context.extensionUri);
  // The cap-eviction callback: a session auto-archived because a project went over
  // MAX_SESSIONS_PER_PROJECT_VIEW shouldn't be left running in a now-hidden tab.
  const treeProvider = new SessionTreeProvider(state, context.extensionUri, (sessionIds) => {
    for (const sessionId of sessionIds) {
      terminalService.closeSessionTerminal(sessionId);
    }
  });
  const activeSessionProvider = new ActiveSessionProvider(treeProvider, terminalService, context.extensionUri);
  // resources/icon.png (not the tree's claude-mark.svg): most OS notifiers expect a raster icon.
  const waitingNotifier = new WaitingNotifier(path.join(context.extensionPath, 'resources', 'icon.png'));

  const treeView = vscode.window.createTreeView('sessionDeck.sessions', {
    treeDataProvider: treeProvider,
    showCollapseAll: true,
  });

  context.subscriptions.push(
    treeView,
    outputChannel,
    terminalService,
    vscode.window.registerFileDecorationProvider(statusDecorationProvider),
    vscode.window.registerTreeDataProvider('sessionDeck.activeSession', activeSessionProvider),
    terminalService.onDidChangeOpenSessions(() => activeSessionProvider.refresh()),
    treeView.onDidChangeSelection((e) => {
      const selectedSessions = e.selection.filter((node): node is SessionNode => node.kind === 'session');
      if (selectedSessions.length === 0) {
        return;
      }
      for (const session of selectedSessions) {
        acknowledgeSessionStatus(session.sessionId);
      }
      treeProvider.refresh();
      statusDecorationProvider.refresh();
      activeSessionProvider.refresh();
    }),
    vscode.workspace.registerTextDocumentContentProvider(SESSION_SCHEME, contentProvider),
    vscode.commands.registerCommand('sessionDeck.refresh', () => {
      clearProjectRootCache();
      clearSessionMetaCache();
      clearSearchTextCache();
      terminalService.clearBinaryCaches();
      treeProvider.refresh();
      activeSessionProvider.refresh();
    }),
    vscode.commands.registerCommand('sessionDeck.openSession', async (session: SessionNode) => {
      await ensureSessionActive(session, state, treeProvider);
      await terminalService.openSession(session);
      acknowledgeAndRefresh(session, treeProvider, statusDecorationProvider, activeSessionProvider);
    }),
    vscode.commands.registerCommand('sessionDeck.openSessionDangerously', (session: SessionNode) =>
      openSessionDangerously(session, state, treeProvider, statusDecorationProvider, activeSessionProvider, terminalService)
    ),
    vscode.commands.registerCommand('sessionDeck.openSessionInNewTerminal', async (session: SessionNode) => {
      await ensureSessionActive(session, state, treeProvider);
      await terminalService.openSessionInNewTerminal(session);
      acknowledgeAndRefresh(session, treeProvider, statusDecorationProvider, activeSessionProvider);
    }),
    vscode.commands.registerCommand('sessionDeck.viewTranscript', (session: SessionNode) => viewTranscript(session)),
    vscode.commands.registerCommand('sessionDeck.search', () => searchSessions(treeProvider)),
    vscode.commands.registerCommand('sessionDeck.archiveSession', (node: SessionNode) =>
      archiveSession(node, state, treeProvider, terminalService)
    ),
    vscode.commands.registerCommand('sessionDeck.unarchiveSession', (node: SessionNode) =>
      unarchiveSession(node, state, treeProvider)
    ),
    vscode.commands.registerCommand('sessionDeck.copyLastResponse', (session: SessionNode) => copyLastResponse(session)),
    vscode.commands.registerCommand('sessionDeck.copySessionInfo', (session: SessionNode) => copySessionInfo(session)),
    vscode.commands.registerCommand('sessionDeck.addProject', () => addProject(treeProvider)),
    vscode.commands.registerCommand('sessionDeck.editProjectList', () => editProjectList()),
    vscode.commands.registerCommand('sessionDeck.newSession', (node: ProjectGroupNode) => newSession(node, terminalService)),
    vscode.commands.registerCommand('sessionDeck.newSessionDangerously', (node: ProjectGroupNode) =>
      newSessionDangerously(node, terminalService)
    ),
    vscode.commands.registerCommand('sessionDeck.renameProject', (node: ProjectGroupNode) =>
      renameProject(node, treeProvider)
    ),
    vscode.commands.registerCommand('sessionDeck.removeProject', (node: ProjectGroupNode) =>
      removeProject(node, treeProvider)
    ),
    vscode.commands.registerCommand('sessionDeck.hideProject', (node: ProjectGroupNode) => hideProject(node, treeProvider)),
    vscode.commands.registerCommand('sessionDeck.renameSession', (node: SessionNode) =>
      renameSession(node, state, treeProvider)
    ),
    vscode.commands.registerCommand('sessionDeck.enableStatusTracking', () =>
      enableStatusTrackingCommand(context, treeProvider)
    ),
    vscode.commands.registerCommand('sessionDeck.disableStatusTracking', () =>
      disableStatusTrackingCommand(treeProvider)
    )
  );

  treeProvider.watch(context, () => {
    statusDecorationProvider.refresh();
    activeSessionProvider.refresh();
    waitingNotifier.check(terminalService.getOpenSessions(), (sessionId) => terminalService.revealSession(sessionId));
  });

  // Auto-refresh when the workspace's project list file is created/edited/deleted
  // (by us via "Add Project"/"Remove Project", or by hand).
  const configPattern = getConfigRelativePattern();
  if (configPattern) {
    const refreshAll = () => {
      treeProvider.refresh();
      activeSessionProvider.refresh();
    };
    const configWatcher = vscode.workspace.createFileSystemWatcher(configPattern);
    configWatcher.onDidChange(refreshAll);
    configWatcher.onDidCreate(refreshAll);
    configWatcher.onDidDelete(refreshAll);
    context.subscriptions.push(configWatcher);
  }
}

/**
 * Opens a session as a virtual read-only markdown document, in preview mode.
 * VS Code reuses a single "preview" tab (the italicized one) across successive
 * opens as long as it hasn't been pinned/edited — clicking another session swaps
 * that tab's content instead of opening a new one, mirroring agent-deck's
 * single-pane session switching without any custom webview/tab-management code.
 *
 * A secondary action (👁 in the tree, or the context menu) — the primary click
 * now resumes the session in a real terminal instead (`sessionDeck.openSession`,
 * `terminalService.ts`), matching the reference extension's core behavior.
 */
async function viewTranscript(session: SessionNode): Promise<void> {
  if (!session) {
    return;
  }

  const uri =
    session.agent === 'claude'
      ? vscode.Uri.parse(`${SESSION_SCHEME}:/claude/${encodeURIComponent(session.projectDirName ?? '')}/${session.sessionId}.md`)
      : vscode.Uri.parse(`${SESSION_SCHEME}:/copilot/${session.sessionId}.md`);

  const doc = await vscode.workspace.openTextDocument(uri);
  await vscode.languages.setTextDocumentLanguage(doc, 'markdown');
  await vscode.window.showTextDocument(doc, {
    preview: true,
    preserveFocus: false,
    viewColumn: vscode.ViewColumn.Active,
  });
}

/** `claude --resume --dangerously-skip-permissions` / `copilot --resume --allow-all`, gated behind an explicit confirmation. */
async function openSessionDangerously(
  session: SessionNode,
  state: DeckState,
  tree: SessionTreeProvider,
  statusDecorationProvider: SessionStatusDecorationProvider,
  activeSessionProvider: ActiveSessionProvider,
  terminalService: AgentTerminalService
): Promise<void> {
  if (!session) {
    return;
  }
  if (shouldConfirmDangerousSkipPermissions(session.projectRoot)) {
    const confirm = await vscode.window.showWarningMessage(
      `Resume "${session.displayName}" with ${dangerousModeFlag(session.agent)}?`,
      {
        modal: true,
        detail: dangerousModeDetail(session.agent),
      },
      'Resume'
    );
    if (confirm !== 'Resume') {
      return;
    }
  }
  await ensureSessionActive(session, state, tree);
  // Always a fresh terminal (never reuses this session's existing terminal, if any) — silently
  // reusing a non-dangerous terminal here would ignore the flag the user just confirmed.
  await terminalService.openSessionInNewTerminal(session, { dangerouslySkipPermissions: true });
  acknowledgeAndRefresh(session, tree, statusDecorationProvider, activeSessionProvider);
}

/**
 * Opening/resuming a session is "looking at it" — a "done"/"error" dot only means "something
 * happened, you haven't looked yet" (see `sessionStatus.ts`), so this clears it the same way
 * selecting the row in the main tree already does (`treeView.onDidChangeSelection` below). Needed
 * as its own step because opening a session doesn't always go through that listener: the Explorer
 * "Open Sessions" view and Search's QuickPick both resume a session without ever selecting its row
 * in the main tree, so relying on tree selection alone left their session's status stuck showing
 * "done"/"error" after you'd already reopened it — reported live against a Copilot session that
 * stayed green after being reopened from the Explorer view.
 */
function acknowledgeAndRefresh(
  session: SessionNode,
  tree: SessionTreeProvider,
  statusDecorationProvider: SessionStatusDecorationProvider,
  activeSessionProvider: ActiveSessionProvider
): void {
  acknowledgeSessionStatus(session.sessionId);
  tree.refresh();
  statusDecorationProvider.refresh();
  activeSessionProvider.refresh();
}

/**
 * `sessionDeck.confirmDangerousSkipPermissions` (default `true`) — read live on every dangerous-mode
 * launch rather than cached, so toggling it in Settings takes effect immediately, no reload needed.
 * Ported from the reference extension's own setting of the same name/default. A project entry's own
 * `dangerouslySkipPermissions: true` (`.vscode/session-deck.json`) overrides this to `false` for that
 * project specifically — a per-project "I trust this one" exception, never the other way around: there's
 * no way to force confirmation back on for one project while the global setting is off.
 */
function shouldConfirmDangerousSkipPermissions(rootPath: string): boolean {
  if (getWorkspaceProjectEntry(rootPath)?.dangerouslySkipPermissions) {
    return false;
  }
  return vscode.workspace.getConfiguration('sessionDeck').get<boolean>('confirmDangerousSkipPermissions', true);
}

/** The actual CLI flag each agent's dangerous mode maps to — used only for confirmation-dialog wording. */
function dangerousModeFlag(agent: AgentType): string {
  return agent === 'claude' ? '--dangerously-skip-permissions' : '--allow-all';
}

function dangerousModeDetail(agent: AgentType): string {
  const label = agent === 'claude' ? 'Claude' : 'Copilot';
  return `${label} will not ask for approval before running tools in this session. Only do this if you trust what it will be doing.`;
}

/** Resuming an archived session takes it out of the archive — there's no such thing as an actively-open archived one. */
async function ensureSessionActive(session: SessionNode, state: DeckState, tree: SessionTreeProvider): Promise<void> {
  if (state.isSessionArchived(session.sessionId)) {
    await state.setSessionArchived(session.sessionId, false);
    tree.refresh();
  }
}

/** Manual "Archive" (session row inline icon) — also closes the session's terminal if it's currently open, same as an auto-archived one. */
async function archiveSession(
  node: SessionNode,
  state: DeckState,
  tree: SessionTreeProvider,
  terminalService: AgentTerminalService
): Promise<void> {
  if (!node) {
    return;
  }
  terminalService.closeSessionTerminal(node.sessionId);
  await state.setSessionArchived(node.sessionId, true);
  tree.refresh();
}

/** Manual "Unarchive" (archived session row inline icon). */
async function unarchiveSession(node: SessionNode, state: DeckState, tree: SessionTreeProvider): Promise<void> {
  if (!node) {
    return;
  }
  await state.setSessionArchived(node.sessionId, false);
  tree.refresh();
}

/** Copies just the last assistant reply — e.g. to paste a fix/answer somewhere else without opening the terminal or the transcript view. */
async function copyLastResponse(session: SessionNode): Promise<void> {
  if (!session) {
    return;
  }
  const text =
    session.agent === 'claude'
      ? await readLastAssistantResponse(session.filePath as string)
      : lastCopilotAssistantResponse(session.sessionId);
  if (!text) {
    vscode.window.showInformationMessage('No assistant response found in this session yet.');
    return;
  }
  await vscode.env.clipboard.writeText(text);
  vscode.window.showInformationMessage('Copied last response to the clipboard.');
}

/** Copies a plain-text summary — handy for pasting into an issue, a chat with a teammate, or another session. */
async function copySessionInfo(session: SessionNode): Promise<void> {
  if (!session) {
    return;
  }
  // Copilot sessions aren't status-tracked yet (see the "does NOT do yet" list), so this is
  // always blank for one today — reads live from the shared status files either way.
  const status = readEffectiveSessionStatus(session.sessionId);
  const lines = [
    session.displayName,
    `Agent: ${session.agent === 'claude' ? 'Claude Code' : 'GitHub Copilot'}`,
    `Project: ${path.basename(session.projectRoot)} (${session.projectRoot})`,
    `Working directory: ${session.cwd}`,
    `Session ID: ${session.sessionId}`,
    `Last modified: ${session.lastModified.toISOString()}`,
    status ? `Status: ${status.status}` : undefined,
    session.agent === 'claude' ? `Transcript: ${session.filePath}` : 'Transcript: ~/.copilot/session-store.db',
  ].filter((line): line is string => Boolean(line));
  await vscode.env.clipboard.writeText(lines.join('\n'));
  vscode.window.showInformationMessage('Copied session info to the clipboard.');
}

interface SearchPickItem extends vscode.QuickPickItem {
  session: SessionNode;
}

/**
 * Full-text search across every configured project's session content (not just
 * first prompts) — user prompts and assistant replies, extracted the same way
 * the transcript view renders them. The per-session content list is built once,
 * lazily, on the first keystroke (each session already caches its own extracted
 * text by mtime in `claudeStorage.ts`, so repeat searches are effectively free);
 * everything after that is an in-memory substring filter, so typing stays fast.
 */
/** A leading status filter in Search, same characters as agent-deck: `!running`, `@waiting for input`, `#done`, `~error`. */
const SEARCH_STATUS_PREFIXES: Record<string, SessionStatus> = {
  '!': 'running',
  '@': 'waiting',
  '#': 'done',
  '~': 'error',
};

async function searchSessions(tree: SessionTreeProvider): Promise<void> {
  const all = await tree.listAllSessions();
  if (all.length === 0) {
    vscode.window.showInformationMessage('No sessions to search — add a project first.');
    return;
  }

  const quickPick = vscode.window.createQuickPick<SearchPickItem>();
  quickPick.placeholder = 'Search sessions… ! running, @ waiting, # done, ~ error';
  quickPick.matchOnDescription = false;
  quickPick.matchOnDetail = false;

  const toItem = (entry: SessionWithProject): SearchPickItem => ({
    label: entry.session.displayName,
    description: entry.projectDisplayName,
    session: entry.session,
  });

  let searchTextBySessionId: Map<string, string> | undefined;

  quickPick.onDidChangeValue(async (value) => {
    const trimmed = value.trim();
    if (!trimmed) {
      quickPick.items = [];
      return;
    }

    const statusFilter = SEARCH_STATUS_PREFIXES[trimmed[0]];
    const query = (statusFilter ? trimmed.slice(1) : trimmed).trim().toLowerCase();

    if (query && !searchTextBySessionId) {
      quickPick.busy = true;
      const pairs = await Promise.all(
        all.map(async (entry): Promise<[string, string]> => [
          entry.session.sessionId,
          entry.session.agent === 'claude'
            ? await readSessionSearchText(entry.session.filePath as string)
            : copilotSessionSearchText(entry.session.sessionId),
        ])
      );
      searchTextBySessionId = new Map(pairs);
      quickPick.busy = false;
    }

    quickPick.items = all
      .filter((entry) => {
        if (statusFilter && readEffectiveSessionStatus(entry.session.sessionId)?.status !== statusFilter) {
          return false;
        }
        if (!query) {
          return true;
        }
        return (searchTextBySessionId?.get(entry.session.sessionId) ?? '').toLowerCase().includes(query);
      })
      .map(toItem);
  });

  quickPick.onDidAccept(() => {
    const [selected] = quickPick.selectedItems;
    quickPick.hide();
    if (selected) {
      // Routed through the command itself (not `terminalService.openSession` directly) so this gets
      // exactly the same `ensureSessionActive`/acknowledge/refresh behavior as clicking the session
      // anywhere else, with no duplicated logic here.
      void vscode.commands.executeCommand('sessionDeck.openSession', selected.session);
    }
  });

  quickPick.onDidHide(() => quickPick.dispose());
  quickPick.show();
}

interface ProjectPickItem extends vscode.QuickPickItem {
  rootPath?: string;
  addNew?: boolean;
}

/**
 * Lets the user build up this workspace's `.vscode/session-deck.json` list:
 * pick from every project Session Deck has discovered under `~/.claude/projects`,
 * or browse to a folder that isn't in that list yet (e.g. a project not yet used
 * with Claude Code).
 */
async function addProject(tree: SessionTreeProvider): Promise<void> {
  if (!vscode.workspace.workspaceFolders?.length) {
    vscode.window.showErrorMessage('Open a folder or workspace first — the project list is saved per workspace.');
    return;
  }

  const currentList = (readWorkspaceProjectEntries() ?? []).map((e) => e.root);
  const known = await tree.listKnownProjects();
  const notYetAdded = known.filter((p) => !currentList.includes(p.rootPath));

  const items: ProjectPickItem[] = [
    { label: '$(folder-opened) Add a project folder not listed here…', addNew: true },
    ...notYetAdded.map((p) => ({ label: p.displayName, description: p.rootPath, rootPath: p.rootPath })),
  ];

  const picked = await vscode.window.showQuickPick(items, {
    placeHolder: 'Select a Claude Code project to show in this workspace',
  });
  if (!picked) {
    return;
  }

  let rootPath: string | undefined;
  let name: string;
  if (picked.addNew) {
    const folders = await vscode.window.showOpenDialog({
      canSelectFolders: true,
      canSelectFiles: false,
      canSelectMany: false,
      openLabel: 'Add Project',
    });
    const folder = folders?.[0];
    if (!folder) {
      return;
    }
    rootPath = (await resolveProjectRoot(folder.fsPath)).root;
    name = path.basename(rootPath) || rootPath;
  } else {
    rootPath = picked.rootPath;
    name = picked.label;
  }

  if (!rootPath) {
    return;
  }
  await addProjectToWorkspaceList(rootPath, name);
  tree.refresh();
}

/**
 * Opens `.vscode/session-deck.json` for direct hand-editing — of names as well as
 * the project list itself, since it's the single source of truth for both.
 * Creates the file first, empty (`{ "projects": [] }`), if this workspace
 * doesn't have one yet — populating it is what "Add Project" and hand-editing
 * are for, not this button.
 */
async function editProjectList(): Promise<void> {
  if (!vscode.workspace.workspaceFolders?.length) {
    vscode.window.showErrorMessage('Open a folder or workspace first — the project list is saved per workspace.');
    return;
  }

  if (!readWorkspaceProjectEntries()) {
    writeWorkspaceProjectEntries([]);
  }

  const configPath = getConfigFsPath();
  if (!configPath) {
    return;
  }
  await vscode.window.showTextDocument(vscode.Uri.file(configPath));
}

/** Starts a brand-new session (Claude or Copilot, per the node's own agent) rooted at the project's own root path — not any particular worktree/subfolder member. */
async function newSession(node: ProjectGroupNode, terminalService: AgentTerminalService): Promise<void> {
  if (!node) {
    return;
  }
  await terminalService.startNewSession(node.agent, node.rootPath, node.displayName);
}

/** Same reasoning as `openSessionDangerously`: always a fresh terminal, gated behind an explicit confirmation. */
async function newSessionDangerously(node: ProjectGroupNode, terminalService: AgentTerminalService): Promise<void> {
  if (!node) {
    return;
  }
  if (shouldConfirmDangerousSkipPermissions(node.rootPath)) {
    const confirm = await vscode.window.showWarningMessage(
      `Start a new session in "${node.displayName}" with ${dangerousModeFlag(node.agent)}?`,
      {
        modal: true,
        detail: dangerousModeDetail(node.agent),
      },
      'Start'
    );
    if (confirm !== 'Start') {
      return;
    }
  }
  await terminalService.startNewSession(node.agent, node.rootPath, node.displayName, { dangerouslySkipPermissions: true });
}

/** A project only ever appears in the tree because it's on this workspace's list, so renaming always writes its `name` field there. */
async function renameProject(node: ProjectGroupNode, tree: SessionTreeProvider): Promise<void> {
  if (!node) {
    return;
  }
  const name = await vscode.window.showInputBox({
    prompt: `Rename "${node.displayName}" (${node.rootPath})`,
    value: node.displayName,
    ignoreFocusOut: true,
  });
  if (!name || name === node.displayName) {
    return;
  }
  if (!setWorkspaceProjectName(node.rootPath, name)) {
    vscode.window.showErrorMessage(
      `Couldn't rename "${node.displayName}" — its entry in session-deck.json seems to have changed. Try refreshing.`
    );
    return;
  }
  tree.refresh();
}

/** Same reasoning as rename: a visible project is always on the list, so "Remove" always just takes it off. */
async function removeProject(node: ProjectGroupNode, tree: SessionTreeProvider): Promise<void> {
  if (!node) {
    return;
  }
  const confirm = await vscode.window.showWarningMessage(
    `Remove "${node.displayName}" from this workspace's project list?`,
    {
      modal: true,
      detail:
        'It stays available to add back via "Session Deck: Add Project", and your Claude Code session history on disk is untouched.',
    },
    'Remove'
  );
  if (confirm !== 'Remove') {
    return;
  }
  await removeProjectFromWorkspaceList(node.rootPath);
  tree.refresh();
}

/**
 * Soft-hide: sets `hidden: true` on the entry rather than removing it (see `WorkspaceProjectEntry.hidden`).
 * No confirmation dialog — it's non-destructive and one hand-edit away from undone — but a project that
 * just vanished from the tree with no visible "Unhide" button anywhere is worth a pointer to how.
 */
async function hideProject(node: ProjectGroupNode, tree: SessionTreeProvider): Promise<void> {
  if (!node) {
    return;
  }
  if (!setWorkspaceProjectHidden(node.rootPath, true)) {
    vscode.window.showErrorMessage(
      `Couldn't hide "${node.displayName}" — its entry in session-deck.json seems to have changed. Try refreshing.`
    );
    return;
  }
  tree.refresh();
  vscode.window.showInformationMessage(
    `Hid "${node.displayName}". Set "hidden": false (or remove the property) for it in session-deck.json to bring it back — "Edit Project List" opens the file directly.`
  );
}

async function renameSession(node: SessionNode, state: DeckState, tree: SessionTreeProvider): Promise<void> {
  if (!node) {
    return;
  }
  const name = await vscode.window.showInputBox({
    prompt: 'Rename session',
    value: node.displayName,
    ignoreFocusOut: true,
  });
  if (!name || name === node.displayName) {
    return;
  }
  await state.setSessionName(node.sessionId, name);
  tree.refresh();
}

function statusHookCommand(context: vscode.ExtensionContext): string {
  const scriptPath = path.join(context.extensionPath, 'out', 'status', 'reportStatus.js');
  return `node "${scriptPath}"`;
}

/**
 * Adds the hook entries `reportStatus.ts` needs to Claude Code's *global*
 * `~/.claude/settings.json` — shown in full before writing anything, since this
 * affects every Claude Code session on the machine, not just this workspace.
 */
async function enableStatusTrackingCommand(context: vscode.ExtensionContext, tree: SessionTreeProvider): Promise<void> {
  const command = statusHookCommand(context);

  let alreadyEnabled: boolean;
  try {
    alreadyEnabled = isStatusTrackingEnabled(command);
  } catch (error) {
    vscode.window.showErrorMessage(errorMessage(error));
    return;
  }
  if (alreadyEnabled) {
    vscode.window.showInformationMessage('Session Deck status tracking is already enabled.');
    return;
  }

  const confirm = await vscode.window.showWarningMessage(
    'Enable live session status tracking?',
    {
      modal: true,
      detail: [
        `Adds 4 hook entries to your global ${getClaudeSettingsPath()}`,
        '(UserPromptSubmit, Stop, Notification, SessionEnd), each running:',
        '',
        command,
        '',
        'This affects every Claude Code session on this machine, not just this workspace.',
        'It only writes small status files under ~/.claude/session-deck-status/ — it never',
        'blocks or changes your prompts or tool calls. Undo any time with',
        '"Session Deck: Disable Live Status Tracking".',
      ].join('\n'),
    },
    'Enable'
  );
  if (confirm !== 'Enable') {
    return;
  }

  try {
    enableStatusTracking(command);
  } catch (error) {
    vscode.window.showErrorMessage(errorMessage(error));
    return;
  }
  tree.refresh();
  vscode.window.showInformationMessage(
    'Live session status tracking enabled — status dots appear once a hook actually fires (e.g. on your next prompt).'
  );
}

async function disableStatusTrackingCommand(tree: SessionTreeProvider): Promise<void> {
  let present: boolean;
  try {
    present = isStatusTrackingPresent();
  } catch (error) {
    vscode.window.showErrorMessage(errorMessage(error));
    return;
  }
  if (!present) {
    vscode.window.showInformationMessage('Session Deck status tracking is not currently enabled.');
    return;
  }

  try {
    disableStatusTracking();
  } catch (error) {
    vscode.window.showErrorMessage(errorMessage(error));
    return;
  }
  tree.refresh();
  vscode.window.showInformationMessage('Live session status tracking disabled.');
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function deactivate() {}
