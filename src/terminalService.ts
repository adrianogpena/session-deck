import { execFile, exec } from 'child_process';
import { promisify } from 'util';
import * as fs from 'fs';
import * as path from 'path';
import * as vscode from 'vscode';
import { getClaudeProjectsDir, readSessionMeta } from './claudeStorage';
import { normalizeFsPath } from './pathUtils';
import { SessionNode } from './sessionProvider';

const execFileAsync = promisify(execFile);
const execAsync = promisify(exec);

/** Time to wait for shell integration before falling back to sendText. */
const SHELL_INTEGRATION_TIMEOUT_MS = 500;

/** How long to wait for a brand-new session's transcript file to appear before giving up on correlating it to its terminal. */
const NEW_SESSION_DISCOVERY_TIMEOUT_MS = 15_000;

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
export class ClaudeTerminalService implements vscode.Disposable {
  private readonly sessionTerminals = new Map<string, vscode.Terminal>();
  private readonly closeListener: vscode.Disposable;

  public constructor(private readonly outputChannel: vscode.OutputChannel) {
    this.closeListener = vscode.window.onDidCloseTerminal((terminal) => {
      for (const [sessionId, tracked] of this.sessionTerminals) {
        if (tracked === terminal) {
          this.sessionTerminals.delete(sessionId);
          break;
        }
      }
    });
  }

  public dispose(): void {
    this.closeListener.dispose();
  }

  /** The default action: reuses (just `.show()`s) this session's own terminal if it's still alive, otherwise opens a new one. */
  public async openSession(session: SessionNode, options: OpenSessionTerminalOptions = {}): Promise<void> {
    const existing = this.sessionTerminals.get(session.sessionId);
    if (existing && existing.exitStatus === undefined) {
      existing.show();
      return;
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
   * either), so the real id can only be learned after the fact. Claude Code
   * writes its startup metadata records to the new transcript file almost
   * immediately — before you've typed anything — so `correlateNewSession`
   * watches `~/.claude/projects` for that file to appear and registers this
   * terminal against its real session id the moment it does. Once that
   * happens, clicking the session's tree entry (once it shows up) reuses this
   * same terminal via `openSession`, exactly like any other tracked session,
   * instead of opening a redundant second one.
   */
  public async startNewSession(cwd: string, projectName: string, options: OpenSessionTerminalOptions = {}): Promise<void> {
    const dangerouslySkipPermissions = options.dangerouslySkipPermissions === true;

    const hasClaude = await this.hasClaudeBinary();
    if (!hasClaude) {
      this.reportClaudeNotFound();
      return;
    }

    const terminal = vscode.window.createTerminal({
      name: truncate(`New: ${projectName}`, 35),
      cwd,
      location: { viewColumn: vscode.ViewColumn.Active },
    });
    terminal.show(true);
    void this.correlateNewSession(cwd, terminal);

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
   * Watches `~/.claude/projects` for a new `.jsonl` file whose recorded `cwd`
   * matches, then registers `terminal` under that file's session id (the same
   * map `openSession` checks). Fire-and-forget from the caller's perspective —
   * gives up silently after {@link NEW_SESSION_DISCOVERY_TIMEOUT_MS}, in which
   * case a later click on that session just falls back to today's behavior
   * (a second terminal), no worse than before this existed.
   */
  private correlateNewSession(cwd: string, terminal: vscode.Terminal): Promise<void> {
    const root = getClaudeProjectsDir();
    if (!fs.existsSync(root)) {
      return Promise.resolve();
    }

    return new Promise<void>((resolve) => {
      let settled = false;
      const finish = () => {
        if (settled) {
          return;
        }
        settled = true;
        clearTimeout(timeout);
        watcher.close();
        resolve();
      };

      let watcher: fs.FSWatcher;
      try {
        watcher = fs.watch(root, { recursive: true }, (_eventType, filename) => {
          if (settled || !filename || !filename.toString().endsWith('.jsonl')) {
            return;
          }
          const filePath = path.join(root, filename.toString());
          readSessionMeta(filePath)
            .then((meta) => {
              if (settled || !meta.cwd || normalizeFsPath(meta.cwd) !== normalizeFsPath(cwd)) {
                return;
              }
              const sessionId = path.basename(filePath, '.jsonl');
              this.sessionTerminals.set(sessionId, terminal);
              this.outputChannel.appendLine(`[terminal] New session ${sessionId} correlated with its terminal.`);
              finish();
            })
            .catch(() => {
              // A file mid-write when we stat it isn't necessarily ours; the next change event will retry.
            });
        });
      } catch {
        return resolve();
      }

      const timeout = setTimeout(finish, NEW_SESSION_DISCOVERY_TIMEOUT_MS);
    });
  }

  private async launch(session: SessionNode, options: OpenSessionTerminalOptions): Promise<void> {
    const dangerouslySkipPermissions = options.dangerouslySkipPermissions === true;

    const hasClaude = await this.hasClaudeBinary();
    if (!hasClaude) {
      this.reportClaudeNotFound();
      return;
    }

    const terminal = vscode.window.createTerminal({
      name: truncate(session.displayName, 35),
      cwd: session.cwd,
      location: { viewColumn: vscode.ViewColumn.Active },
    });
    this.sessionTerminals.set(session.sessionId, terminal);
    terminal.show(true);

    const resumeCommand = buildClaudeResumeCommand(session.sessionId, dangerouslySkipPermissions);
    this.outputChannel.appendLine(
      `[terminal] Opening session ${session.sessionId} (skipPermissions=${String(dangerouslySkipPermissions)}).`
    );

    await vscode.window.withProgress(
      { location: vscode.ProgressLocation.Notification, title: 'Waiting for terminal to be ready…' },
      () => executeInTerminal(terminal, resumeCommand, this.terminalDeps())
    );
  }

  private terminalDeps(): ExecuteInTerminalDeps {
    return {
      subscribe: vscode.window.onDidChangeTerminalShellIntegration,
      onDidStartExecution: vscode.window.onDidStartTerminalShellExecution,
      log: (msg) => this.outputChannel.appendLine(msg),
      timeoutMs: SHELL_INTEGRATION_TIMEOUT_MS,
    };
  }

  private reportClaudeNotFound(): void {
    vscode.window.showErrorMessage('Could not find `claude` in PATH. Install Claude Code CLI to resume sessions.');
    this.outputChannel.appendLine('[terminal] `claude` executable not found in PATH.');
  }

  /**
   * VS Code's extension host often has a trimmed PATH that excludes what a
   * user's interactive shell profile (~/.zshrc, ~/.bashrc, PowerShell $PROFILE)
   * adds — falls back to checking through the user's actual configured shell
   * before concluding `claude` really isn't installed.
   */
  private async hasClaudeBinary(): Promise<boolean> {
    const checker = process.platform === 'win32' ? 'where' : 'which';

    try {
      await execFileAsync(checker, ['claude']);
      return true;
    } catch {
      try {
        const shell = vscode.env.shell || '/bin/sh';
        await execAsync(`${checker} claude`, { env: { ...process.env, SHELL: shell } });
        return true;
      } catch {
        try {
          const shell = vscode.env.shell || '/bin/sh';
          await execAsync(`"${shell}" -l -c "${checker} claude"`);
          return true;
        } catch {
          return false;
        }
      }
    }
  }
}

export function buildClaudeResumeCommand(sessionId: string, dangerouslySkipPermissions: boolean): string {
  const args = ['claude'];
  if (dangerouslySkipPermissions) {
    args.push('--dangerously-skip-permissions');
  }
  args.push('--resume', shellQuote(sessionId));
  return args.join(' ');
}

export function buildClaudeNewSessionCommand(dangerouslySkipPermissions: boolean): string {
  return dangerouslySkipPermissions ? 'claude --dangerously-skip-permissions' : 'claude';
}

function shellQuote(value: string): string {
  return `'${value.replace(/'/g, `'\\''`)}'`;
}

function truncate(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}

interface ExecuteInTerminalDeps {
  readonly subscribe: typeof vscode.window.onDidChangeTerminalShellIntegration;
  readonly onDidStartExecution: typeof vscode.window.onDidStartTerminalShellExecution;
  readonly log: (message: string) => void;
  readonly timeoutMs: number;
}

/**
 * Runs a command in a terminal via shell integration when available, waiting
 * for it to activate first — this avoids a real race where a slow-starting
 * shell (oh-my-zsh update checks, etc.) eats the first characters of a command
 * sent via plain `sendText`. Falls back to `sendText` if shell integration
 * never activates within the timeout.
 */
function executeInTerminal(terminal: vscode.Terminal, command: string, deps: ExecuteInTerminalDeps): Promise<void> {
  const { subscribe, onDidStartExecution, log, timeoutMs } = deps;

  if (terminal.shellIntegration) {
    log('[terminal] Shell integration available, using executeCommand.');
    terminal.shellIntegration.executeCommand(command);
    return awaitCommandStart(terminal, onDidStartExecution, log, timeoutMs);
  }

  return new Promise<void>((resolve) => {
    let executed = false;

    const listener = subscribe(({ terminal: t, shellIntegration }) => {
      if (t === terminal && !executed) {
        executed = true;
        listener.dispose();
        log('[terminal] Shell integration activated, using executeCommand.');
        shellIntegration.executeCommand(command);
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
