import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  extractText,
  extractAssistantDisplayText,
  isDisplayableUserPrompt,
  decodeProjectPath,
  findSessionTitle,
} from '../discovery/claudeStorage';

const aiTitle = (title: string) => JSON.stringify({ type: 'ai-title', aiTitle: title, sessionId: 's' });
const customTitle = (title: string) => JSON.stringify({ type: 'custom-title', customTitle: title, sessionId: 's' });
const userLine = JSON.stringify({ type: 'user', message: { role: 'user', content: 'hi' } });

test('findSessionTitle returns the newest ai-title', () => {
  assert.equal(findSessionTitle([aiTitle('Old title'), userLine, aiTitle('New title'), userLine]), 'New title');
});

test('findSessionTitle prefers a /rename custom-title over a newer ai-title', () => {
  assert.equal(findSessionTitle([customTitle('My name'), aiTitle('AI title')]), 'My name');
});

test('findSessionTitle ignores agent-name records', () => {
  const agentName = JSON.stringify({ type: 'agent-name', agentName: 'agent', sessionId: 's' });
  assert.equal(findSessionTitle([aiTitle('AI title'), agentName]), 'AI title');
});

test('findSessionTitle skips a cut-off first line and blank titles', () => {
  const cut = aiTitle('Cut title').slice(10);
  assert.equal(findSessionTitle([cut, aiTitle('Real title'), aiTitle('   ')]), 'Real title');
});

test('findSessionTitle returns undefined when there is no title record', () => {
  assert.equal(findSessionTitle([userLine, '']), undefined);
});

test('extractText returns a plain string as-is', () => {
  assert.equal(extractText('hello'), 'hello');
});

test('extractText joins text/thinking blocks and ignores tool blocks', () => {
  const content = [
    { type: 'text', text: 'first' },
    { type: 'tool_use', input: {} },
    { type: 'thinking', thinking: 'second' },
  ];
  assert.equal(extractText(content), 'first\nsecond');
});

test('extractText handles a single content-block object', () => {
  assert.equal(extractText({ type: 'text', text: 'solo' }), 'solo');
});

test('extractText returns an empty string for null/undefined/unsupported input', () => {
  assert.equal(extractText(undefined), '');
  assert.equal(extractText(null), '');
  assert.equal(extractText(42), '');
});

test('extractAssistantDisplayText keeps only text blocks, in order', () => {
  const content = [
    { type: 'thinking', thinking: 'skip me' },
    { type: 'text', text: 'first' },
    { type: 'tool_use', input: {} },
    { type: 'text', text: 'second' },
  ];
  assert.equal(extractAssistantDisplayText(content), 'first\nsecond');
});

test('extractAssistantDisplayText returns an empty string for non-array content', () => {
  assert.equal(extractAssistantDisplayText('plain string'), '');
});

test('isDisplayableUserPrompt rejects blank and synthetic prompts', () => {
  assert.equal(isDisplayableUserPrompt(''), false);
  assert.equal(isDisplayableUserPrompt('   '), false);
  assert.equal(isDisplayableUserPrompt('<command-name>/clear</command-name>'), false);
  assert.equal(isDisplayableUserPrompt('agentId: abc123'), false);
});

test('isDisplayableUserPrompt accepts a real prompt', () => {
  assert.equal(isDisplayableUserPrompt('fix the bug in foo.ts'), true);
});

test('decodeProjectPath reverses the encoded-path scheme', () => {
  assert.equal(decodeProjectPath('C--Users-me-app'), 'C:\\Users\\me\\app');
});
