/**
 * Pure decision logic for auto-stopping (closing the terminal/PTY of) a project's sessions once
 * they've sat idle past a threshold — vscode-free so it's directly unit-testable
 * (test/stopIdlePolicy.test.ts). Companion to archivePolicy.ts: this only stops a session's live
 * process, it never archives it.
 */

/**
 * `openSessions`: every session id that currently has a live terminal/PTY open, with the mtime of
 * its last recorded activity. Returns the ids that have been idle for at least `idleMinutes`.
 */
export function selectSessionsToStop(
  openSessions: readonly { sessionId: string; lastActivity: Date }[],
  idleMinutes: number,
  now: Date = new Date()
): string[] {
  const thresholdMs = idleMinutes * 60_000;
  return openSessions
    .filter((s) => now.getTime() - s.lastActivity.getTime() >= thresholdMs)
    .map((s) => s.sessionId);
}
