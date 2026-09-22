import { execFile, exec } from 'child_process';
import { promisify } from 'util';
import * as vscode from 'vscode';
import { SessionNode } from './sessionProvider';

const execFileAsync = promisify(execFile);
const execAsync = promisify(exec);

/** Time to wait for shell integration before falling back to sendText. */
const SHELL_INTEGRATION_TIMEOUT_MS = 500;

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
