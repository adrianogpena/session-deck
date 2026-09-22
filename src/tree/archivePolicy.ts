/**
 * Pure decision logic for auto-archiving a project's older sessions once a new one pushes it past the
 * cap (`MAX_SESSIONS_PER_PROJECT_VIEW` in sessionProvider.ts) — kept vscode-free so it's directly
 * unit-testable; see test/suite/archivePolicy.test.ts. Everything else about *when* this runs (only on
 * a genuinely new session, never on first sight of a project so pre-existing sessions are left alone)
 * and what happens to an evicted session's terminal lives in sessionProvider.ts / terminalService.ts.
 */

/**
 * `activeIdsByRecency` is every non-archived session id for one project, most recently modified first.
 * `newlyDiscoveredIds` is whichever of those weren't seen on the previous pass — the signal that "a new
 * session showed up" rather than just "there happen to be more than `cap` already, as there always have
 * been". Returns the ids (oldest first) that should be archived to bring the active count back to `cap`.
 */
export function selectSessionsToArchive(
  activeIdsByRecency: readonly string[],
  cap: number,
  newlyDiscoveredIds: ReadonlySet<string>
): string[] {
  if (newlyDiscoveredIds.size === 0) {
    return [];
  }
  return activeIdsByRecency.slice(cap);
}
