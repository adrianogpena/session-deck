import { test } from 'node:test';
import assert from 'node:assert/strict';
import { defaultTreePrefs, folderNodeKey, freezeSessionOrder, projectNodeKey, SessionCategory, TreePrefs } from '@session-deck/core';
import type { DeckSession } from '../sessions';
import { buildTree, projectLabels, TreeRow } from '../tree';

function session(id: string, project: string, mtime: number): DeckSession {
  const root = `C:\\repos\\${project}`;
  return { agent: 'claude', id, cwd: root, projectRoot: root, projectKey: root.toLowerCase(), title: id, mtime };
}

const key = (project: string) => `c:\\repos\\${project}`;

/** Compact picture of the rows: "F:Work(2)", "P:api", "  s:a1", "--". */
function outline(rows: TreeRow[]): string[] {
  return rows.map((r) => {
    if (r.kind === 'folder') return `${r.hotkey ?? ' '}F:${r.name}(${r.count})${r.collapsed ? '+' : ''}`;
    if (r.kind === 'project') return `${r.hotkey ?? ' '}${'  '.repeat(r.depth)}P:${r.label}${r.collapsed ? '+' : ''}`;
    if (r.kind === 'session') return `${'  '.repeat(r.depth)}s:${r.session.id}${r.pin ? '^' : ''}`;
    if (r.kind === 'tag') return `T:${r.name}(${r.count})`;
    return '--';
  });
}

const sessions = [session('a1', 'api', 50), session('a2', 'api', 10), session('w1', 'web', 40), session('d1', 'docs', 30)];
const categories: Record<string, SessionCategory> = { a1: 'idle', a2: 'stopped', w1: 'running', d1: 'stopped' };
const opts = (overrides: Partial<Parameters<typeof buildTree>[2]> = {}) => ({
  include: () => true,
  categoryOf: (s: DeckSession) => categories[s.id!],
  pinOf: () => undefined,
  gitOf: () => undefined,
  filtering: false,
  recentProjectsFirst: false,
  recentSessionsFirst: true,
  tagsOf: () => [],
  ...overrides,
});
const workTree = (): TreePrefs => ({ ...defaultTreePrefs(), folders: [{ id: 'f1', name: 'Work', projects: [key('web'), key('docs')] }] });

test('buildTree puts folders first, then top-level projects, numbering top-level rows', () => {
  const { rows, containers } = buildTree(sessions, workTree(), opts());
  assert.deepEqual(outline(rows), ['1F:Work(2)', '   P:web', '    s:w1', '   P:docs', '    s:d1', '2P:api', '  s:a1', '  s:a2']);
  assert.deepEqual(containers.get('f1'), [key('web'), key('docs')]);
  assert.deepEqual(containers.get(''), [key('api')]);
});

test('buildTree defaults top-level projects to a fixed alphabetical order, or most-recent-activity first when recentProjectsFirst is on', () => {
  const projectLabelsOf = (rows: TreeRow[]) => rows.filter((r): r is Extract<TreeRow, { kind: 'project' }> => r.kind === 'project').map((r) => r.label);
  const fixed = buildTree(sessions, defaultTreePrefs(), opts()).rows;
  assert.deepEqual(projectLabelsOf(fixed), ['api', 'docs', 'web']);
  const recent = buildTree(sessions, defaultTreePrefs(), opts({ recentProjectsFirst: true })).rows;
  assert.deepEqual(projectLabelsOf(recent), ['api', 'web', 'docs']);
});

test('buildTree keeps a fixed session order when recentSessionsFirst is off, moving only via the stored sessionOrder', () => {
  const tree = { ...defaultTreePrefs(), sessionOrder: { [key('api')]: ['a2', 'a1'] } };
  const built = buildTree(sessions, tree, opts({ recentSessionsFirst: false }));
  assert.deepEqual(outline(built.rows).slice(0, 3), ['1P:api', '  s:a2', '  s:a1']);
  assert.deepEqual(built.sessionContainers.get(key('api')), ['a2', 'a1']);
  // Unordered project (docs) falls back to most-recent-first for sessions it hasn't stored an order for.
  assert.deepEqual(built.sessionContainers.get(key('docs')), ['d1']);
});

test('reproduces the reported bug: with recentSessionsFirst off, a session that becomes newest does not jump up once frozen (app.ts wires buildTree + freezeSessionOrder together on every render)', () => {
  // api starts with a2 (mtime 10) second, behind a1 (mtime 50) — nothing manually ordered yet.
  let tree = defaultTreePrefs();
  const renderOnce = (currentSessions: DeckSession[]) => {
    const built = buildTree(currentSessions, tree, opts({ recentSessionsFirst: false }));
    tree = freezeSessionOrder(tree, built.sessionContainers); // what app.ts's rebuildRows() does each time
    return built;
  };

  const first = renderOnce(sessions);
  assert.deepEqual(outline(first.rows).slice(0, 3), ['1P:api', '  s:a1', '  s:a2']); // fallback: mtime order, now frozen

  // a2 becomes the most recently active session in the project (its transcript just got a new turn).
  const bumped = [session('a1', 'api', 50), session('a2', 'api', 999), session('w1', 'web', 40), session('d1', 'docs', 30)];
  const second = renderOnce(bumped);
  assert.deepEqual(outline(second.rows).slice(0, 3), ['1P:api', '  s:a1', '  s:a2']); // unchanged: order is frozen, not activity-based
});

test('buildTree hides the children of collapsed folders and projects', () => {
  const tree = { ...workTree(), collapsed: [folderNodeKey('f1'), projectNodeKey(key('api'))] };
  assert.deepEqual(outline(buildTree(sessions, tree, opts()).rows), ['1F:Work(2)+', '2P:api+']);
});

test('buildTree with a filter hides projects and folders with nothing visible', () => {
  const tree = { ...workTree(), folders: [...workTree().folders, { id: 'f2', name: 'Empty', projects: [] }] };
  assert.deepEqual(outline(buildTree(sessions, tree, opts()).rows).slice(5, 6), ['2F:Empty(0)']); // shown without a filter
  const running = buildTree(sessions, tree, opts({ include: (s) => categories[s.id!] === 'running', filtering: true }));
  assert.deepEqual(outline(running.rows), ['1F:Work(1)', '   P:web', '    s:w1']);
});

test('buildTree "active" view hoists groups with running/waiting sessions above an idle / done divider', () => {
  const tree: TreePrefs = { ...defaultTreePrefs(), view: 'active' };
  const rows = buildTree(sessions, tree, opts()).rows;
  assert.deepEqual(outline(rows), ['1P:web', '  s:w1', '--', '2P:api', '  s:a1', '  s:a2', '3P:docs', '  s:d1']);
});

test('buildTree orders pinned sessions around the rest', () => {
  const rows = buildTree(sessions, defaultTreePrefs(), opts({ pinOf: (s) => (s.id === 'a2' ? 'top' : undefined) })).rows;
  assert.deepEqual(outline(rows).slice(0, 3), ['1P:api', '  s:a2^', '  s:a1']);
});

test('buildTree aggregates git status across a project\'s sessions to the worst case, one badge per project', () => {
  const gitBySessionId: Record<string, { ahead: number; behind: number; dirty: number }> = {
    a1: { ahead: 0, behind: 0, dirty: 0 },
    a2: { ahead: 2, behind: 0, dirty: 5 },
  };
  const rows = buildTree(sessions, defaultTreePrefs(), opts({ gitOf: (s) => gitBySessionId[s.id!] })).rows;
  const project = (label: string) => rows.find((r): r is Extract<TreeRow, { kind: 'project' }> => r.kind === 'project' && r.label === label)!;
  assert.deepEqual(project('api').git, { ahead: 2, behind: 0, dirty: 5 });
  assert.equal(project('docs').git, undefined); // d1 has no entry in gitBySessionId
});

test('buildTree appends a TAGS divider and one row per distinct tag, counting sessions, independent of the current filter', () => {
  const tagsBySession: Record<string, string[]> = { a1: ['urgent', 'vwde'], w1: ['vwde'] };
  const rows = buildTree(sessions, defaultTreePrefs(), opts({ tagsOf: (s) => tagsBySession[s.id!] ?? [] })).rows;
  assert.deepEqual(
    rows.filter((r) => r.kind === 'tag' || (r.kind === 'divider' && r.label === 'TAGS')),
    [
      { kind: 'divider', label: 'TAGS' },
      { kind: 'tag', name: 'urgent', count: 1 },
      { kind: 'tag', name: 'vwde', count: 2 },
    ]
  );
});

test('buildTree omits the TAGS section entirely when nothing is tagged', () => {
  const rows = buildTree(sessions, defaultTreePrefs(), opts()).rows;
  assert.equal(rows.some((r) => r.kind === 'tag' || (r.kind === 'divider' && r.label === 'TAGS')), false);
});

test('projectLabels adds the parent folder only when two projects share a name', () => {
  const labels = projectLabels(['C:\\repos\\api', 'X:\\demo\\shop', 'C:\\work\\shop']);
  assert.equal(labels.get('C:\\repos\\api'), 'api');
  assert.equal(labels.get('X:\\demo\\shop'), 'shop · demo');
  assert.equal(labels.get('C:\\work\\shop'), 'shop · work');
});
