/**
 * Session ids are plain UUID-shaped tokens (both agents' own, and Session Deck's `randomUUID()`-issued
 * ones) — enforced wherever one is interpolated into a terminal command (never needs shell-specific
 * quoting: bash, PowerShell, or cmd.exe) or built into a filesystem path (never needs escaping, and
 * can't be turned into `..` traversal), so a malformed id from an untrusted or corrupted source (e.g.
 * a hand-edited or third-party-written state file) fails loudly instead of being used as-is.
 */
const SAFE_SESSION_ID = /^[A-Za-z0-9-]+$/;

export function assertSafeSessionId(sessionId: string): string {
  if (!SAFE_SESSION_ID.test(sessionId)) {
    throw new Error(`Refusing to use an unexpected session id: ${JSON.stringify(sessionId)}`);
  }
  return sessionId;
}

export function isSafeSessionId(sessionId: string): boolean {
  return SAFE_SESSION_ID.test(sessionId);
}
