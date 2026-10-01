import { test } from 'node:test';
import assert from 'node:assert/strict';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

process.env.SESSION_DECK_STATUS_DIR = fs.mkdtempSync(path.join(os.tmpdir(), 'sd-alert-dir-'));

import { ALERT_LOG_CAP, appendAlert, readAlerts } from '../status/alertLog';

test('appendAlert is deduped by session id + at, and readAlerts returns newest first', () => {
  appendAlert({ sessionId: 'a1', status: 'waiting', label: 'one', at: 100 });
  appendAlert({ sessionId: 'a1', status: 'waiting', label: 'one', at: 100 }); // same transition, logged by the other front end
  appendAlert({ sessionId: 'a1', status: 'done', label: 'one', at: 200 });

  const entries = readAlerts();
  assert.equal(entries.length, 2);
  assert.equal(entries[0].status, 'done'); // newest first
  assert.equal(entries[1].status, 'waiting');
});

test('appendAlert refuses an unsafe session id', () => {
  const before = readAlerts().length;
  appendAlert({ sessionId: '../escape', status: 'error', label: 'nope', at: 999 });
  assert.equal(readAlerts().length, before);
});

test('readAlerts caps at ALERT_LOG_CAP, dropping the oldest first', () => {
  process.env.SESSION_DECK_STATUS_DIR = fs.mkdtempSync(path.join(os.tmpdir(), 'sd-alert-dir-cap-'));
  for (let i = 0; i < ALERT_LOG_CAP + 10; i++) {
    appendAlert({ sessionId: 'cap', status: 'waiting', label: `turn ${i}`, at: i });
  }
  const entries = readAlerts(ALERT_LOG_CAP + 50);
  assert.equal(entries.length, ALERT_LOG_CAP);
  assert.equal(entries[0].at, ALERT_LOG_CAP + 9); // newest kept
  assert.equal(entries[entries.length - 1].at, 10); // the oldest 10 were pruned away
});
