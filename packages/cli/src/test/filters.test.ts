import { test } from 'node:test';
import assert from 'node:assert/strict';
import { matchesStatusFilter, nextTimeFilter, withinTimeFilter } from '../filters';

const HOUR = 60 * 60 * 1000;

test('nextTimeFilter cycles all → today → 3d → 7d → all', () => {
  assert.equal(nextTimeFilter('all'), 'today');
  assert.equal(nextTimeFilter('today'), '3d');
  assert.equal(nextTimeFilter('3d'), '7d');
  assert.equal(nextTimeFilter('7d'), 'all');
});

test('withinTimeFilter uses local midnight for today and rolling windows otherwise', () => {
  const now = new Date(2026, 8, 26, 10, 0, 0).getTime();
  assert.equal(withinTimeFilter(now - 9 * HOUR, 'today', now), true); // 01:00 today
  assert.equal(withinTimeFilter(now - 11 * HOUR, 'today', now), false); // 23:00 yesterday
  assert.equal(withinTimeFilter(now - 70 * HOUR, '3d', now), true);
  assert.equal(withinTimeFilter(now - 80 * HOUR, '3d', now), false);
  assert.equal(withinTimeFilter(now - 160 * HOUR, '7d', now), true);
  assert.equal(withinTimeFilter(0, 'all', now), true);
});

test('matchesStatusFilter treats an empty filter as everything', () => {
  assert.equal(matchesStatusFilter('stopped', new Set()), true);
  assert.equal(matchesStatusFilter('stopped', new Set(['running', 'waiting'])), false);
  assert.equal(matchesStatusFilter('waiting', new Set(['running', 'waiting'])), true);
});
