import { test } from 'node:test';
import assert from 'node:assert/strict';
import { localDateKey, sevenDayDailySpend } from '../status/usageDisplay';

// A fixed Wednesday so the Monday-of-week math is predictable regardless of when tests run.
const WED = new Date(2026, 0, 7, 15, 0, 0); // 2026-01-07
const MON = new Date(2026, 0, 5, 9, 0, 0); // 2026-01-05
const SUN = new Date(2026, 0, 11, 20, 0, 0); // 2026-01-11, same week as MON

test('localDateKey formats local year-month-day, zero-padded', () => {
  assert.equal(localDateKey(new Date(2026, 0, 7)), '2026-01-07');
  assert.equal(localDateKey(new Date(2026, 10, 30)), '2026-11-30');
});

test('sevenDayDailySpend turns cumulative readings into per-day deltas, Monday baselined at 0', () => {
  const history = { '2026-01-05': 12, '2026-01-06': 20, '2026-01-07': 35 };
  assert.deepEqual(sevenDayDailySpend(history, WED), [
    { label: 'Mo', percent: 12 },
    { label: 'Tu', percent: 8 },
    { label: 'We', percent: 15 },
  ]);
});

test('sevenDayDailySpend stops at today and never looks past it', () => {
  const history = { '2026-01-05': 12, '2026-01-06': 20, '2026-01-07': 35, '2026-01-08': 50 };
  assert.deepEqual(
    sevenDayDailySpend(history, MON).map((d) => d.label),
    ['Mo']
  );
});

test('sevenDayDailySpend includes weekend days and leaves out days with no recorded reading', () => {
  const history = { '2026-01-05': 10, '2026-01-10': 40, '2026-01-11': 55 }; // Tue-Fri unrecorded
  assert.deepEqual(sevenDayDailySpend(history, SUN), [
    { label: 'Mo', percent: 10 },
    { label: 'Sa', percent: 30 }, // the Tue-Fri gap folds into Saturday's delta
    { label: 'Su', percent: 15 },
  ]);
});

test('sevenDayDailySpend clamps a drop in the rolling total to 0 rather than a negative spend', () => {
  const history = { '2026-01-05': 40, '2026-01-06': 25 }; // old usage aged out of the rolling window
  assert.deepEqual(sevenDayDailySpend(history, new Date(2026, 0, 6)), [
    { label: 'Mo', percent: 40 },
    { label: 'Tu', percent: 0 },
  ]);
});

test('sevenDayDailySpend is empty with no history at all', () => {
  assert.deepEqual(sevenDayDailySpend({}, WED), []);
});
