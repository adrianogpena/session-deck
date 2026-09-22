import { test } from 'node:test';
import assert from 'node:assert/strict';
import { classifyCopilotEvent, splitLines } from '../../status/copilotStatusWatcher';

test('classifyCopilotEvent maps a turn start to running', () => {
  assert.deepEqual(classifyCopilotEvent({ type: 'assistant.turn_start' }), { kind: 'status', status: 'running' });
});

test('classifyCopilotEvent maps a resolved permission/elicitation/input request to running too', () => {
  assert.deepEqual(classifyCopilotEvent({ type: 'permission.completed' }), { kind: 'status', status: 'running' });
  assert.deepEqual(classifyCopilotEvent({ type: 'elicitation.completed' }), { kind: 'status', status: 'running' });
  assert.deepEqual(classifyCopilotEvent({ type: 'user_input.completed' }), { kind: 'status', status: 'running' });
});

test('classifyCopilotEvent maps a pending permission/elicitation/input request to waiting', () => {
  assert.deepEqual(classifyCopilotEvent({ type: 'permission.requested' }), { kind: 'status', status: 'waiting' });
  assert.deepEqual(classifyCopilotEvent({ type: 'elicitation.requested' }), { kind: 'status', status: 'waiting' });
  assert.deepEqual(classifyCopilotEvent({ type: 'user_input.requested' }), { kind: 'status', status: 'waiting' });
});

test('classifyCopilotEvent maps a turn end to done', () => {
  assert.deepEqual(classifyCopilotEvent({ type: 'assistant.turn_end' }), { kind: 'status', status: 'done' });
});

test('classifyCopilotEvent maps a session error to error', () => {
  assert.deepEqual(classifyCopilotEvent({ type: 'session.error' }), { kind: 'status', status: 'error' });
});

test('classifyCopilotEvent maps session shutdown to clear', () => {
  assert.deepEqual(classifyCopilotEvent({ type: 'session.shutdown' }), { kind: 'clear' });
});

test('classifyCopilotEvent ignores an event type it does not recognize', () => {
  assert.equal(classifyCopilotEvent({ type: 'session.model_change' }), undefined);
  assert.equal(classifyCopilotEvent({ type: 'assistant.message_delta' }), undefined);
});

test('classifyCopilotEvent maps the ask_user tool starting to waiting, but leaves any other tool call alone', () => {
  assert.deepEqual(classifyCopilotEvent({ type: 'tool.execution_start', data: { toolName: 'ask_user' } }), {
    kind: 'status',
    status: 'waiting',
  });
  assert.equal(classifyCopilotEvent({ type: 'tool.execution_start', data: { toolName: 'view' } }), undefined);
  assert.equal(classifyCopilotEvent({ type: 'tool.execution_start' }), undefined);
});

test('classifyCopilotEvent maps the ask_user tool completing back to running, but leaves any other tool call alone', () => {
  assert.deepEqual(classifyCopilotEvent({ type: 'tool.execution_complete', data: { toolName: 'ask_user' } }), {
    kind: 'status',
    status: 'running',
  });
  assert.equal(classifyCopilotEvent({ type: 'tool.execution_complete', data: { toolName: 'grep' } }), undefined);
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
