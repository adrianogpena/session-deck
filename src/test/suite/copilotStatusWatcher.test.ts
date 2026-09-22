import { test } from 'node:test';
import assert from 'node:assert/strict';
import { classifyCopilotEvent, splitLines } from '../../status/copilotStatusWatcher';

test('classifyCopilotEvent maps a turn start to running', () => {
  assert.deepEqual(classifyCopilotEvent('assistant.turn_start'), { kind: 'status', status: 'running' });
});

test('classifyCopilotEvent maps a resolved permission/elicitation/input request to running too', () => {
  assert.deepEqual(classifyCopilotEvent('permission.completed'), { kind: 'status', status: 'running' });
  assert.deepEqual(classifyCopilotEvent('elicitation.completed'), { kind: 'status', status: 'running' });
  assert.deepEqual(classifyCopilotEvent('user_input.completed'), { kind: 'status', status: 'running' });
});

test('classifyCopilotEvent maps a pending permission/elicitation/input request to waiting', () => {
  assert.deepEqual(classifyCopilotEvent('permission.requested'), { kind: 'status', status: 'waiting' });
  assert.deepEqual(classifyCopilotEvent('elicitation.requested'), { kind: 'status', status: 'waiting' });
  assert.deepEqual(classifyCopilotEvent('user_input.requested'), { kind: 'status', status: 'waiting' });
});

test('classifyCopilotEvent maps a turn end to done', () => {
  assert.deepEqual(classifyCopilotEvent('assistant.turn_end'), { kind: 'status', status: 'done' });
});

test('classifyCopilotEvent maps a session error to error', () => {
  assert.deepEqual(classifyCopilotEvent('session.error'), { kind: 'status', status: 'error' });
});

test('classifyCopilotEvent maps session shutdown to clear', () => {
  assert.deepEqual(classifyCopilotEvent('session.shutdown'), { kind: 'clear' });
});

test('classifyCopilotEvent ignores an event type it does not recognize', () => {
  assert.equal(classifyCopilotEvent('session.model_change'), undefined);
  assert.equal(classifyCopilotEvent('assistant.message_delta'), undefined);
});

test('splitLines returns complete lines and carries a trailing partial line forward', () => {
  const result = splitLines('', '{"a":1}\n{"a":2}\n{"a":3');
  assert.deepEqual(result.lines, ['{"a":1}', '{"a":2}']);
  assert.equal(result.carry, '{"a":3');
});

test('splitLines completes a carried partial line once the rest arrives', () => {
  const first = splitLines('', '{"a":1}\n{"partial":');
  const second = splitLines(first.carry, 'true}\n');
  assert.deepEqual(second.lines, ['{"partial":true}']);
  assert.equal(second.carry, '');
});

test('splitLines handles a chunk with no newline at all as a pure carry', () => {
  const result = splitLines('abc', 'def');
  assert.deepEqual(result.lines, []);
  assert.equal(result.carry, 'abcdef');
});
