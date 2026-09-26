/** Session ids are plain UUID-shaped tokens — enforced before interpolating one into a terminal command, since that never needs shell-specific quoting (bash, PowerShell, or cmd.exe). */
const SAFE_SESSION_ID = /^[A-Za-z0-9-]+$/;

export function assertSafeSessionId(sessionId: string): string {
  if (!SAFE_SESSION_ID.test(sessionId)) {
    throw new Error(`Refusing to build a terminal command for an unexpected session id: ${JSON.stringify(sessionId)}`);
  }
  return sessionId;
}
