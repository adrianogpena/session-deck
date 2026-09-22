import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mapWithConcurrency } from '../../concurrency';

function deferred<T>(): { promise: Promise<T>; resolve: (value: T) => void } {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => (resolve = r));
  return { promise, resolve };
}

test('mapWithConcurrency preserves input order regardless of completion order', async () => {
  const delays = [30, 10, 20, 0];
  const result = await mapWithConcurrency(delays, 4, (ms) => new Promise((r) => setTimeout(() => r(ms), ms)));
  assert.deepEqual(result, delays);
});

test('mapWithConcurrency never runs more than `limit` calls at once', async () => {
  const items = Array.from({ length: 10 }, (_, i) => i);
  let inFlight = 0;
  let maxInFlight = 0;

  await mapWithConcurrency(items, 3, async (item) => {
    inFlight++;
    maxInFlight = Math.max(maxInFlight, inFlight);
    await new Promise((r) => setTimeout(r, 5));
    inFlight--;
    return item;
  });

  assert.ok(maxInFlight <= 3, `expected at most 3 concurrent calls, saw ${maxInFlight}`);
  assert.equal(maxInFlight, 3, 'expected the limit to actually be reached with 10 items and a limit of 3');
});

test('mapWithConcurrency with a limit larger than the item count still works', async () => {
  const result = await mapWithConcurrency([1, 2, 3], 100, async (n) => n * 2);
  assert.deepEqual(result, [2, 4, 6]);
});

test('mapWithConcurrency handles an empty input', async () => {
  const result = await mapWithConcurrency([], 5, async () => {
    throw new Error('fn should never be called for an empty input');
  });
  assert.deepEqual(result, []);
});

test('mapWithConcurrency propagates a rejection', async () => {
  await assert.rejects(
    () =>
      mapWithConcurrency([1, 2, 3], 2, async (n) => {
        if (n === 2) {
          throw new Error('boom');
        }
        return n;
      }),
    /boom/
  );
});

test('mapWithConcurrency starts the next item as soon as a slot frees up, not in fixed-size chunks', async () => {
  // Item 0 is slow; items 1 and 2 are fast. With a limit of 2, item 2 should start as soon as
  // item 1 finishes, well before item 0 (the slow one) ever completes — proving this is a
  // worker pool, not batching into fixed chunks of `limit` that wait for the whole chunk.
  const order: number[] = [];
  const slow = deferred<void>();

  const runPromise = mapWithConcurrency([0, 1, 2], 2, async (n) => {
    if (n === 0) {
      await slow.promise;
    }
    order.push(n);
    return n;
  });

  // Let items 1 (and, if incorrectly gated, 2) get a chance to run before unblocking item 0.
  await new Promise((r) => setTimeout(r, 20));
  assert.deepEqual(order, [1, 2], 'items 1 and 2 should both have completed while item 0 is still pending');

  slow.resolve();
  await runPromise;
  assert.deepEqual(order, [1, 2, 0]);
});
