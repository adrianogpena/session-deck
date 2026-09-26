import { test } from 'node:test';
import assert from 'node:assert/strict';
import * as os from 'os';
import * as path from 'path';
import { normalizeFsPath, isInside, expandHome } from '../discovery/pathUtils';

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

test('expandHome expands a bare ~ to the home directory', () => {
  assert.equal(expandHome('~'), os.homedir());
});

test('expandHome expands a leading ~/ to a path under the home directory', () => {
  assert.equal(expandHome('~/foo/bar'), path.join(os.homedir(), 'foo/bar'));
});

test('expandHome expands a leading ~\\ to a path under the home directory', () => {
  assert.equal(expandHome('~\\foo\\bar'), path.join(os.homedir(), 'foo\\bar'));
});

test('expandHome leaves an absolute path untouched', () => {
  assert.equal(expandHome('C:/Source/my-project'), 'C:/Source/my-project');
});

test('expandHome does not expand ~user (no cross-platform resolution available)', () => {
  assert.equal(expandHome('~someuser/foo'), '~someuser/foo');
});
