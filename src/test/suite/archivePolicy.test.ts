import { test } from 'node:test';
import assert from 'node:assert/strict';
import { selectSessionsToArchive } from '../../tree/archivePolicy';

test('selectSessionsToArchive does nothing when nothing new was discovered', () => {
  const ids = ['a', 'b', 'c', 'd', 'e', 'f'];
  assert.deepEqual(selectSessionsToArchive(ids, 5, new Set()), []);
});

test('selectSessionsToArchive evicts everything past the cap when something new showed up', () => {
  const ids = ['new', 'e4', 'e3', 'e2', 'e1', 'old2', 'old1'];
  assert.deepEqual(selectSessionsToArchive(ids, 5, new Set(['new'])), ['old2', 'old1']);
});

test('selectSessionsToArchive evicts nothing if the active count is still within the cap', () => {
  const ids = ['new', 'e3', 'e2', 'e1'];
  assert.deepEqual(selectSessionsToArchive(ids, 5, new Set(['new'])), []);
});

test('selectSessionsToArchive evicts exactly one when a single new session pushes past the cap', () => {
  const ids = ['new', 'e4', 'e3', 'e2', 'e1', 'oldest'];
  assert.deepEqual(selectSessionsToArchive(ids, 5, new Set(['new'])), ['oldest']);
});
