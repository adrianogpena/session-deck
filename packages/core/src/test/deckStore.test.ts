import { test } from 'node:test';
import assert from 'node:assert/strict';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { DeckStore, applySessionPatch, parseDeckState } from '../store/deckStore';

function tempStorePath(): string {
  return path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'session-deck-store-')), 'state.json');
}

test('parseDeckState rejects non-JSON and keeps only valid session fields', () => {
  assert.equal(parseDeckState('{not json'), undefined);
  const state = parseDeckState(
    JSON.stringify({ version: 1, sessions: { a: { name: 'A', archived: true }, b: { name: '  ', archived: 'yes' }, c: 5 } })
  );
  assert.deepEqual(state, { version: 1, sessions: { a: { name: 'A', archived: true } } });
});

test('applySessionPatch sets and clears fields, removing entries left empty', () => {
  let state = applySessionPatch({ version: 1, sessions: {} }, 's', { name: 'N', archived: true });
  assert.deepEqual(state.sessions.s, { name: 'N', archived: true });
  state = applySessionPatch(state, 's', { archived: false });
  assert.deepEqual(state.sessions.s, { name: 'N' });
  state = applySessionPatch(state, 's', { name: undefined });
  assert.equal('s' in state.sessions, false);
});

test('DeckStore reads an empty state when the file does not exist', () => {
  assert.deepEqual(new DeckStore(tempStorePath()).read(), { version: 1, sessions: {} });
});

test('DeckStore merges a write onto changes another front end made meanwhile', async () => {
  const file = tempStorePath();
  const extension = new DeckStore(file);
  const terminal = new DeckStore(file);
  await extension.updateSession('a', { name: 'From extension' });
  assert.equal(terminal.getSession('a')?.name, 'From extension'); // warm the terminal's cache
  await extension.updateSession('b', { archived: true });
  await terminal.updateSession('a', { name: 'From terminal' });
  assert.deepEqual(new DeckStore(file).read().sessions, { a: { name: 'From terminal' }, b: { archived: true } });
});

test('DeckStore backs up an unparseable file instead of overwriting it', async () => {
  const file = tempStorePath();
  fs.writeFileSync(file, '{broken');
  await new DeckStore(file).updateSession('a', { archived: true });
  const backups = fs.readdirSync(path.dirname(file)).filter((f) => f.startsWith('state.json.corrupt-'));
  assert.equal(backups.length, 1);
  assert.equal(fs.readFileSync(path.join(path.dirname(file), backups[0]), 'utf8'), '{broken');
  assert.deepEqual(new DeckStore(file).read().sessions, { a: { archived: true } });
});
