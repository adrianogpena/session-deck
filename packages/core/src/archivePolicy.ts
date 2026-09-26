/** Pure decision logic for auto-archiving a project's older sessions past the cap — vscode-free so it's directly unit-testable (test/suite/archivePolicy.test.ts). */

/**
 * `activeIdsByRecency`: every non-archived session id for one project, most recent first.
 * `newlyDiscoveredIds`: ids not seen on the previous pass — archiving only triggers when something
 * new showed up, not just because there are already more than `cap`. Returns the ids (oldest first)
 * to archive to get back to `cap`.
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
