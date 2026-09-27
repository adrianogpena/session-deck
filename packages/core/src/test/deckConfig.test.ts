import { test } from 'node:test';
import assert from 'node:assert/strict';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { parseDeckConfig, readDeckConfig, writeDeckConfig } from '../store/deckConfig';

test('parseDeckConfig returns every default when the file is empty', () => {
  const config = parseDeckConfig('{}');
  assert.equal(config.ui.maxSessionsListed, 30);
  assert.equal(config.ui.notifications, true);
  assert.deepEqual(config.ui.notifyStatuses, ['waiting', 'done', 'error']);
  assert.deepEqual(config.tools.claude, {});
  assert.deepEqual(config.tools.copilot, {});
  assert.equal(config.trash.retentionDays, 30);
});

test('parseDeckConfig accepts a valid ui.notifyStatuses and rejects an unknown status wholesale', () => {
  assert.deepEqual(parseDeckConfig(JSON.stringify({ ui: { notifyStatuses: ['waiting'] } })).ui.notifyStatuses, ['waiting']);
  assert.deepEqual(parseDeckConfig(JSON.stringify({ ui: { notifyStatuses: ['waiting', 'bogus'] } })).ui.notifyStatuses, [
    'waiting',
    'done',
    'error',
  ]);
});

test('parseDeckConfig accepts tools.*.enabled and trash.retentionDays', () => {
  const config = parseDeckConfig(JSON.stringify({ tools: { copilot: { enabled: false } }, trash: { retentionDays: 7 } }));
  assert.equal(config.tools.copilot.enabled, false);
  assert.equal(config.tools.claude.enabled, undefined);
  assert.equal(config.trash.retentionDays, 7);
});

test('parseDeckConfig falls back to the default retentionDays for a non-positive value', () => {
  assert.equal(parseDeckConfig(JSON.stringify({ trash: { retentionDays: 0 } })).trash.retentionDays, 30);
});

test('parseDeckConfig falls back to defaults for malformed JSON', () => {
  const config = parseDeckConfig('{ not json');
  assert.equal(config.ui.maxSessionsListed, 30);
  assert.equal(config.ui.notifications, true);
});

test('parseDeckConfig keeps only valid fields, dropping the rest to their defaults', () => {
  const config = parseDeckConfig(
    JSON.stringify({
      ui: { maxSessionsListed: 0, notifications: false },
      tools: { claude: { command: '  claude-nightly  ', args: ['--model', 'opus', 42] } },
    })
  );
  // maxSessionsListed: 0 is not a positive integer, so it falls back to the default.
  assert.equal(config.ui.maxSessionsListed, 30);
  assert.equal(config.ui.notifications, false);
  assert.equal(config.tools.claude.command, 'claude-nightly');
  // A mixed array (a number alongside strings) is rejected wholesale, not partially kept.
  assert.equal(config.tools.claude.args, undefined);
  assert.deepEqual(config.tools.copilot, {});
});

test('parseDeckConfig accepts a valid tools.*.args array', () => {
  const config = parseDeckConfig(JSON.stringify({ tools: { copilot: { args: ['--allow-all'] } } }));
  assert.deepEqual(config.tools.copilot.args, ['--allow-all']);
});

test('readDeckConfig returns the defaults when the file does not exist', () => {
  const config = readDeckConfig(path.join(os.tmpdir(), `session-deck-config-test-missing-${Date.now()}.json`));
  assert.equal(config.ui.maxSessionsListed, 30);
});

test('readDeckConfig reads and parses a real file', () => {
  const file = path.join(os.tmpdir(), `session-deck-config-test-${Date.now()}.json`);
  fs.writeFileSync(file, JSON.stringify({ ui: { maxSessionsListed: 50 } }));
  try {
    const config = readDeckConfig(file);
    assert.equal(config.ui.maxSessionsListed, 50);
  } finally {
    fs.unlinkSync(file);
  }
});

test('writeDeckConfig round-trips through readDeckConfig, creating the directory if needed', () => {
  const dir = path.join(os.tmpdir(), `session-deck-config-test-dir-${Date.now()}`);
  const file = path.join(dir, 'config.json');
  const config = readDeckConfig(file);
  config.ui.maxSessionsListed = 75;
  config.ui.notifications = false;
  config.tools.claude.command = 'claude-nightly';
  try {
    writeDeckConfig(config, file);
    const reread = readDeckConfig(file);
    assert.deepEqual(reread, config);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});
