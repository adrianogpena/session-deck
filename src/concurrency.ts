/**
 * Runs `fn` over `items` with at most `limit` concurrent in-flight calls, preserving input order in
 * the result array. Plain `Promise.all(items.map(fn))` has no concurrency cap at all — fine for a
 * handful of items, but a real resource risk once there are dozens/hundreds (e.g. spawning that many
 * `git` processes at once for git-root resolution, or reading that many session transcripts/opening
 * that many SQLite connections at once for search) — a worker-pool over a shared index cursor, the
 * standard pattern for this, rather than chunking into fixed-size batches (which would leave earlier
 * workers idle waiting for the slowest item in their chunk instead of immediately picking up the
 * next pending item).
 */
export async function mapWithConcurrency<T, R>(
  items: readonly T[],
  limit: number,
  fn: (item: T, index: number) => Promise<R>
): Promise<R[]> {
  const results: R[] = new Array(items.length);
  let nextIndex = 0;

  async function worker(): Promise<void> {
    for (;;) {
      const i = nextIndex++;
      if (i >= items.length) {
        return;
      }
      results[i] = await fn(items[i], i);
    }
  }

  const workerCount = Math.max(1, Math.min(limit, items.length));
  await Promise.all(Array.from({ length: workerCount }, () => worker()));
  return results;
}
