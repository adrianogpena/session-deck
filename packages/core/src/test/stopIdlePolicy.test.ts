import { test } from 'node:test';
import assert from 'node:assert/strict';
import { selectSessionsToStop } from '../stopIdlePolicy';

test('selectSessionsToStop stops nothing under the threshold', () => {
  const now = new Date('2024-01-01T01:00:00Z');
  const sessions = [{ sessionId: 'a', lastActivity: new Date('2024-01-01T00:45:00Z') }];
  assert.deepEqual(selectSessionsToStop(sessions, 30, now), []);
});

test('selectSessionsToStop stops sessions idle for at least the threshold', () => {
  const now = new Date('2024-01-01T01:00:00Z');
  const sessions = [
    { sessionId: 'stale', lastActivity: new Date('2024-01-01T00:30:00Z') },
    { sessionId: 'fresh', lastActivity: new Date('2024-01-01T00:50:00Z') },
  ];
  assert.deepEqual(selectSessionsToStop(sessions, 30, now), ['stale']);
});

test('selectSessionsToStop treats an idle time exactly at the threshold as due', () => {
  const now = new Date('2024-01-01T01:00:00Z');
  const sessions = [{ sessionId: 'exact', lastActivity: new Date('2024-01-01T00:30:00Z') }];
  assert.deepEqual(selectSessionsToStop(sessions, 30, now), ['exact']);
});
