import { execFile } from 'child_process';
import { promisify } from 'util';
import * as fs from 'fs';
import * as path from 'path';
import * as vscode from 'vscode';
import { getClaudeProjectsDir, listProjectDirNames, listSessionFiles } from '../discovery/claudeStorage';
import { SessionNode } from '../tree/sessionProvider';
import { agentIconPath } from '../tree/agentIcons';
import { clearSessionStatus, markSessionError } from '../status/sessionStatus';
import { buildClaudeNewSessionCommand, buildClaudeResumeCommand } from './claudeCommand';
import { buildCopilotResumeCommand } from './copilotCommand';

const execFileAsync = promisify(execFile);

/** Time to wait for shell integration before falling back to sendText. */
const SHELL_INTEGRATION_TIMEOUT_MS = 500;

/**
 * How long to wait for a brand-new session's transcript file to appear
 * before giving up on correlating it to its terminal. Generous on purpose:
 * shell integration never activates in some environments (a slow-loading
 * shell profile, or shell integration disabled/unsupported), which means
 * `claude` is only started via a blind `sendText` 500ms after the terminal
 * is created — if the shell itself is still initializing at that point, the
 * actual `claude` process (and thus its `.jsonl` file) can start noticeably
 * later than a plain terminal launch would suggest.
 */
const NEW_SESSION_DISCOVERY_TIMEOUT_MS = 60_000;

/** How often to re-scan `~/.claude/projects` while waiting for a new session file — see `correlateNewSession`. */
const NEW_SESSION_POLL_INTERVAL_MS = 500;

export interface OpenSessionTerminalOptions {
  readonly dangerouslySkipPermissions?: boolean;
}

/**
 * Launches `claude --resume <sessionId>` in a real terminal — ported from the
 * reference extension's `terminal.ts` (cloned and read directly).
 *
 * One terminal *per session*, tracked in `sessionTerminals`: switching to a
 * different session never touches whatever's running in another session's
 * terminal — no interrupt, no visible "logout/login" flicker, and (critically)
 * no orphaned live-status: a session left running in the background keeps
 * firing its own `UserPromptSubmit`/`Stop`/`Notification` hooks normally,
 * exactly as if you'd never looked away. An earlier version of this file tried
 * a single shared terminal (interrupting the running session with Ctrl+C to
 * reuse the tab for a different one) to match this project's single-tab
 * philosophy — but that philosophy only holds for *passive* views like the
 * read-only transcript tab; there's no way to "pause" a foreground CLI process
 * without either killing it or moving it out of the way, so a live terminal
 * needs a real process per session. Verified against the actual desktop
 * "Claude Terminal" Electron app (`Sterll/claude-terminal`), which keeps the
 * same shape: a `Map<id, ptyProcess>` of concurrently-alive processes, never
 * one shared pty for every session.
 */
export class AgentTerminalService implements vscode.Disposable {
  private readonly sessionTerminals = new Map<string, vscode.Terminal>();
  /** Display label per tracked session id — `session.displayName` for a resume, the project name for a just-started new session (its real title isn't known yet). Used only for `WaitingNotifier`'s notification text. */
  private readonly sessionLabels = new Map<string, string>();
  /** Session ids already claimed by a pending `correlateNewSession` call, so two "New Session" clicks in quick succession can't both grab the same newly-created file. */
  private readonly claimedByCorrelation = new Set<string>();
  private readonly closeListener: vscode.Disposable;
  /** Memoized PATH check per CLI command ("claude", "copilot") — PATH doesn't change mid-session, so there's no need to re-probe it (up to 3 sequential subprocess spawns) on every single session launch. Cleared on manual refresh, same as `gitProject.ts`'s and `claudeStorage.ts`'s caches. */
  private readonly binaryCheckCache = new Map<string, Promise<boolean>>();
  private readonly _onDidChangeOpenSessions = new vscode.EventEmitter<void>();
  /** Fires whenever a session gains or loses a tracked open terminal — what `activeSessionProvider.ts`'s Explorer view refreshes on. */
  public readonly onDidChangeOpenSessions = this._onDidChangeOpenSessions.event;
  /** Which session a just-launched `claude --resume` execution belongs to — see `executionEndListener` and `launch`. */
  private readonly sessionIdByExecution = new Map<vscode.TerminalShellExecution, string>();
  private readonly executionEndListener: vscode.Disposable;

  public constructor(private readonly outputChannel: vscode.OutputChannel, private readonly extensionUri: vscode.Uri) {
    this.closeListener = vscode.window.onDidCloseTerminal((terminal) => {
      for (const [sessionId, tracked] of this.sessionTerminals) {
        if (tracked === terminal) {
          this.untrackTerminal(sessionId);
          // Closing the terminal kills the process tree outright (no graceful
          // shutdown on Windows without WSL/tmux), so Claude Code may never get
          // to fire its own SessionEnd hook — clear the status ourselves so the
          // dot doesn't get stuck showing "running"/"waiting" forever for a
          // session that's actually dead. Idempotent if it already exited
          // cleanly and the hook beat us to it.
          clearSessionStatus(sessionId);
          this.outputChannel.appendLine(`[terminal] Terminal for session ${sessionId} closed; status cleared.`);
          break;
        }
      }
    });

    /**
     * Neither agent's hooks expose a distinct failure signal (confirmed for Claude Code: `Stop` fires
     * the same way on a clean turn end or a fatal error; Copilot has no hook mechanism wired up at all
     * yet) — this infers "error" instead from the exit code of the `claude`/`copilot` command itself,
     * via shell integration's per-command completion event, agent-agnostic. Deliberately only acts on a
     * real, positive exit code: `exitCode` comes back `undefined` for a Ctrl+C cancel, a sub-shell being
     * opened, or a shell integration script that isn't reporting properly (documented on
     * `TerminalShellExecutionEndEvent.exitCode`), so those are correctly left alone rather than misread
     * as a crash.
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

  /**
   * Closes every session terminal Session Deck opened, on top of the base
   * `closeListener` teardown — called when the extension host shuts down
   * (window close/reload, wired via `context.subscriptions` in extension.ts).
   * Each terminal already opts out of VS Code's persistent-session restore via
   * `isTransient: true` at creation time (see `launch`/`startNewSession`) —
   * that's the part that actually matters, since `deactivate()`/`dispose()`
   * aren't reliably called on an abrupt window close (only on a graceful
   * reload/disable), so nothing here could be depended on to run in time
   * anyway. This is just belt-and-suspenders cleanup for the cases where it
   * *does* run: closing the terminal outright instead of leaving it open with
   * a dead process, and clearing its status so a "running"/"waiting" dot
   * doesn't get stuck for a session Claude Code never got to send its own
   * `SessionEnd` for.
   */
  public dispose(): void {
    this.closeListener.dispose();
    this.executionEndListener.dispose();
    for (const [sessionId, terminal] of this.sessionTerminals) {
      clearSessionStatus(sessionId);
      terminal.dispose();
    }
    this.sessionTerminals.clear();
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

  /**
   * Closes this session's terminal if it's currently tracked — a no-op otherwise. Used when a session
   * is archived (manually, or auto-evicted by the 5-per-project cap): an archived session shouldn't be
   * left running with a now-hidden tab. `terminal.dispose()` triggers the same `onDidCloseTerminal`
   * bookkeeping (untrack + clear status) as any other terminal close, so nothing extra to do here.
   */
  public closeSessionTerminal(sessionId: string): void {
    this.sessionTerminals.get(sessionId)?.dispose();
  }

  private trackTerminal(sessionId: string, terminal: vscode.Terminal, label: string): void {
    this.sessionTerminals.set(sessionId, terminal);
    this.sessionLabels.set(sessionId, label);
    this._onDidChangeOpenSessions.fire();
  }

  private untrackTerminal(sessionId: string): void {
    this.sessionTerminals.delete(sessionId);
    this.sessionLabels.delete(sessionId);
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

  /**
   * The explicit escape hatch (a tree-item button next to "Rename Session",
   * not the default click): always opens a brand-new terminal for this
   * session, even if one is already tracked — e.g. to run it twice side by
   * side, or to recover from a terminal stuck in a broken state. Also used for
   * the "Skip Permissions" resume, deliberately bypassing the reuse check: a
   * dangerous relaunch should never silently reuse (and thus ignore the flag
   * on) an already-open, non-dangerous terminal for the same session.
   */
  public async openSessionInNewTerminal(session: SessionNode, options: OpenSessionTerminalOptions = {}): Promise<void> {
    await this.launch(session, options);
  }

  /**
   * Starts a brand-new Claude Code session (plain `claude`, no `--resume`)
   * rooted at `cwd` — always a fresh terminal, since there's no existing
   * session id to reuse by yet.
   *
   * There's no CLI flag to pre-assign a session id (checked: `claude --help`
   * has no `--session-id`; `--name` is a separate alias on top of the
   * auto-generated id, not a replacement for it, so it doesn't help here
   * either), so the real id can only be learned after the fact via
   * `correlateNewSession`. Once that resolves, clicking the session's tree
   * entry (once it shows up) reuses this same terminal via `openSession`,
   * exactly like any other tracked session, instead of opening a redundant
   * second one.
   */
  public async startNewSession(cwd: string, projectName: string, options: OpenSessionTerminalOptions = {}): Promise<void> {
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
      `[terminal] Starting a new session in ${cwd} (skipPermissions=${String(dangerouslySkipPermissions)}).`
    );

    await vscode.window.withProgress(
      { location: vscode.ProgressLocation.Notification, title: 'Waiting for terminal to be ready…' },
      () => executeInTerminal(terminal, command, this.terminalDeps())
    );
  }

  /**
   * Polls `~/.claude/projects` for the next brand-new session file to appear,
   * then registers `terminal` under that file's session id (the same map
   * `openSession` checks).
   *
   * Deliberately does **not** wait for that file to record a `cwd` and match
   * it against the cwd this terminal started at: a session's `cwd` is only
   * written on its first actual user turn (confirmed against real transcript
   * data), not on the startup metadata records Claude Code writes
   * immediately — so if you click the session's tree entry before typing
   * anything into the fresh terminal, that cwd-matching approach would still
   * be waiting and never correlate in time, which was exactly the bug this
   * replaced.
   *
   * This used to watch for `fs.watch`'s `rename` event instead of polling —
   * fires exactly once, specifically when a path is created (confirmed
   * empirically: appending to an existing file fires `change`, not
   * `rename`), so in principle a lighter-weight signal than polling. In
   * practice, on a real `~/.claude/projects` tree (dozens of projects, one of
   * them potentially *this very Claude Code conversation* being actively
   * appended to while the user works), that recursive watch reliably dropped
   * the new file's `rename` event outright — confirmed via output-channel
   * logging showing only `change` events for an unrelated, already-existing
   * file, then a full timeout, twice in a row, even though the new session's
   * file did exist by the time the tree was manually refreshed. This is a
   * known reliability limitation of Windows' `ReadDirectoryChangesW`-backed
   * recursive watching under a large/busy tree (its notification buffer can
   * silently overflow), made worse here by `SessionTreeProvider.watch()`
   * already running its own independent recursive watcher on the same
   * directory. Polling sidesteps OS notification delivery entirely: each tick
   * just re-reads the actual directory structure directly.
   *
   * `claimedByCorrelation` guards against two "New Session" clicks in quick
   * succession both grabbing the same file were one to appear while both
   * pollers are still active.
   *
   * Fire-and-forget from the caller's perspective — gives up silently after
   * {@link NEW_SESSION_DISCOVERY_TIMEOUT_MS}, in which case a later click on
   * that session just falls back to a second terminal, no worse than before
   * this existed.
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
          this.trackTerminal(sessionId, terminal, projectName);
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
   * Drops the "New: " prefix from the tab name once a session is actually
   * correlated, so it stops looking like a not-yet-real session. There's no
   * API to rename a `vscode.Terminal` directly — the only way is the
   * `workbench.action.terminal.renameWithArg` command, which also switches
   * the *visible* terminal tab to whichever one it renames (a `show()` side
   * effect that `preserveFocus` doesn't prevent — it only affects keyboard
   * focus, not which tab is displayed). Correlation can take several seconds
   * (a slow-starting shell profile has been observed pushing it past 10s), so
   * by the time it resolves the user may well have switched to a different
   * tab; forcibly yanking that back just to fix a label would be a worse
   * surprise than leaving the stale name. Only renames when nothing would
   * visibly move.
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
    this.trackTerminal(session.sessionId, terminal, session.displayName);
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

  /**
   * VS Code's extension host often has a trimmed PATH that excludes what a
   * user's interactive shell profile (~/.zshrc, ~/.bashrc, PowerShell $PROFILE)
   * adds — falls back to checking through the user's actual configured shell
   * before concluding the CLI really isn't installed.
   */
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
          // execFile (not exec) so vscode.env.shell is invoked directly rather than parsed by
          // an intermediate shell — only its `-c` argument (a fixed, non-interpolated string) is
          // ever shell-interpreted.
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

/**
 * Runs a command in a terminal via shell integration when available, waiting
 * for it to activate first — this avoids a real race where a slow-starting
 * shell (oh-my-zsh update checks, etc.) eats the first characters of a command
 * sent via plain `sendText`. Falls back to `sendText` if shell integration
 * never activates within the timeout.
 */
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
