import { execFile } from 'child_process';
import { randomUUID } from 'crypto';
import { promisify } from 'util';
import * as fs from 'fs';
import * as path from 'path';
import * as vscode from 'vscode';
import { getClaudeProjectsDir, listProjectDirNames, listSessionFiles } from '../discovery/claudeStorage';
import { AgentType, SessionNode } from '../tree/sessionProvider';
import { agentIconPath } from '../tree/agentIcons';
import { clearSessionStatus, markSessionError } from '../status/sessionStatus';
import { CopilotStatusWatcher } from '../status/copilotStatusWatcher';
import { buildClaudeForkCommand, buildClaudeNewSessionCommand, buildClaudeResumeCommand } from './claudeCommand';
import { buildCopilotNewSessionCommand, buildCopilotResumeCommand } from './copilotCommand';

const execFileAsync = promisify(execFile);

/** Time to wait for shell integration before falling back to sendText. */
const SHELL_INTEGRATION_TIMEOUT_MS = 500;

/** How long to wait for a brand-new session's transcript file to appear before giving up on correlating it to its terminal — generous since a slow-starting shell can delay it well past a plain launch. */
const NEW_SESSION_DISCOVERY_TIMEOUT_MS = 60_000;

/** How often to re-scan `~/.claude/projects` while waiting for a new session file — see `correlateNewSession`. */
const NEW_SESSION_POLL_INTERVAL_MS = 500;

export interface OpenSessionTerminalOptions {
  readonly dangerouslySkipPermissions?: boolean;
}

/**
 * Launches `claude --resume <sessionId>` in a real terminal — one terminal per session, tracked in
 * `sessionTerminals`, so switching sessions never touches another session's running terminal.
 */
export class AgentTerminalService implements vscode.Disposable {
  private readonly sessionTerminals = new Map<string, vscode.Terminal>();
  /** Display label per tracked session id — `session.displayName` for a resume, the project name for a just-started new session (its real title isn't known yet). Used only for `WaitingNotifier`'s notification text. */
  private readonly sessionLabels = new Map<string, string>();
  /** Which agent each tracked session belongs to — used to start/stop `copilotStatusWatcher` tailing on track/untrack. */
  private readonly sessionAgents = new Map<string, AgentType>();
  /** Session ids already claimed by a pending `correlateNewSession` call, so two "New Session" clicks can't both grab the same file. */
  private readonly claimedByCorrelation = new Set<string>();
  private readonly closeListener: vscode.Disposable;
  /** Memoized PATH check per CLI command — cleared on manual refresh. */
  private readonly binaryCheckCache = new Map<string, Promise<boolean>>();
  private readonly _onDidChangeOpenSessions = new vscode.EventEmitter<void>();
  /** Fires whenever a session gains or loses a tracked open terminal — what `activeSessionProvider.ts`'s Explorer view refreshes on. */
  public readonly onDidChangeOpenSessions = this._onDidChangeOpenSessions.event;
  /** Which session a just-launched `claude --resume` execution belongs to — see `executionEndListener` and `launch`. */
  private readonly sessionIdByExecution = new Map<vscode.TerminalShellExecution, string>();
  private readonly executionEndListener: vscode.Disposable;
  /** Tails a Copilot session's `events.jsonl` for live status, started/stopped alongside its terminal. No Claude equivalent needed here — Claude's status comes from its own separate watcher process. */
  private readonly copilotStatusWatcher: CopilotStatusWatcher;

  public constructor(private readonly outputChannel: vscode.OutputChannel, private readonly extensionUri: vscode.Uri) {
    this.copilotStatusWatcher = new CopilotStatusWatcher((message) => this.outputChannel.appendLine(message));

    this.closeListener = vscode.window.onDidCloseTerminal((terminal) => {
      for (const [sessionId, tracked] of this.sessionTerminals) {
        if (tracked === terminal) {
          this.untrackTerminal(sessionId);
          // No graceful shutdown on close, so the watcher may never notice — clear status ourselves
          // so the dot doesn't get stuck showing "running"/"waiting" for a session that's actually dead.
          clearSessionStatus(sessionId);
          this.outputChannel.appendLine(`[terminal] Terminal for session ${sessionId} closed; status cleared.`);
          break;
        }
      }
    });

    /**
     * Backstop error signal from the exit code of the `claude`/`copilot` command itself. Copilot has
     * its own more precise `session.error` event that usually resolves first; `exitCode` is
     * `undefined` for a Ctrl+C cancel or a sub-shell, so those are correctly left alone.
     */
    this.executionEndListener = vscode.window.onDidEndTerminalShellExecution((event) => {
      const sessionId = this.sessionIdByExecution.get(event.execution);
      if (!sessionId) {
        return;
      }
      this.sessionIdByExecution.delete(event.execution);
      if (typeof event.exitCode === 'number' && event.exitCode !== 0) {
        markSessionError(sessionId);
        this.outputChannel.appendLine(`[terminal] Session ${sessionId}'s command exited with code ${event.exitCode}; marked as error.`);
      }
    });
  }

  /** Closes every tracked terminal on extension-host shutdown — best-effort, since `dispose()` isn't reliably called on an abrupt window close (only a graceful reload/disable). */
  public dispose(): void {
    this.closeListener.dispose();
    this.executionEndListener.dispose();
    for (const [sessionId, terminal] of this.sessionTerminals) {
      clearSessionStatus(sessionId);
      this.copilotStatusWatcher.stop(sessionId);
      terminal.dispose();
    }
    this.sessionTerminals.clear();
    this.sessionAgents.clear();
  }

  /** Session ids that currently have a tracked, still-open terminal — what the Explorer "Open Sessions" view shows. */
  public openSessionIds(): ReadonlySet<string> {
    return new Set(this.sessionTerminals.keys());
  }

  /** Same, with a display label per id — what `WaitingNotifier` watches and names in its notifications. */
  public getOpenSessions(): { sessionId: string; label: string }[] {
    return [...this.sessionTerminals.keys()].map((sessionId) => ({
      sessionId,
      label: this.sessionLabels.get(sessionId) ?? sessionId,
    }));
  }

  /** Brings this session's terminal to the front within the window — the click-through target for `WaitingNotifier`. A no-op if it's not (or no longer) tracked. */
  public revealSession(sessionId: string): void {
    this.sessionTerminals.get(sessionId)?.show(true);
  }

  /** Closes this session's terminal if tracked — used when a session is archived so it doesn't keep running behind a hidden tab. */
  public closeSessionTerminal(sessionId: string): void {
    this.sessionTerminals.get(sessionId)?.dispose();
  }

  private trackTerminal(sessionId: string, terminal: vscode.Terminal, label: string, agent: AgentType): void {
    this.sessionTerminals.set(sessionId, terminal);
    this.sessionLabels.set(sessionId, label);
    this.sessionAgents.set(sessionId, agent);
    if (agent === 'copilot') {
      this.copilotStatusWatcher.start(sessionId);
    }
    this._onDidChangeOpenSessions.fire();
  }

  private untrackTerminal(sessionId: string): void {
    if (this.sessionAgents.get(sessionId) === 'copilot') {
      this.copilotStatusWatcher.stop(sessionId);
    }
    this.sessionTerminals.delete(sessionId);
    this.sessionLabels.delete(sessionId);
    this.sessionAgents.delete(sessionId);
    this._onDidChangeOpenSessions.fire();
  }

  /** The default action: reuses (just `.show()`s) this session's own terminal if it's still alive, otherwise opens a new one. */
  public async openSession(session: SessionNode, options: OpenSessionTerminalOptions = {}): Promise<void> {
    const existing = this.sessionTerminals.get(session.sessionId);
    if (existing) {
      this.outputChannel.appendLine(
        `[terminal] openSession(${session.sessionId}): tracked terminal found, exitStatus=${JSON.stringify(existing.exitStatus)}.`
      );
      if (existing.exitStatus === undefined) {
        existing.show();
        return;
      }
    } else {
      this.outputChannel.appendLine(`[terminal] openSession(${session.sessionId}): no tracked terminal, opening a new one.`);
    }
    await this.launch(session, options);
  }

  /** Always opens a fresh terminal for this session, even if one's tracked — used for "Skip Permissions" resume, which must never silently reuse a non-dangerous terminal. */
  public async openSessionInNewTerminal(session: SessionNode, options: OpenSessionTerminalOptions = {}): Promise<void> {
    await this.launch(session, options);
  }

  /** Starts a brand-new session for the given agent, rooted at `cwd` — always a fresh terminal. Dispatches to the two differently-shaped implementations below. */
  public async startNewSession(
    agent: AgentType,
    cwd: string,
    projectName: string,
    options: OpenSessionTerminalOptions = {}
  ): Promise<void> {
    if (agent === 'claude') {
      return this.startNewClaudeSession(cwd, projectName, options);
    }
    return this.startNewCopilotSession(cwd, projectName, options);
  }

  /** Plain `claude` (no `--resume`) — no CLI flag to pre-assign a session id, so its real id is learned after the fact via `correlateNewSession`. */
  private async startNewClaudeSession(cwd: string, projectName: string, options: OpenSessionTerminalOptions): Promise<void> {
    const dangerouslySkipPermissions = options.dangerouslySkipPermissions === true;

    const hasClaude = await this.hasBinaryOnPath('claude');
    if (!hasClaude) {
      this.reportBinaryNotFound('claude');
      return;
    }

    const terminal = vscode.window.createTerminal({
      name: truncate(`New: ${projectName}`, 35),
      cwd,
      location: { viewColumn: vscode.ViewColumn.Active },
      iconPath: agentIconPath(this.extensionUri, 'claude'),
      isTransient: true,
    });
    terminal.show(true);
    void this.correlateNewSession(terminal, projectName);

    const command = buildClaudeNewSessionCommand(dangerouslySkipPermissions);
    this.outputChannel.appendLine(
      `[terminal] Starting a new Claude session in ${cwd} (skipPermissions=${String(dangerouslySkipPermissions)}).`
    );

    await vscode.window.withProgress(
      { location: vscode.ProgressLocation.Notification, title: 'Waiting for terminal to be ready…' },
      () => executeInTerminal(terminal, command, this.terminalDeps())
    );
  }

  /** Unlike Claude, Copilot's `--session-id` flag lets a new session's UUID be chosen upfront, so the terminal is tracked under its real id from creation — no polling needed. */
  private async startNewCopilotSession(cwd: string, projectName: string, options: OpenSessionTerminalOptions): Promise<void> {
    const dangerouslySkipPermissions = options.dangerouslySkipPermissions === true;

    const hasCopilot = await this.hasBinaryOnPath('copilot');
    if (!hasCopilot) {
      this.reportBinaryNotFound('copilot');
      return;
    }

    const sessionId = randomUUID();
    const terminal = vscode.window.createTerminal({
      name: truncate(projectName, 35),
      cwd,
      location: { viewColumn: vscode.ViewColumn.Active },
      iconPath: agentIconPath(this.extensionUri, 'copilot'),
      isTransient: true,
    });
    this.trackTerminal(sessionId, terminal, projectName, 'copilot');
    terminal.show(true);

    const command = buildCopilotNewSessionCommand(sessionId, dangerouslySkipPermissions);
    this.outputChannel.appendLine(
      `[terminal] Starting a new Copilot session ${sessionId} in ${cwd} (skipPermissions=${String(dangerouslySkipPermissions)}).`
    );

    await vscode.window.withProgress(
      { location: vscode.ProgressLocation.Notification, title: 'Waiting for terminal to be ready…' },
      () =>
        executeInTerminal(
          terminal,
          command,
          this.terminalDeps((execution) => this.sessionIdByExecution.set(execution, sessionId))
        )
    );
  }

  /** Claude-only — resumes `session` under a brand-new session id via `--fork-session`, leaving the original untouched. Always a fresh terminal; the new id is learned via `correlateNewSession`. */
  public async forkSession(session: SessionNode, options: OpenSessionTerminalOptions = {}): Promise<void> {
    const dangerouslySkipPermissions = options.dangerouslySkipPermissions === true;

    const hasClaude = await this.hasBinaryOnPath('claude');
    if (!hasClaude) {
      this.reportBinaryNotFound('claude');
      return;
    }

    const terminal = vscode.window.createTerminal({
      name: truncate(`Fork: ${session.displayName}`, 35),
      cwd: session.cwd,
      location: { viewColumn: vscode.ViewColumn.Active },
      iconPath: agentIconPath(this.extensionUri, 'claude'),
      isTransient: true,
    });
    terminal.show(true);
    void this.correlateNewSession(terminal, session.displayName);

    const command = buildClaudeForkCommand(session.sessionId, dangerouslySkipPermissions);
    this.outputChannel.appendLine(
      `[terminal] Forking session ${session.sessionId} in ${session.cwd} (skipPermissions=${String(dangerouslySkipPermissions)}).`
    );

    await vscode.window.withProgress(
      { location: vscode.ProgressLocation.Notification, title: 'Waiting for terminal to be ready…' },
      () => executeInTerminal(terminal, command, this.terminalDeps())
    );
  }

  /**
   * Polls `~/.claude/projects` for the next brand-new session file, then tracks `terminal` under it.
   * Doesn't wait for a `cwd` match — `cwd` is only written on a session's first real turn, not at
   * startup, so matching on it would miss a session opened before anything's been typed. Polls
   * instead of `fs.watch`, which proved unreliable here on Windows for a large/busy directory tree.
   * Fire-and-forget: gives up silently after {@link NEW_SESSION_DISCOVERY_TIMEOUT_MS}.
   */
  private correlateNewSession(terminal: vscode.Terminal, projectName: string): Promise<void> {
    const root = getClaudeProjectsDir();
    if (!fs.existsSync(root)) {
      this.outputChannel.appendLine(`[terminal] correlateNewSession: ${root} doesn't exist, cannot poll for the new session.`);
      return Promise.resolve();
    }

    const before = snapshotSessionFilePaths(root);

    return new Promise<void>((resolve) => {
      let settled = false;
      const finish = (correlated: boolean) => {
        if (settled) {
          return;
        }
        settled = true;
        clearInterval(interval);
        clearTimeout(timeout);
        if (!correlated) {
          this.outputChannel.appendLine(
            `[terminal] correlateNewSession: gave up after ${NEW_SESSION_DISCOVERY_TIMEOUT_MS}ms without seeing a new session file.`
          );
        }
        resolve();
      };

      const poll = () => {
        if (settled) {
          return;
        }
        const current = snapshotSessionFilePaths(root);
        for (const filePath of current) {
          if (before.has(filePath)) {
            continue;
          }
          const sessionId = path.basename(filePath, '.jsonl');
          if (this.claimedByCorrelation.has(sessionId)) {
            continue;
          }
          this.claimedByCorrelation.add(sessionId);
          this.trackTerminal(sessionId, terminal, projectName, 'claude');
          this.outputChannel.appendLine(`[terminal] New session ${sessionId} correlated with its terminal.`);
          this.renameIfStillActive(terminal, projectName);
          finish(true);
          return;
        }
      };

      const interval = setInterval(poll, NEW_SESSION_POLL_INTERVAL_MS);
      const timeout = setTimeout(() => finish(false), NEW_SESSION_DISCOVERY_TIMEOUT_MS);
    });
  }

  /**
   * Drops the "New: " prefix once a session is correlated. No API renames a `vscode.Terminal`
   * directly — the only way, `workbench.action.terminal.renameWithArg`, also switches the visible
   * terminal tab, so this only renames if the terminal is still the active one.
   */
  private renameIfStillActive(terminal: vscode.Terminal, projectName: string): void {
    if (vscode.window.activeTerminal !== terminal) {
      return;
    }
    vscode.commands.executeCommand('workbench.action.terminal.renameWithArg', { name: truncate(projectName, 35) }).then(
      undefined,
      (error) => this.outputChannel.appendLine(`[terminal] renameIfStillActive: rename command failed: ${String(error)}`)
    );
  }

  private async launch(session: SessionNode, options: OpenSessionTerminalOptions): Promise<void> {
    const dangerouslySkipPermissions = options.dangerouslySkipPermissions === true;
    const command = session.agent === 'claude' ? 'claude' : 'copilot';

    const hasBinary = await this.hasBinaryOnPath(command);
    if (!hasBinary) {
      this.reportBinaryNotFound(command);
      return;
    }

    let resumeCommand: string;
    try {
      resumeCommand =
        session.agent === 'claude'
          ? buildClaudeResumeCommand(session.sessionId, dangerouslySkipPermissions)
          : buildCopilotResumeCommand(session.sessionId, dangerouslySkipPermissions);
    } catch (error) {
      vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
      return;
    }

    const terminal = vscode.window.createTerminal({
      name: truncate(session.displayName, 35),
      cwd: session.cwd,
      location: { viewColumn: vscode.ViewColumn.Active },
      iconPath: agentIconPath(this.extensionUri, session.agent),
      isTransient: true,
    });
    this.trackTerminal(session.sessionId, terminal, session.displayName, session.agent);
    terminal.show(true);

    this.outputChannel.appendLine(
      `[terminal] Opening session ${session.sessionId} (skipPermissions=${String(dangerouslySkipPermissions)}).`
    );

    await vscode.window.withProgress(
      { location: vscode.ProgressLocation.Notification, title: 'Waiting for terminal to be ready…' },
      () =>
        executeInTerminal(
          terminal,
          resumeCommand,
          this.terminalDeps((execution) => this.sessionIdByExecution.set(execution, session.sessionId))
        )
    );
  }

  private terminalDeps(onExecutionStarted?: (execution: vscode.TerminalShellExecution) => void): ExecuteInTerminalDeps {
    return {
      subscribe: vscode.window.onDidChangeTerminalShellIntegration,
      onDidStartExecution: vscode.window.onDidStartTerminalShellExecution,
      log: (msg) => this.outputChannel.appendLine(msg),
      timeoutMs: SHELL_INTEGRATION_TIMEOUT_MS,
      onExecutionStarted,
    };
  }

  private reportBinaryNotFound(command: string): void {
    const label = command === 'claude' ? 'Claude Code' : 'Copilot';
    vscode.window.showErrorMessage(`Could not find \`${command}\` in PATH. Install the ${label} CLI to resume sessions.`);
    this.outputChannel.appendLine(`[terminal] \`${command}\` executable not found in PATH.`);
  }

  /** Call on a manual refresh so a CLI install that happened mid-session is picked up without a window reload. */
  public clearBinaryCaches(): void {
    this.binaryCheckCache.clear();
  }

  private hasBinaryOnPath(command: string): Promise<boolean> {
    let check = this.binaryCheckCache.get(command);
    if (!check) {
      check = this.probeBinaryOnPath(command);
      this.binaryCheckCache.set(command, check);
    }
    return check;
  }

  /** VS Code's extension host often has a trimmed PATH missing what an interactive shell profile adds — falls back to checking via the user's configured shell before concluding a CLI isn't installed. */
  private async probeBinaryOnPath(command: string): Promise<boolean> {
    const checker = process.platform === 'win32' ? 'where' : 'which';

    try {
      await execFileAsync(checker, [command]);
      return true;
    } catch {
      try {
        const shell = vscode.env.shell || '/bin/sh';
        await execFileAsync(checker, [command], { env: { ...process.env, SHELL: shell } });
        return true;
      } catch {
        try {
          // execFile, not exec — only the shell's -c argument (a fixed string) is shell-interpreted.
          const shell = vscode.env.shell || '/bin/sh';
          await execFileAsync(shell, ['-l', '-c', `${checker} ${command}`]);
          return true;
        } catch {
          return false;
        }
      }
    }
  }
}

/** Every `.jsonl` session file path currently under `root`, for `correlateNewSession`'s before/after diff. */
function snapshotSessionFilePaths(root: string): Set<string> {
  const result = new Set<string>();
  for (const dirName of listProjectDirNames()) {
    for (const file of listSessionFiles(dirName)) {
      result.add(path.join(root, dirName, file));
    }
  }
  return result;
}

function truncate(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}

interface ExecuteInTerminalDeps {
  readonly subscribe: typeof vscode.window.onDidChangeTerminalShellIntegration;
  readonly onDidStartExecution: typeof vscode.window.onDidStartTerminalShellExecution;
  readonly log: (message: string) => void;
  readonly timeoutMs: number;
  /** Called with the `TerminalShellExecution` the moment `executeCommand` actually runs, whichever path gets there — lets a caller (`launch`) correlate it to a session id for `executionEndListener`. */
  readonly onExecutionStarted?: (execution: vscode.TerminalShellExecution) => void;
}

/** Runs a command via shell integration once active, avoiding a race where a slow-starting shell eats the first characters sent via plain `sendText`. Falls back to `sendText` if integration never activates. */
function executeInTerminal(terminal: vscode.Terminal, command: string, deps: ExecuteInTerminalDeps): Promise<void> {
  const { subscribe, onDidStartExecution, log, timeoutMs, onExecutionStarted } = deps;

  if (terminal.shellIntegration) {
    log('[terminal] Shell integration available, using executeCommand.');
    onExecutionStarted?.(terminal.shellIntegration.executeCommand(command));
    return awaitCommandStart(terminal, onDidStartExecution, log, timeoutMs);
  }

  return new Promise<void>((resolve) => {
    let executed = false;

    const listener = subscribe(({ terminal: t, shellIntegration }) => {
      if (t === terminal && !executed) {
        executed = true;
        listener.dispose();
        log('[terminal] Shell integration activated, using executeCommand.');
        onExecutionStarted?.(shellIntegration.executeCommand(command));
        awaitCommandStart(terminal, onDidStartExecution, log, timeoutMs).then(resolve);
      }
    });

    setTimeout(() => {
      if (!executed) {
        executed = true;
        listener.dispose();
        log('[terminal] Shell integration not available after timeout, falling back to sendText.');
        terminal.sendText(command, true);
        resolve();
      }
    }, timeoutMs);
  });
}

/** Keeps the progress toast up until the command has actually started (or a safeguard timeout fires). */
function awaitCommandStart(
  terminal: vscode.Terminal,
  onDidStartExecution: ExecuteInTerminalDeps['onDidStartExecution'],
  log: ExecuteInTerminalDeps['log'],
  timeoutMs: number
): Promise<void> {
  return new Promise<void>((resolve) => {
    let resolved = false;

    const listener = onDidStartExecution(({ terminal: t }) => {
      if (t === terminal && !resolved) {
        resolved = true;
        listener.dispose();
        log('[terminal] Command execution started.');
        resolve();
      }
    });

    setTimeout(() => {
      if (!resolved) {
        resolved = true;
        listener.dispose();
        resolve();
      }
    }, timeoutMs);
  });
}
