import { GitStatus, mapWithConcurrency, readGitStatus } from '@session-deck/core';

const POLL_CONCURRENCY = 6;

/** One `git status` per distinct cwd, cached between polls — `get` never blocks, `poll` is what actually shells out. */
export class GitStatusTracker {
  private byCwd = new Map<string, GitStatus | undefined>();

  get(cwd: string): GitStatus | undefined {
    return this.byCwd.get(cwd);
  }

  /** Re-reads every distinct cwd in `cwds`, dropping ones no longer passed in. */
  async poll(cwds: readonly string[]): Promise<void> {
    const unique = [...new Set(cwds)];
    const statuses = await mapWithConcurrency(unique, POLL_CONCURRENCY, (cwd) => readGitStatus(cwd));
    const next = new Map<string, GitStatus | undefined>();
    unique.forEach((cwd, i) => next.set(cwd, statuses[i]));
    this.byCwd = next;
  }
}
