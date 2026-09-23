import * as path from 'path';
import * as vscode from 'vscode';
import { mapWithConcurrency } from './concurrency';
import { fuzzyMatch } from './fuzzyMatch';
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
import { resolveProjectRoot, clearProjectRootCache } from './discovery/gitProject';
import { acknowledgeSessionStatus, readEffectiveSessionStatus, SessionStatus } from './status/sessionStatus';
import { SessionStatusDecorationProvider } from './status/sessionStatusDecorationProvider';
import { ClaudeProcessWatcher } from './status/claudeProcessWatcher';
import { WaitingNotifier } from './status/waitingNotifier';
import { DeckState } from './config/state';
import { AgentTerminalService } from './terminal/terminalService';
import {
  addProjectToWorkspaceList,
  getConfigFsPath,
  getConfigGlobPattern,
  getWorkspaceProjectEntry,
  readWorkspaceProjectEntries,
  readWorkspaceProjectEntriesForFolder,
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
  // An auto-archived session shouldn't be left running in a now-hidden tab.
  const treeProvider = new SessionTreeProvider(state, context.extensionUri, (sessionIds) => {
    for (const sessionId of sessionIds) {
      terminalService.closeSessionTerminal(sessionId);
    }
  });
  const activeSessionProvider = new ActiveSessionProvider(treeProvider, terminalService, context.extensionUri);
  // resources/icon.png (not the tree's claude-mark.svg): most OS notifiers expect a raster icon.
  const waitingNotifier = new WaitingNotifier(path.join(context.extensionPath, 'resources', 'icon.png'));
  const claudeProcessWatcher = new ClaudeProcessWatcher((message) => outputChannel.appendLine(message));

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
    vscode.commands.registerCommand('sessionDeck.forkSession', (session: SessionNode) => forkSession(session, terminalService)),
    vscode.commands.registerCommand('sessionDeck.forkSessionDangerously', (session: SessionNode) =>
      forkSessionDangerously(session, terminalService)
    ),
    { dispose: () => claudeProcessWatcher.dispose() }
  );

  claudeProcessWatcher.start();

  treeProvider.watch(context, () => {
    statusDecorationProvider.refresh();
    activeSessionProvider.refresh();
    waitingNotifier.check(terminalService.getOpenSessions(), (sessionId) => terminalService.revealSession(sessionId));
  });

  // Auto-refresh when any workspace folder's project list file changes (a plain glob string
  // watches every open folder, not just the first — see getConfigGlobPattern).
  const configPattern = getConfigGlobPattern();
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

/** Opens a session as a virtual read-only markdown document, in a reused VS Code preview tab. */
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
  // Always a fresh terminal — reusing an existing one would ignore the flag just confirmed.
  await terminalService.openSessionInNewTerminal(session, { dangerouslySkipPermissions: true });
  acknowledgeAndRefresh(session, tree, statusDecorationProvider, activeSessionProvider);
}

/** Opening a session counts as acknowledging its status, same as selecting its row does. */
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

/** Reads the setting live (no caching) so toggling it takes effect immediately; a project's own `dangerouslySkipPermissions: true` overrides it to `false` for that project only. */
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

/** A leading status filter in Search: `!running`, `@waiting`, `#done`, `~error`. */
const SEARCH_STATUS_PREFIXES: Record<string, SessionStatus> = {
  '!': 'running',
  '@': 'waiting',
  '#': 'done',
  '~': 'error',
};

/** Caps how many sessions' content Search reads concurrently on first use. */
const SEARCH_READ_CONCURRENCY = 8;

/** Full-text search across every session's prompts/replies, with an optional leading status filter. */
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
    alwaysShow: true,
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
      const pairs = await mapWithConcurrency(all, SEARCH_READ_CONCURRENCY, async (entry): Promise<[string, string]> => [
        entry.session.sessionId,
        entry.session.agent === 'claude'
          ? await readSessionSearchText(entry.session.filePath as string)
          : copilotSessionSearchText(entry.session.sessionId),
      ]);
      searchTextBySessionId = new Map(pairs);
      quickPick.busy = false;
    }

    quickPick.items = all
      .filter((entry) => !statusFilter || readEffectiveSessionStatus(entry.session.sessionId)?.status === statusFilter)
      .map((entry) => ({ entry, match: fuzzyMatch(query, searchTextBySessionId?.get(entry.session.sessionId) ?? '') }))
      .filter((x) => x.match.matched)
      .sort((a, b) => a.match.score - b.match.score)
      .map((x) => toItem(x.entry));
  });

  quickPick.onDidAccept(() => {
    const [selected] = quickPick.selectedItems;
    quickPick.hide();
    if (selected) {
      // Routed through the command itself so this gets the same open/acknowledge/refresh behavior.
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

interface WorkspaceFolderPickItem extends vscode.QuickPickItem {
  folder: vscode.WorkspaceFolder;
}

/** Which workspace folder's session-deck.json to write to — only prompts when more than one is open. */
async function pickTargetWorkspaceFolder(promptContext: string): Promise<vscode.WorkspaceFolder | undefined> {
  const folders = vscode.workspace.workspaceFolders;
  if (!folders || folders.length === 0) {
    return undefined;
  }
  if (folders.length === 1) {
    return folders[0];
  }
  const items: WorkspaceFolderPickItem[] = folders.map((folder) => ({
    label: folder.name,
    description: folder.uri.fsPath,
    folder,
  }));
  const picked = await vscode.window.showQuickPick(items, {
    placeHolder: `Which workspace folder's session-deck.json should ${promptContext}?`,
  });
  return picked?.folder;
}

/** Pick from discovered Claude/Copilot projects, or browse to a folder not listed yet. */
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
    placeHolder: 'Select a project to show in this workspace',
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

  const targetFolder = await pickTargetWorkspaceFolder('this project be added to');
  if (!targetFolder) {
    return;
  }

  await addProjectToWorkspaceList(rootPath, name, targetFolder);
  tree.refresh();
}

/** Opens a workspace folder's session-deck.json for direct hand-editing, creating it empty first if needed. */
async function editProjectList(): Promise<void> {
  if (!vscode.workspace.workspaceFolders?.length) {
    vscode.window.showErrorMessage('Open a folder or workspace first — the project list is saved per workspace.');
    return;
  }

  const targetFolder = await pickTargetWorkspaceFolder('be opened for editing');
  if (!targetFolder) {
    return;
  }

  if (!readWorkspaceProjectEntriesForFolder(targetFolder)) {
    writeWorkspaceProjectEntries([], targetFolder);
  }

  const configPath = getConfigFsPath(targetFolder);
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

/** Claude-only (`claude --resume --fork-session`, no Copilot equivalent) — branches a brand-new session off this one's full history, leaving the original untouched. */
async function forkSession(session: SessionNode, terminalService: AgentTerminalService): Promise<void> {
  if (!session) {
    return;
  }
  if (session.agent !== 'claude') {
    await vscode.window.showInformationMessage("Forking a session isn't supported by Copilot CLI yet.");
    return;
  }
  await terminalService.forkSession(session);
}

/** Same reasoning as `newSessionDangerously`: always a fresh terminal (forking always is), gated behind an explicit confirmation. */
async function forkSessionDangerously(session: SessionNode, terminalService: AgentTerminalService): Promise<void> {
  if (!session) {
    return;
  }
  if (session.agent !== 'claude') {
    await vscode.window.showInformationMessage("Forking a session isn't supported by Copilot CLI yet.");
    return;
  }
  if (shouldConfirmDangerousSkipPermissions(session.projectRoot)) {
    const confirm = await vscode.window.showWarningMessage(
      `Fork "${session.displayName}" with ${dangerousModeFlag(session.agent)}?`,
      {
        modal: true,
        detail: dangerousModeDetail(session.agent),
      },
      'Fork'
    );
    if (confirm !== 'Fork') {
      return;
    }
  }
  await terminalService.forkSession(session, { dangerouslySkipPermissions: true });
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

/** Soft-hide: sets `hidden: true` rather than removing the entry — no confirmation, non-destructive. */
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

export function deactivate() {}
