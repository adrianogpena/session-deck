import { test } from 'node:test';
import assert from 'node:assert/strict';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

// Before anything reads it: keep off the real ~/.session-deck.
process.env.SESSION_DECK_HOME = fs.mkdtempSync(path.join(os.tmpdir(), 'sd-trash-home-'));

import { appendClaudeRenameRecords, findSessionTitle } from '../discovery/claudeStorage';
import { listTrash, purgeTrash, restoreSession, trashClaudeSession, TRASH_TTL_MS } from '../store/trash';

/** A fake `~/.claude/projects/<dir>/<id>.jsonl`, with Claude's companion folder when asked. */
function fakeTranscript(id: string, withCompanion = false): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'sd-trash-projects-'));
  const file = path.join(dir, `${id}.jsonl`);
  fs.writeFileSync(file, '{"type":"user"}\n');
  if (withCompanion) {
    fs.mkdirSync(path.join(dir, id));
    fs.writeFileSync(path.join(dir, id, 'tool-result.txt'), 'x');
  }
  return file;
}

test('trashClaudeSession moves the transcript and its companion folder out, and restoreSession puts both back', () => {
  const file = fakeTranscript('t1', true);
  trashClaudeSession(file, 't1', 'My session');
  assert.equal(fs.existsSync(file), false);
  assert.equal(fs.existsSync(file.replace(/\.jsonl$/, '')), false);
  assert.deepEqual(listTrash().map((e) => [e.sessionId, e.title]), [['t1', 'My session']]);

  restoreSession('t1');
  assert.equal(fs.readFileSync(file, 'utf8'), '{"type":"user"}\n');
  assert.equal(fs.existsSync(path.join(file.replace(/\.jsonl$/, ''), 'tool-result.txt')), true);
  assert.deepEqual(listTrash(), []);
});

test('restoreSession refuses to overwrite a transcript that exists again', () => {
  const file = fakeTranscript('t2');
  trashClaudeSession(file, 't2', 'x');
  fs.writeFileSync(file, 'new');
  assert.throws(() => restoreSession('t2'), /already exists/);
  assert.equal(fs.readFileSync(file, 'utf8'), 'new');
  assert.equal(listTrash().some((e) => e.sessionId === 't2'), true);
});

test('purgeTrash removes only entries older than the TTL', () => {
  const file = fakeTranscript('t3');
  const entry = trashClaudeSession(file, 't3', 'old');
  purgeTrash(entry.trashedAt + TRASH_TTL_MS - 1);
  assert.equal(listTrash().some((e) => e.sessionId === 't3'), true); // not old enough yet
  purgeTrash(entry.trashedAt + TRASH_TTL_MS + 1);
  assert.deepEqual(listTrash(), []);
  assert.equal(fs.readdirSync(path.join(process.env.SESSION_DECK_HOME!, 'trash')).filter((f) => f.endsWith('.jsonl')).length, 0);
});

test('appendClaudeRenameRecords writes what /rename writes, never glued to a partial last line', () => {
  const file = fakeTranscript('r1');
  fs.appendFileSync(file, '{"type":"ai-title","aiTitle":"Old"}'); // no trailing newline
  appendClaudeRenameRecords(file, 'r1', 'New name');
  const lines = fs.readFileSync(file, 'utf8').split('\n');
  assert.equal(findSessionTitle(lines), 'New name');
  assert.ok(lines.some((l) => l.includes('"type":"agent-name"') && l.includes('New name')));
  assert.equal(JSON.parse(lines[1]).aiTitle, 'Old');
});
