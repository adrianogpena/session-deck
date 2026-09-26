import { test } from 'node:test';
import assert from 'node:assert/strict';
import { buildCopilotNewSessionCommand, buildCopilotResumeCommand } from '../commands/copilotCommand';

test('buildCopilotResumeCommand resumes a plain session by id', () => {
  assert.equal(
    buildCopilotResumeCommand('da497aac-3654-4b17-b84b-af17ec9eeaf8', false),
    'copilot --resume=da497aac-3654-4b17-b84b-af17ec9eeaf8'
  );
});

test('buildCopilotResumeCommand adds --allow-all when asked', () => {
  assert.equal(
    buildCopilotResumeCommand('da497aac-3654-4b17-b84b-af17ec9eeaf8', true),
    'copilot --resume=da497aac-3654-4b17-b84b-af17ec9eeaf8 --allow-all'
  );
});

test('buildCopilotResumeCommand rejects a session id containing shell metacharacters', () => {
  assert.throws(() => buildCopilotResumeCommand('abc; rm -rf ~', false), /Refusing to build a terminal command/);
});

test('buildCopilotNewSessionCommand pre-assigns the given session id', () => {
  assert.equal(
    buildCopilotNewSessionCommand('da497aac-3654-4b17-b84b-af17ec9eeaf8', false),
    'copilot --session-id=da497aac-3654-4b17-b84b-af17ec9eeaf8'
  );
});

test('buildCopilotNewSessionCommand adds --allow-all when asked', () => {
  assert.equal(
    buildCopilotNewSessionCommand('da497aac-3654-4b17-b84b-af17ec9eeaf8', true),
    'copilot --session-id=da497aac-3654-4b17-b84b-af17ec9eeaf8 --allow-all'
  );
});

test('buildCopilotNewSessionCommand rejects a session id containing shell metacharacters', () => {
  assert.throws(() => buildCopilotNewSessionCommand('abc; rm -rf ~', false), /Refusing to build a terminal command/);
});
