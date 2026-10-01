import { test } from 'node:test';
import assert from 'node:assert/strict';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

// Before anything reads them: keep off the real ~/.claude/session-deck-status.
process.env.SESSION_DECK_STATUS_DIR = fs.mkdtempSync(path.join(os.tmpdir(), 'sd-usage-dir-'));

import { recordSevenDayUsageSample, readSevenDayDailySpend } from '../status/sessionUsage';
import { getSessionStatusDir } from '../status/sessionStatus';

const MON = new Date(2026, 0, 5, 9, 0, 0);
const TUE = new Date(2026, 0, 6, 9, 0, 0);

test('recordSevenDayUsageSample persists a day\'s reading so readSevenDayDailySpend survives a restart', () => {
  recordSevenDayUsageSample(12, MON);
  recordSevenDayUsageSample(20, TUE);
  assert.deepEqual(readSevenDayDailySpend(TUE), [
    { label: 'Mo', percent: 12 },
    { label: 'Tu', percent: 8 },
  ]);
});

test('recordSevenDayUsageSample is a no-op write when the day\'s value is unchanged', () => {
  const historyPath = path.join(getSessionStatusDir(), 'seven-day-history.json');
  recordSevenDayUsageSample(30, MON);
  const before = fs.statSync(historyPath).mtimeMs;
  recordSevenDayUsageSample(30, new Date(MON.getTime() + 60_000)); // same day, same value, later timestamp
  assert.equal(fs.statSync(historyPath).mtimeMs, before);
});

test('recordSevenDayUsageSample prunes readings far outside the retention window', () => {
  const stale = new Date(2025, 0, 1);
  recordSevenDayUsageSample(5, stale);
  recordSevenDayUsageSample(40, MON);
  const historyPath = path.join(getSessionStatusDir(), 'seven-day-history.json');
  const history = JSON.parse(fs.readFileSync(historyPath, 'utf8'));
  assert.equal('2025-01-01' in history, false);
});
