import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseGitStatusPorcelain } from '../discovery/gitStatus';

test('parseGitStatusPorcelain reads the branch name', () => {
  const status = parseGitStatusPorcelain('# branch.oid abc123\n# branch.head main\n');
  assert.equal(status.branch, 'main');
  assert.equal(status.ahead, 0);
  assert.equal(status.behind, 0);
  assert.equal(status.dirty, 0);
});

test('parseGitStatusPorcelain reads a detached HEAD as no branch', () => {
  assert.equal(parseGitStatusPorcelain('# branch.head (detached)\n').branch, undefined);
});

test('parseGitStatusPorcelain reads ahead/behind counts', () => {
  const status = parseGitStatusPorcelain('# branch.head main\n# branch.ab +2 -1\n');
  assert.equal(status.ahead, 2);
  assert.equal(status.behind, 1);
});

test('parseGitStatusPorcelain defaults ahead/behind to 0 with no upstream', () => {
  const status = parseGitStatusPorcelain('# branch.head main\n');
  assert.equal(status.ahead, 0);
  assert.equal(status.behind, 0);
});

test('parseGitStatusPorcelain counts changed and untracked entries as dirty', () => {
  const output = [
    '# branch.head main',
    '1 .M N... 100644 100644 100644 abc123 def456 src/app.ts',
    '2 R. N... 100644 100644 100644 abc123 def456 R100 src/new.ts\tsrc/old.ts',
    'u UU N... 100644 100644 100644 100644 a b c d src/conflict.ts',
    '? untracked.txt',
  ].join('\n');
  assert.equal(parseGitStatusPorcelain(output).dirty, 4);
});

test('parseGitStatusPorcelain ignores blank lines', () => {
  assert.equal(parseGitStatusPorcelain('# branch.head main\n\n\n').dirty, 0);
});

test('parseGitStatusPorcelain returns a clean status for empty input', () => {
  const status = parseGitStatusPorcelain('');
  assert.deepEqual(status, { branch: undefined, ahead: 0, behind: 0, dirty: 0 });
});
