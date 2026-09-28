import { test } from 'node:test';
import assert from 'node:assert/strict';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { DeckStore, applyHiddenProjectPatch, applySessionPatch, applyUiPatch, parseDeckState } from '../store/deckStore';

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

test('parseDeckState keeps valid UI prefs and drops invalid ones', () => {
  const state = parseDeckState(JSON.stringify({ version: 1, sessions: {}, ui: { theme: 'light', sidebarPct: 5 } }));
  assert.deepEqual(state?.ui, { theme: 'light' });
  assert.equal(parseDeckState(JSON.stringify({ version: 1, sessions: {}, ui: { theme: 'neon' } }))?.ui, undefined);
});

test('applyUiPatch merges, clears with undefined, and drops an empty ui section', () => {
  let state = applyUiPatch({ version: 1, sessions: {} }, { theme: 'dark', sidebarPct: 40 });
  assert.deepEqual(state.ui, { theme: 'dark', sidebarPct: 40 });
  state = applyUiPatch(state, { sidebarPct: undefined });
  assert.deepEqual(state.ui, { theme: 'dark' });
  state = applyUiPatch(state, { theme: undefined });
  assert.equal('ui' in state, false);
});

test('DeckStore keeps UI prefs when a session is updated, and sessions when UI prefs are', async () => {
  const file = tempStorePath();
  const store = new DeckStore(file);
  await store.updateUi({ theme: 'light' });
  await store.updateSession('a', { name: 'A' });
  await store.updateUi({ sidebarPct: 30 });
  assert.deepEqual(new DeckStore(file).read(), { version: 1, sessions: { a: { name: 'A' } }, ui: { theme: 'light', sidebarPct: 30 } });
});

test('applyHiddenProjectPatch adds and removes keys, dropping the field once empty', () => {
  let state = applyHiddenProjectPatch({ version: 1, sessions: {} }, '/a', true);
  assert.deepEqual(state.hiddenProjects, ['/a']);
  state = applyHiddenProjectPatch(state, '/b', true);
  assert.deepEqual(new Set(state.hiddenProjects), new Set(['/a', '/b']));
  state = applyHiddenProjectPatch(state, '/a', false);
  assert.deepEqual(state.hiddenProjects, ['/b']);
  state = applyHiddenProjectPatch(state, '/b', false);
  assert.equal('hiddenProjects' in state, false);
});

test('parseDeckState keeps unique, valid hidden project keys and drops an empty list', () => {
  const state = parseDeckState(JSON.stringify({ version: 1, sessions: {}, hiddenProjects: ['/a', '/a', 5, '/b'] }));
  assert.deepEqual(new Set(state?.hiddenProjects), new Set(['/a', '/b']));
  assert.equal(parseDeckState(JSON.stringify({ version: 1, sessions: {}, hiddenProjects: [] }))?.hiddenProjects, undefined);
});

test('DeckStore.setProjectHidden persists across instances and a project can be un-hidden', async () => {
  const file = tempStorePath();
  const store = new DeckStore(file);
  await store.setProjectHidden('/proj', true);
  assert.deepEqual(new DeckStore(file).getHiddenProjects(), ['/proj']);
  await store.setProjectHidden('/proj', false);
  assert.deepEqual(new DeckStore(file).getHiddenProjects(), []);
});
