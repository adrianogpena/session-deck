/**
 * Both Claude Code and Copilot CLI session ids are plain UUID-shaped tokens (hex digits and hyphens).
 * Enforced here before either's resume command interpolates one into a string sent to a live terminal —
 * a bare token matching this never needs quoting to stay a single, unambiguous argument in bash,
 * PowerShell, *or* cmd.exe alike, which sidesteps having to pick a shell-specific quoting style
 * (POSIX-only single-quote escaping would be silently wrong for a Windows terminal on cmd.exe or
 * PowerShell). vscode-free, like the command builders that use it, so it stays directly unit-testable.
 */
const SAFE_SESSION_ID = /^[A-Za-z0-9-]+$/;

export function assertSafeSessionId(sessionId: string): string {
  if (!SAFE_SESSION_ID.test(sessionId)) {
    throw new Error(`Refusing to build a terminal command for an unexpected session id: ${JSON.stringify(sessionId)}`);
  }
  return sessionId;
}
