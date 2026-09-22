import { test } from 'node:test';
import assert from 'node:assert/strict';
import { buildClaudeResumeCommand, buildClaudeNewSessionCommand } from '../../terminal/claudeCommand';

test('buildClaudeResumeCommand resumes a plain session by id', () => {
  assert.equal(
    buildClaudeResumeCommand('550e8400-e29b-41d4-a716-446655440000', false),
    'claude --resume 550e8400-e29b-41d4-a716-446655440000'
  );
});

test('buildClaudeResumeCommand adds --dangerously-skip-permissions when asked', () => {
  assert.equal(
    buildClaudeResumeCommand('550e8400-e29b-41d4-a716-446655440000', true),
    'claude --dangerously-skip-permissions --resume 550e8400-e29b-41d4-a716-446655440000'
  );
});

test('buildClaudeResumeCommand rejects a session id containing shell metacharacters', () => {
  assert.throws(() => buildClaudeResumeCommand('abc; rm -rf ~', false), /Refusing to build a terminal command/);
});

test('buildClaudeNewSessionCommand', () => {
  assert.equal(buildClaudeNewSessionCommand(false), 'claude');
  assert.equal(buildClaudeNewSessionCommand(true), 'claude --dangerously-skip-permissions');
});
