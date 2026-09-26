/**
 * Runs `fn` over `items` with at most `limit` concurrent in-flight calls, preserving input order.
 * A worker-pool over a shared index cursor, not fixed-size batching — a worker picks up the next
 * pending item as soon as it's free, rather than waiting on the slowest item in its own batch.
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
