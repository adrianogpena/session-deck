import { test } from 'node:test';
import assert from 'node:assert/strict';
import { humanizeSince } from '../format';

const NOW = Date.UTC(2026, 8, 26, 12, 0, 0);
const MIN = 60_000;

test('humanizeSince buckets elapsed time', () => {
  assert.equal(humanizeSince(NOW - 30_000, NOW), 'just now');
  assert.equal(humanizeSince(NOW - 5 * MIN, NOW), '5m ago');
  assert.equal(humanizeSince(NOW - 130 * MIN, NOW), '2h 10m ago');
  assert.equal(humanizeSince(NOW - 120 * MIN, NOW), '2h ago');
  assert.equal(humanizeSince(NOW - (3 * 24 * 60 + 4 * 60) * MIN, NOW), '3d 4h ago');
  assert.equal(humanizeSince(NOW - 3 * 24 * 60 * MIN, NOW), '3d ago');
});

test('humanizeSince treats a future time as just now', () => {
  assert.equal(humanizeSince(NOW + 10 * MIN, NOW), 'just now');
});
