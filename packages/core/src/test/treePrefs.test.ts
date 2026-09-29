import { test } from 'node:test';
import assert from 'node:assert/strict';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { DeckStore } from '../store/deckStore';
import {
  arrangeByManualOrder,
  arrangeProjects,
  createFolder,
  defaultTreePrefs,
  deleteFolder,
  folderNodeKey,
  freezeSessionOrder,
  moveFolder,
  moveProject,
  moveProjectToFolder,
  moveSession,
  parseTreePrefs,
  prependSession,
  renameSessionId,
  SessionCategory,
  setCollapsed,
  sortSessions,
  TreePrefs,
} from '../store/treePrefs';

function withFolders(...names: string[]): { tree: TreePrefs; ids: string[] } {
  let tree = defaultTreePrefs();
  const ids: string[] = [];
  for (const name of names) {
    const created = createFolder(tree, name);
    tree = created.tree;
    ids.push(created.folderId);
  }
  return { tree, ids };
}

test('parseTreePrefs falls back to defaults for invalid parts and keeps a project in one place only', () => {
  const tree = parseTreePrefs({
    folders: [
      { id: 'a', name: 'Work', projects: ['p1', 'p2'] },
      { id: 'b', name: '  ', projects: ['p3'] },
      { id: 'c', name: 'Home', projects: ['p2', 'p4'] },
    ],
    rootOrder: ['p1', 'p5', 7],
    sessionOrder: { p1: ['s2', 's1', 3], p2: [], p3: 'nope' },
    sort: 'sideways',
    view: 'active',
  });
  assert.deepEqual(tree?.folders, [
    { id: 'a', name: 'Work', projects: ['p1', 'p2'] },
    { id: 'c', name: 'Home', projects: ['p4'] },
  ]);
  assert.deepEqual(tree?.rootOrder, ['p5']);
  assert.deepEqual(tree?.sessionOrder, { p1: ['s2', 's1'] });
  assert.equal(tree?.sort, 'recent');
  assert.equal(tree?.view, 'active');
});

test('moveProjectToFolder moves between folders and back to the top level', () => {
  const { tree, ids } = withFolders('Work', 'Home');
  let t = moveProjectToFolder(tree, 'p1', ids[0]);
  t = moveProjectToFolder(t, 'p1', ids[1]);
  assert.deepEqual(t.folders.map((f) => f.projects), [[], ['p1']]);
  t = moveProjectToFolder(t, 'p1', null);
  assert.deepEqual(t.folders.map((f) => f.projects), [[], []]);
  assert.deepEqual(t.rootOrder, ['p1']);
});

test('deleteFolder returns its projects to the top of the top level and drops its collapsed flag', () => {
  const { tree, ids } = withFolders('Work');
  let t = moveProjectToFolder({ ...tree, rootOrder: ['p9'] }, 'p1', ids[0]);
  t = setCollapsed(t, folderNodeKey(ids[0]), true);
  t = deleteFolder(t, ids[0]);
  assert.deepEqual(t.folders, []);
  assert.deepEqual(t.rootOrder, ['p1', 'p9']);
  assert.deepEqual(t.collapsed, []);
});

test('moveFolder swaps with the neighbor and ignores moves past the ends', () => {
  const { tree, ids } = withFolders('A', 'B', 'C');
  assert.deepEqual(moveFolder(tree, ids[2], -1).folders.map((f) => f.name), ['A', 'C', 'B']);
  assert.equal(moveFolder(tree, ids[0], -1), tree);
});

test('moveProject swaps displayed neighbors and keeps projects the other front end shows in place', () => {
  // Stored order has a project (x) this front end doesn't show; b and c were never ordered.
  const tree = { ...defaultTreePrefs(), rootOrder: ['a', 'x'] };
  const t = moveProject(tree, 'c', -1, ['a', 'b', 'c']);
  assert.deepEqual(t.rootOrder, ['a', 'x', 'c', 'b']);
  assert.deepEqual(arrangeProjects(t, ['a', 'b', 'c']).root, ['a', 'c', 'b']);
  assert.equal(moveProject(tree, 'a', -1, ['a', 'b']), tree);
});

test('moveSession swaps displayed neighbors within one project and keeps other projects\' orders untouched', () => {
  const tree = { ...defaultTreePrefs(), sessionOrder: { p1: ['a', 'x'], p2: ['z'] } };
  const t = moveSession(tree, 'p1', 'c', -1, ['a', 'b', 'c']);
  assert.deepEqual(t.sessionOrder, { p1: ['a', 'x', 'c', 'b'], p2: ['z'] });
  assert.equal(moveSession(tree, 'p1', 'a', -1, ['a', 'b']), tree);
});

test('prependSession puts a new session first, moving it up from an earlier stored position if any', () => {
  const tree = { ...defaultTreePrefs(), sessionOrder: { p1: ['a', 'b'], p2: ['z'] } };
  const t = prependSession(tree, 'p1', 'c');
  assert.deepEqual(t.sessionOrder, { p1: ['c', 'a', 'b'], p2: ['z'] });
  // Already-first is a no-op in effect (still first), and other projects are untouched either way.
  const t2 = prependSession(t, 'p1', 'b');
  assert.deepEqual(t2.sessionOrder, { p1: ['b', 'c', 'a'], p2: ['z'] });
  // A project with nothing stored yet starts a fresh order with just this session.
  assert.deepEqual(prependSession(tree, 'p3', 'x').sessionOrder, { p1: ['a', 'b'], p2: ['z'], p3: ['x'] });
});

test('renameSessionId keeps the old id right after the new one, so an orphaned /clear transcript lands just below it, not at the back', () => {
  const tree = { ...defaultTreePrefs(), sessionOrder: { p1: ['a', 'b'], p2: ['z'] } };
  const t = renameSessionId(tree, 'p1', 'a', 'a2');
  assert.deepEqual(t.sessionOrder, { p1: ['a2', 'a', 'b'], p2: ['z'] });
  // Clearing again on the same terminal chains the previous orphan right after the newest id.
  const t2 = renameSessionId(t, 'p1', 'a2', 'a3');
  assert.deepEqual(t2.sessionOrder, { p1: ['a3', 'a2', 'a', 'b'], p2: ['z'] });
  // A no-op when the old id was never recorded (falls back to prependSession) or is unchanged.
  assert.equal(renameSessionId(tree, 'p1', 'x', 'y'), tree);
  assert.equal(renameSessionId(tree, 'p1', 'a', 'a'), tree);
});

test('freezeSessionOrder locks in the currently displayed order, once, leaving already-stored sessions untouched', () => {
  const tree = { ...defaultTreePrefs(), sessionOrder: { p1: ['a', 'b'] } };
  // p1: 'c' is new (gets appended); p2: nothing stored yet, so its whole displayed order is captured.
  const displayed = new Map([
    ['p1', ['a', 'b', 'c']],
    ['p2', ['y', 'x']],
  ]);
  const t = freezeSessionOrder(tree, displayed);
  assert.deepEqual(t.sessionOrder, { p1: ['a', 'b', 'c'], p2: ['y', 'x'] });
  // Nothing new to freeze the second time around: returns the same object (no-op, no spurious write).
  assert.equal(freezeSessionOrder(t, displayed), t);
});

test('arrangeByManualOrder keeps stored positions and appends unlisted ids in fallback order', () => {
  assert.deepEqual(arrangeByManualOrder(['a', 'b', 'c'], ['c', 'a']), ['c', 'a', 'b']);
  assert.deepEqual(arrangeByManualOrder(['a', 'b'], ['z', 'a']), ['a', 'b']); // 'z' isn't present, dropped
});

test('arrangeProjects places folder members, then ordered top-level projects, then the rest in given order', () => {
  const { tree, ids } = withFolders('Work', 'Empty');
  const t = moveProjectToFolder({ ...tree, rootOrder: ['z'] }, 'm', ids[0]);
  const arranged = arrangeProjects(t, ['q', 'm', 'z', 'r']);
  assert.deepEqual(
    arranged.folders.map((f) => [f.folder.name, f.projects]),
    [
      ['Work', ['m']],
      ['Empty', []],
    ]
  );
  assert.deepEqual(arranged.root, ['z', 'q', 'r']);
});

test('sortSessions puts pinned bands around the sorted rest; actionable ranks error, waiting, running, idle', () => {
  type S = { id: string; mtime: number; pin?: 'top' | 'bottom'; category: SessionCategory };
  const items: S[] = [
    { id: 'idle-new', mtime: 9, category: 'idle' },
    { id: 'running', mtime: 5, category: 'running' },
    { id: 'pin-bottom', mtime: 8, pin: 'bottom', category: 'waiting' },
    { id: 'waiting', mtime: 1, category: 'waiting' },
    { id: 'pin-top', mtime: 2, pin: 'top', category: 'stopped' },
    { id: 'error', mtime: 3, category: 'error' },
  ];
  const order = (sort: 'recent' | 'actionable') =>
    sortSessions(items, sort, (s) => s.pin, (s) => s.mtime, (s) => s.category).map((s) => s.id);
  assert.deepEqual(order('recent'), ['pin-top', 'idle-new', 'running', 'error', 'waiting', 'pin-bottom']);
  assert.deepEqual(order('actionable'), ['pin-top', 'error', 'waiting', 'running', 'idle-new', 'pin-bottom']);
});

test('DeckStore.updateTree applies the change to the tree on disk and keeps sessions and pins', async () => {
  const file = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'session-deck-tree-')), 'state.json');
  const extension = new DeckStore(file);
  const terminal = new DeckStore(file);
  await extension.updateSession('s1', { pin: 'top' });
  assert.deepEqual(terminal.getTree(), defaultTreePrefs()); // warm the terminal's cache
  let folderId = '';
  await extension.updateTree((t) => {
    const created = createFolder(t, 'Work');
    folderId = created.folderId;
    return created.tree;
  });
  await terminal.updateTree((t) => moveProjectToFolder(t, 'p1', folderId));
  const state = new DeckStore(file).read();
  assert.deepEqual(state.tree?.folders, [{ id: folderId, name: 'Work', projects: ['p1'] }]);
  assert.deepEqual(state.sessions, { s1: { pin: 'top' } });
  await terminal.updateTree((t) => deleteFolder(t, folderId));
  await terminal.updateTree((t) => ({ ...t, rootOrder: [] }));
  assert.equal('tree' in new DeckStore(file).read(), false);
});
