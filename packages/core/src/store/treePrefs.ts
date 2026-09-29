import { randomUUID } from 'crypto';

/**
 * How sessions are organized, shared by both front ends. Projects are identified by their key:
 * `normalizeFsPath` of the project's git root (see `resolveProjectRoot`), the same thing both front
 * ends group sessions by. Folders are one level deep and hold projects only.
 */
export interface FolderPrefs {
  id: string;
  name: string;
  /** Member project keys, in display order. */
  projects: string[];
}

export type SessionSort = 'recent' | 'actionable';
export type GroupView = 'normal' | 'active';
export type SessionPin = 'top' | 'bottom';

/** Same categories the terminal UI's filter pills use. */
export type SessionCategory = 'running' | 'waiting' | 'idle' | 'error' | 'stopped';

export interface TreePrefs {
  /** In display order, shown before top-level projects. */
  folders: FolderPrefs[];
  /** Manual order of top-level projects. Projects not listed follow, in each front end's default order. */
  rootOrder: string[];
  /**
   * Manual order of each project's sessions (session id, keyed by project key), used only when
   * `DeckConfig.ui.recentSessionsFirst` is off. Sessions not listed follow, in the front end's default order.
   */
  sessionOrder: Record<string, string[]>;
  /** Collapsed folder/project nodes, see {@link folderNodeKey} / {@link projectNodeKey}. */
  collapsed: string[];
  sort: SessionSort;
  view: GroupView;
}

export function defaultTreePrefs(): TreePrefs {
  return { folders: [], rootOrder: [], sessionOrder: {}, collapsed: [], sort: 'recent', view: 'normal' };
}

export const folderNodeKey = (folderId: string) => `folder:${folderId}`;
export const projectNodeKey = (projectKey: string) => `project:${projectKey}`;

const stringArray = (value: unknown): string[] => (Array.isArray(value) ? value.filter((v): v is string => typeof v === 'string') : []);

/** Tolerant: non-object values, or entries whose value isn't a string array, are dropped (empty arrays too). */
const stringArrayRecord = (value: unknown): Record<string, string[]> => {
  if (typeof value !== 'object' || value === null) {
    return {};
  }
  const result: Record<string, string[]> = {};
  for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
    const arr = stringArray(v);
    if (arr.length) {
      result[k] = arr;
    }
  }
  return result;
};

/** Tolerant: invalid parts fall back to defaults, and a project listed in two places keeps its first one. */
export function parseTreePrefs(raw: unknown): TreePrefs | undefined {
  if (typeof raw !== 'object' || raw === null) {
    return undefined;
  }
  const r = raw as Record<string, unknown>;
  const tree = defaultTreePrefs();
  const placed = new Set<string>();
  for (const f of Array.isArray(r.folders) ? r.folders : []) {
    const { id, name, projects } = (f ?? {}) as Record<string, unknown>;
    if (typeof id !== 'string' || typeof name !== 'string' || !name.trim() || tree.folders.some((x) => x.id === id)) {
      continue;
    }
    const members = stringArray(projects).filter((p) => !placed.has(p));
    members.forEach((p) => placed.add(p));
    tree.folders.push({ id, name, projects: members });
  }
  tree.rootOrder = stringArray(r.rootOrder).filter((p) => !placed.has(p));
  tree.sessionOrder = stringArrayRecord(r.sessionOrder);
  tree.collapsed = [...new Set(stringArray(r.collapsed))];
  if (r.sort === 'recent' || r.sort === 'actionable') {
    tree.sort = r.sort;
  }
  if (r.view === 'normal' || r.view === 'active') {
    tree.view = r.view;
  }
  return tree;
}

export function isDefaultTree(tree: TreePrefs): boolean {
  return JSON.stringify(tree) === JSON.stringify(defaultTreePrefs());
}

// ---------------------------------------------------------------------------------------------
// Pure operations: each returns a new TreePrefs
// ---------------------------------------------------------------------------------------------

export function createFolder(tree: TreePrefs, name: string): { tree: TreePrefs; folderId: string } {
  const folderId = randomUUID().slice(0, 8);
  return { tree: { ...tree, folders: [...tree.folders, { id: folderId, name: name.trim(), projects: [] }] }, folderId };
}

export function renameFolder(tree: TreePrefs, folderId: string, name: string): TreePrefs {
  return { ...tree, folders: tree.folders.map((f) => (f.id === folderId ? { ...f, name: name.trim() } : f)) };
}

/** Its projects move back to the top level, ahead of the other ordered ones. */
export function deleteFolder(tree: TreePrefs, folderId: string): TreePrefs {
  const folder = tree.folders.find((f) => f.id === folderId);
  if (!folder) {
    return tree;
  }
  return {
    ...tree,
    folders: tree.folders.filter((f) => f.id !== folderId),
    rootOrder: [...folder.projects, ...tree.rootOrder],
    collapsed: tree.collapsed.filter((k) => k !== folderNodeKey(folderId)),
  };
}

/** Into a folder (appended at its end), or back to the top level with `folderId` null. */
export function moveProjectToFolder(tree: TreePrefs, projectKey: string, folderId: string | null): TreePrefs {
  const folders = tree.folders.map((f) => ({ ...f, projects: f.projects.filter((p) => p !== projectKey) }));
  const rootOrder = tree.rootOrder.filter((p) => p !== projectKey);
  if (folderId === null) {
    rootOrder.push(projectKey);
  } else {
    const target = folders.find((f) => f.id === folderId);
    if (!target) {
      return tree;
    }
    target.projects.push(projectKey);
  }
  return { ...tree, folders, rootOrder };
}

export function moveFolder(tree: TreePrefs, folderId: string, delta: number): TreePrefs {
  const i = tree.folders.findIndex((f) => f.id === folderId);
  const j = i + delta;
  if (i < 0 || j < 0 || j >= tree.folders.length) {
    return tree;
  }
  const folders = [...tree.folders];
  [folders[i], folders[j]] = [folders[j], folders[i]];
  return { ...tree, folders };
}

/**
 * Swaps a project with its displayed neighbor. `displayed` is the container's current order as shown
 * (which may omit projects the other front end shows, or include ones never ordered): those hidden keep
 * their stored positions, and unordered displayed ones are appended in displayed order first.
 */
export function moveProject(tree: TreePrefs, projectKey: string, delta: number, displayed: string[]): TreePrefs {
  const folder = tree.folders.find((f) => f.projects.includes(projectKey));
  const list = [...(folder ? folder.projects : tree.rootOrder)];
  for (const key of displayed) {
    if (!list.includes(key)) {
      list.push(key);
    }
  }
  const visible = list.filter((k) => displayed.includes(k));
  const i = visible.indexOf(projectKey);
  const j = i + delta;
  if (i < 0 || j < 0 || j >= visible.length) {
    return tree;
  }
  const a = list.indexOf(visible[i]);
  const b = list.indexOf(visible[j]);
  [list[a], list[b]] = [list[b], list[a]];
  return folder
    ? { ...tree, folders: tree.folders.map((f) => (f === folder ? { ...f, projects: list } : f)) }
    : { ...tree, rootOrder: list };
}

/**
 * Swaps a session with its displayed neighbor within one project. `displayed` is that project's
 * currently shown session ids (may include ones never ordered): those keep their stored positions,
 * and unordered displayed ones are appended in displayed order first. See `moveProject`.
 */
export function moveSession(tree: TreePrefs, projectKey: string, sessionId: string, delta: number, displayed: string[]): TreePrefs {
  const list = [...(tree.sessionOrder[projectKey] ?? [])];
  for (const id of displayed) {
    if (!list.includes(id)) {
      list.push(id);
    }
  }
  const visible = list.filter((id) => displayed.includes(id));
  const i = visible.indexOf(sessionId);
  const j = i + delta;
  if (i < 0 || j < 0 || j >= visible.length) {
    return tree;
  }
  const a = list.indexOf(visible[i]);
  const b = list.indexOf(visible[j]);
  [list[a], list[b]] = [list[b], list[a]];
  return { ...tree, sessionOrder: { ...tree.sessionOrder, [projectKey]: list } };
}

/**
 * With `DeckConfig.ui.recentSessionsFirst` off, puts a brand-new session at the very front of its
 * project's manual order instead of leaving it to fall in at the back of `sortSessionsManual`'s
 * fallback (see `tree.ts`). Called once, right when the session is created (or, for Claude, once its
 * id is known) — from then on it only moves via `moveSession` (`K`/`J`) like any other session.
 */
export function prependSession(tree: TreePrefs, projectKey: string, sessionId: string): TreePrefs {
  const existing = tree.sessionOrder[projectKey] ?? [];
  const list = [sessionId, ...existing.filter((id) => id !== sessionId)];
  return { ...tree, sessionOrder: { ...tree.sessionOrder, [projectKey]: list } };
}

/**
 * With `DeckConfig.ui.recentSessionsFirst` off, locks in each project's currently displayed session
 * order for any session not yet in `sessionOrder` — otherwise it keeps falling back to
 * most-recent-first (see `sortSessionsManual` in `tree.ts`), so it would still shuffle as it becomes
 * active. `displayed` is a project's currently shown session ids, keyed by project key. From here on a
 * session only moves via `moveSession` (`K`/`J`).
 */
export function freezeSessionOrder(tree: TreePrefs, displayed: Map<string, string[]>): TreePrefs {
  let sessionOrder = tree.sessionOrder;
  for (const [key, ids] of displayed) {
    const stored = sessionOrder[key] ?? [];
    const missing = ids.filter((id) => !stored.includes(id));
    if (missing.length) {
      sessionOrder = { ...sessionOrder, [key]: [...stored, ...missing] };
    }
  }
  return sessionOrder === tree.sessionOrder ? tree : { ...tree, sessionOrder };
}

/** Orders `ids` (already in a default/fallback order) by `manual`, keeping unlisted ones in that fallback order at the end. */
export function arrangeByManualOrder(ids: string[], manual: string[]): string[] {
  const present = new Set(ids);
  const ordered = manual.filter((id) => present.has(id));
  const orderedSet = new Set(ordered);
  const rest = ids.filter((id) => !orderedSet.has(id));
  return [...ordered, ...rest];
}

export function setCollapsed(tree: TreePrefs, nodeKey: string, collapsed: boolean): TreePrefs {
  const rest = tree.collapsed.filter((k) => k !== nodeKey);
  return { ...tree, collapsed: collapsed ? [...rest, nodeKey] : rest };
}

// ---------------------------------------------------------------------------------------------
// Arranging for display
// ---------------------------------------------------------------------------------------------

export interface ArrangedProjects {
  folders: { folder: FolderPrefs; projects: string[] }[];
  root: string[];
}

/**
 * Places the projects a front end has (given in its own default order) into folders and top-level
 * order. Folders keep only projects present in `projectKeys`; empty folders are still returned.
 */
export function arrangeProjects(tree: TreePrefs, projectKeys: string[]): ArrangedProjects {
  const present = new Set(projectKeys);
  const inFolder = new Set(tree.folders.flatMap((f) => f.projects));
  const ordered = tree.rootOrder.filter((k) => present.has(k) && !inFolder.has(k));
  const rest = projectKeys.filter((k) => !inFolder.has(k) && !ordered.includes(k));
  return {
    folders: tree.folders.map((folder) => ({ folder, projects: folder.projects.filter((k) => present.has(k)) })),
    root: [...ordered, ...rest],
  };
}

const ACTIONABLE_RANK: Record<SessionCategory, number> = { error: 0, waiting: 1, running: 2, idle: 3, stopped: 4 };

/** Pinned-to-top first, then the rest by `sort`, then pinned-to-bottom. Pinned bands are newest first. */
export function sortSessions<T>(
  items: T[],
  sort: SessionSort,
  pinOf: (item: T) => SessionPin | undefined,
  mtimeOf: (item: T) => number,
  categoryOf: (item: T) => SessionCategory
): T[] {
  const byRecent = (a: T, b: T) => mtimeOf(b) - mtimeOf(a);
  const byActionable = (a: T, b: T) => ACTIONABLE_RANK[categoryOf(a)] - ACTIONABLE_RANK[categoryOf(b)] || byRecent(a, b);
  return [
    ...items.filter((i) => pinOf(i) === 'top').sort(byRecent),
    ...items.filter((i) => !pinOf(i)).sort(sort === 'actionable' ? byActionable : byRecent),
    ...items.filter((i) => pinOf(i) === 'bottom').sort(byRecent),
  ];
}
