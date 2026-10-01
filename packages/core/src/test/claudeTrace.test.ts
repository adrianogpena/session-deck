import { test } from 'node:test';
import assert from 'node:assert/strict';
import { buildTraceFromLines } from '../discovery/claudeTrace';

function userLine(content: unknown): string {
  return JSON.stringify({ type: 'user', message: { role: 'user', content } });
}

function assistantLine(content: unknown): string {
  return JSON.stringify({ type: 'assistant', message: { role: 'assistant', content } });
}

test('buildTraceFromLines reads a plain user prompt and assistant reply', () => {
  const steps = buildTraceFromLines([userLine('fix the bug'), assistantLine([{ type: 'text', text: 'Fixed it.' }])]);
  assert.deepEqual(steps, [
    { kind: 'user', label: 'fix the bug', detail: 'fix the bug' },
    { kind: 'assistant', label: 'Fixed it.', detail: 'Fixed it.' },
  ]);
});

test('buildTraceFromLines filters out a hidden slash-command echo', () => {
  const steps = buildTraceFromLines([userLine('<command-name>/clear</command-name>')]);
  assert.deepEqual(steps, []);
});

test('buildTraceFromLines turns a tool_use block into its own step, labeled with its primary argument', () => {
  const steps = buildTraceFromLines([
    assistantLine([{ type: 'tool_use', id: 'tool-1', name: 'Read', input: { file_path: 'src/app.ts' } }]),
  ]);
  assert.deepEqual(steps, [{ kind: 'tool', label: 'Read(src/app.ts)', detail: 'Read\n{\n  "file_path": "src/app.ts"\n}' }]);
});

test('buildTraceFromLines attaches a matching tool_result to its tool_use step', () => {
  const steps = buildTraceFromLines([
    assistantLine([{ type: 'tool_use', id: 'tool-1', name: 'Read', input: { file_path: 'src/app.ts' } }]),
    userLine([{ type: 'tool_result', tool_use_id: 'tool-1', content: 'file contents here' }]),
  ]);
  assert.equal(steps.length, 1);
  assert.equal(steps[0].kind, 'tool');
  assert.ok(steps[0].detail.endsWith('Result:\nfile contents here'));
  assert.equal(steps[0].isError, undefined);
});

test('buildTraceFromLines marks an error result and does not surface the tool_result as its own user step', () => {
  const steps = buildTraceFromLines([
    assistantLine([{ type: 'tool_use', id: 'tool-1', name: 'Bash', input: { command: 'false' } }]),
    userLine([{ type: 'tool_result', tool_use_id: 'tool-1', content: 'command failed', is_error: true }]),
  ]);
  assert.equal(steps.length, 1);
  assert.equal(steps[0].isError, true);
  assert.ok(steps[0].detail.endsWith('Error:\ncommand failed'));
});

test('buildTraceFromLines ignores an unmatched tool_result and non-user/assistant record types', () => {
  const steps = buildTraceFromLines([
    userLine([{ type: 'tool_result', tool_use_id: 'missing', content: 'orphaned' }]),
    JSON.stringify({ type: 'custom-title', customTitle: 'Renamed' }),
  ]);
  assert.deepEqual(steps, []);
});

test('buildTraceFromLines tolerates blank and malformed lines', () => {
  assert.deepEqual(buildTraceFromLines(['', '   ', '{not json']), []);
});
