import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseTranscriptLine } from '../status/claudeTranscriptTailer';

function userLine(text: string): string {
  return JSON.stringify({ type: 'user', message: { role: 'user', content: text } });
}

function assistantLine(blocks: Array<{ type: string; text?: string }>): string {
  return JSON.stringify({ type: 'assistant', message: { role: 'assistant', content: blocks } });
}

test('parseTranscriptLine reads a plain user prompt', () => {
  assert.deepEqual(parseTranscriptLine(userLine('fix the bug')), { role: 'user', text: 'fix the bug' });
});

test('parseTranscriptLine filters out a hidden slash-command echo', () => {
  assert.equal(parseTranscriptLine(userLine('<command-name>/clear</command-name>')), undefined);
});

test('parseTranscriptLine reads assistant display text, excluding thinking blocks', () => {
  const line = assistantLine([
    { type: 'thinking', text: 'internal reasoning' },
    { type: 'text', text: 'Here is the fix.' },
  ]);
  assert.deepEqual(parseTranscriptLine(line), { role: 'assistant', text: 'Here is the fix.' });
});

test('parseTranscriptLine ignores an assistant turn with only tool-use/thinking blocks', () => {
  const line = assistantLine([{ type: 'tool_use' }, { type: 'thinking', text: 'internal reasoning' }]);
  assert.equal(parseTranscriptLine(line), undefined);
});

test('parseTranscriptLine ignores non-user/assistant record types', () => {
  assert.equal(parseTranscriptLine(JSON.stringify({ type: 'custom-title', customTitle: 'Renamed' })), undefined);
});

test('parseTranscriptLine tolerates a blank or malformed line', () => {
  assert.equal(parseTranscriptLine(''), undefined);
  assert.equal(parseTranscriptLine('   '), undefined);
  assert.equal(parseTranscriptLine('{not json'), undefined);
});
