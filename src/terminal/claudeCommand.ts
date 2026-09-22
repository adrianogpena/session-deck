/**
 * A Claude Code session id is always a plain UUID-shaped token (hex digits and hyphens). Enforced here
 * before it's ever interpolated into a command string sent to a live terminal — a bare token matching
 * this never needs quoting to stay a single, unambiguous argument in bash, PowerShell, *or* cmd.exe
 * alike, which sidesteps having to pick a shell-specific quoting style (POSIX-only single-quote escaping
 * would be silently wrong for a Windows terminal on cmd.exe or PowerShell).
 *
 * Kept in its own vscode-free module (unlike the rest of terminalService.ts) so this — the part that
 * actually decides what gets typed into a shell — can be unit-tested directly; see test/suite/claudeCommand.test.ts.
 */
const SAFE_SESSION_ID = /^[A-Za-z0-9-]+$/;

function assertSafeSessionId(sessionId: string): string {
  if (!SAFE_SESSION_ID.test(sessionId)) {
    throw new Error(`Refusing to build a terminal command for an unexpected session id: ${JSON.stringify(sessionId)}`);
  }
  return sessionId;
}

export function buildClaudeResumeCommand(sessionId: string, dangerouslySkipPermissions: boolean): string {
  const args = ['claude'];
  if (dangerouslySkipPermissions) {
    args.push('--dangerously-skip-permissions');
  }
  args.push('--resume', assertSafeSessionId(sessionId));
  return args.join(' ');
}

export function buildClaudeNewSessionCommand(dangerouslySkipPermissions: boolean): string {
  return dangerouslySkipPermissions ? 'claude --dangerously-skip-permissions' : 'claude';
}
