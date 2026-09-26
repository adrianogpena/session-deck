import { test } from 'node:test';
import assert from 'node:assert/strict';
import { classifyClaudeProcessStatus } from '../status/claudeProcessWatcher';

test('classifyClaudeProcessStatus maps busy and shell to running', () => {
  assert.equal(classifyClaudeProcessStatus('busy'), 'running');
  assert.equal(classifyClaudeProcessStatus('shell'), 'running');
});

test('classifyClaudeProcessStatus maps waiting to waiting', () => {
  assert.equal(classifyClaudeProcessStatus('waiting'), 'waiting');
});

test('classifyClaudeProcessStatus maps idle to done', () => {
  assert.equal(classifyClaudeProcessStatus('idle'), 'done');
});

test('classifyClaudeProcessStatus ignores a status it does not recognize', () => {
  assert.equal(classifyClaudeProcessStatus('gone'), undefined);
  assert.equal(classifyClaudeProcessStatus('something-future-versions-might-add'), undefined);
});
