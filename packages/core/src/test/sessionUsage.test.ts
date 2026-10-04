import { test } from 'node:test';
import assert from 'node:assert/strict';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

// Before anything reads them: keep off the real ~/.claude/session-deck-status and ~/.claude.json.
process.env.SESSION_DECK_STATUS_DIR = fs.mkdtempSync(path.join(os.tmpdir(), 'sd-usage-dir-'));
const claudeConfigPath = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'sd-usage-cfg-')), 'claude.json');
process.env.SESSION_DECK_CLAUDE_CONFIG_PATH = claudeConfigPath;

import { recordSevenDayUsageSample, readSevenDayDailySpend } from '../status/sessionUsage';
import { getSessionStatusDir } from '../status/sessionStatus';

function setAccount(email: string | undefined): void {
  if (email === undefined) {
    try {
      fs.unlinkSync(claudeConfigPath);
    } catch {
      // already absent
    }
    return;
  }
  fs.writeFileSync(claudeConfigPath, JSON.stringify({ oauthAccount: { emailAddress: email } }), 'utf8');
}

const MON = new Date(2026, 0, 5, 9, 0, 0);
const TUE = new Date(2026, 0, 6, 9, 0, 0);

test('recordSevenDayUsageSample persists a day\'s reading so readSevenDayDailySpend survives a restart', () => {
  setAccount(undefined);
  recordSevenDayUsageSample(12, MON);
  recordSevenDayUsageSample(20, TUE);
  assert.deepEqual(readSevenDayDailySpend(TUE), [
    { label: 'Mo', percent: 12 },
    { label: 'Tu', percent: 8 },
  ]);
});

test('recordSevenDayUsageSample is a no-op write when the day\'s value is unchanged', () => {
  setAccount(undefined);
  const historyPath = path.join(getSessionStatusDir(), 'seven-day-history.json');
  recordSevenDayUsageSample(30, MON);
  const before = fs.statSync(historyPath).mtimeMs;
  recordSevenDayUsageSample(30, new Date(MON.getTime() + 60_000)); // same day, same value, later timestamp
  assert.equal(fs.statSync(historyPath).mtimeMs, before);
});

test('recordSevenDayUsageSample prunes readings far outside the retention window', () => {
  setAccount(undefined);
  const stale = new Date(2025, 0, 1);
  recordSevenDayUsageSample(5, stale);
  recordSevenDayUsageSample(40, MON);
  const historyPath = path.join(getSessionStatusDir(), 'seven-day-history.json');
  const history = JSON.parse(fs.readFileSync(historyPath, 'utf8'));
  assert.equal('2025-01-01' in history['unknown'], false);
});

test('seven-day history is scoped per logged-in account, not blended on a /login switch', () => {
  setAccount('personal@example.com');
  recordSevenDayUsageSample(15, MON);
  assert.deepEqual(readSevenDayDailySpend(MON), [{ label: 'Mo', percent: 15 }]);

  setAccount('work@example.com');
  recordSevenDayUsageSample(60, MON);
  assert.deepEqual(readSevenDayDailySpend(MON), [{ label: 'Mo', percent: 60 }]);

  setAccount('personal@example.com');
  assert.deepEqual(readSevenDayDailySpend(MON), [{ label: 'Mo', percent: 15 }]);
});
