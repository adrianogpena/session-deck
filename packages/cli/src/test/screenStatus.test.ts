import { test } from 'node:test';
import assert from 'node:assert/strict';
import { detectScreenError, SCREEN_ERROR_LINES } from '../screenStatus';

test('detectScreenError recognizes sign-in failures and API errors', () => {
  assert.equal(detectScreenError(['❯ hi', '  ⎿  API Error: 401 {"type":"error","error":{"type":"authentication_error"}}']), 'sign-in failed · run /login');
  assert.equal(detectScreenError(['Invalid API key · Please run /login']), 'sign-in failed · run /login');
  assert.equal(detectScreenError(['Not logged in · Please run /login']), 'signed out · run /login');
  assert.equal(detectScreenError(['API Error: 529 {"type":"overloaded_error"}']), 'API error · retrying or rate-limited');
});

test('detectScreenError ignores normal output and errors that scrolled out of the last lines', () => {
  assert.equal(detectScreenError(['● Done. The tests pass.', '❯']), undefined);
  const scrolled = ['API Error: 401', ...Array.from({ length: SCREEN_ERROR_LINES }, () => 'later output')];
  assert.equal(detectScreenError(scrolled), undefined);
});
