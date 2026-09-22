import { test } from 'node:test';
import assert from 'node:assert/strict';
import * as path from 'path';
import { normalizeFsPath, isInside } from '../../discovery/pathUtils';

test('normalizeFsPath resolves relative segments away', () => {
  const a = normalizeFsPath(path.join('some', 'dir'));
  const b = normalizeFsPath(path.join('some', '.', 'dir'));
  assert.equal(a, b);
});

test('normalizeFsPath is case-insensitive on Windows, case-sensitive elsewhere', () => {
  const lower = normalizeFsPath('C:\\Users\\me\\app');
  const upper = normalizeFsPath('C:\\USERS\\ME\\APP');
  if (process.platform === 'win32') {
    assert.equal(lower, upper);
  } else {
    assert.notEqual(lower, upper);
  }
});

test('isInside accepts root itself and real children', () => {
  const root = path.resolve('/claude/projects');
  assert.equal(isInside(root, root), true);
  assert.equal(isInside(root, path.join(root, 'proj', 'session.jsonl')), true);
});

test('isInside rejects a path that escapes root via ..', () => {
  const root = path.resolve('/claude/projects');
  const escaped = path.join(root, '..', 'settings.json');
  assert.equal(isInside(root, escaped), false);
});

test('isInside rejects a sibling directory that merely shares a name prefix', () => {
  const root = path.resolve('/claude/projects');
  const sibling = path.resolve('/claude/projects-evil/x');
  assert.equal(isInside(root, sibling), false);
});
