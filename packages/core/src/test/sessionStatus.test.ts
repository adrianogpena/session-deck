import { test } from 'node:test';
import assert from 'node:assert/strict';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

// Before anything reads them: keep off the real ~/.session-deck and ~/.claude/session-deck-status.
process.env.SESSION_DECK_HOME = fs.mkdtempSync(path.join(os.tmpdir(), 'sd-status-home-'));
process.env.SESSION_DECK_STATUS_DIR = fs.mkdtempSync(path.join(os.tmpdir(), 'sd-status-dir-'));

import {
  acknowledgeSessionStatus,
  claimNotification,
  clearSessionStatus,
  isSessionStatusUnseen,
  markSessionUnseen,
  readEffectiveSessionStatus,
  readSessionStatus,
  writeSessionStatus,
} from '../status/sessionStatus';
import { DeckStore } from '../store/deckStore';

test('a "done" stays unseen until acknowledged, and the seen mark is kept in the shared store', async () => {
  writeSessionStatus('s1', 'done');
  assert.equal(isSessionStatusUnseen('s1'), true);
  assert.equal(readEffectiveSessionStatus('s1')?.status, 'done');
  await acknowledgeSessionStatus('s1');
  assert.equal(isSessionStatusUnseen('s1'), false);
  assert.equal(readEffectiveSessionStatus('s1'), undefined);
  // Another front end (its own store instance) sees the same seen mark.
  assert.equal(new DeckStore().getSession('s1')?.seenAt, readSessionStatus('s1')?.updatedAt);
});

test('a newer "done" after the seen one is unseen again', async () => {
  writeSessionStatus('s2', 'done');
  await acknowledgeSessionStatus('s2');
  await new Promise((r) => setTimeout(r, 5)); // a distinct updatedAt
  writeSessionStatus('s2', 'done');
  assert.equal(isSessionStatusUnseen('s2'), true);
});

test('"waiting" and "running" are never acknowledged away', async () => {
  writeSessionStatus('s3', 'waiting');
  await acknowledgeSessionStatus('s3');
  assert.equal(readEffectiveSessionStatus('s3')?.status, 'waiting');
  assert.equal(isSessionStatusUnseen('s3'), false);
});

test('markSessionUnseen un-sees a seen "done", creates one when there is no status, and leaves "waiting" alone', async () => {
  writeSessionStatus('s4', 'done');
  await acknowledgeSessionStatus('s4');
  await markSessionUnseen('s4');
  assert.equal(isSessionStatusUnseen('s4'), true);

  await markSessionUnseen('s5');
  assert.equal(readSessionStatus('s5')?.status, 'done');
  assert.equal(isSessionStatusUnseen('s5'), true);

  writeSessionStatus('s6', 'waiting');
  await markSessionUnseen('s6');
  assert.equal(readSessionStatus('s6')?.status, 'waiting');
});

test('claimNotification succeeds once per status, and clearing the status removes its marker', () => {
  writeSessionStatus('s7', 'waiting');
  const { updatedAt } = readSessionStatus('s7')!;
  assert.equal(claimNotification('s7', updatedAt), true);
  assert.equal(claimNotification('s7', updatedAt), false); // e.g. the other front end
  assert.equal(claimNotification('s7', updatedAt + 1), true); // a new event
  const markers = () => fs.readdirSync(process.env.SESSION_DECK_STATUS_DIR!).filter((f) => f.startsWith('s7.') && f.endsWith('.notified'));
  assert.equal(markers().length, 1); // the older marker was replaced
  clearSessionStatus('s7');
  assert.equal(markers().length, 0);
});
