import * as path from 'path';
import { arrangeProjects, folderNodeKey, GitStatus, projectNodeKey, SessionCategory, SessionPin, sortSessions, TreePrefs } from '@session-deck/core';
import type { DeckSession } from './sessions';

export type TreeRow =
  | { kind: 'folder'; folderId: string; name: string; count: number; running: number; waiting: number; collapsed: boolean; hotkey?: number }
  | {
      kind: 'project';
      projectKey: string;
      label: string;
      root: string;
      count: number;
      running: number;
      waiting: number;
      collapsed: boolean;
      /** 1 inside a folder. */
      depth: number;
      hotkey?: number;
      /** Worst case across the project's visible sessions (they usually share a cwd, so usually all identical). */
      git?: GitStatus;
    }
  | { kind: 'session'; session: DeckSession; isLast: boolean; depth: number; pin?: SessionPin }
  | { kind: 'divider'; label: string };

export interface TreeOptions {
  include(s: DeckSession): boolean;
  categoryOf(s: DeckSession): SessionCategory;
  pinOf(s: DeckSession): SessionPin | undefined;
  /** `undefined` when `ui.gitStatus` is off, or the session's cwd isn't a git repo (or hasn't been polled yet). */
  gitOf(s: DeckSession): GitStatus | undefined;
  /** A status/time filter is on: groups with nothing visible are hidden, even empty folders. */
  filtering: boolean;
  /** Projects never moved (`K`/`J`) sort by most-recent-activity when true, alphabetically (fixed) when false. See `DeckConfig.ui.recentProjectsFirst`. */
  recentProjectsFirst: boolean;
}

export interface BuiltTree {
  rows: TreeRow[];
  /** Displayed project order per container (`''` = top level, else folder id), for reordering. */
  containers: Map<string, string[]>;
}

interface ProjectBucket {
  key: string;
  root: string;
  all: DeckSession[];
  visible: DeckSession[];
}

const isActive = (c: SessionCategory) => c === 'running' || c === 'waiting';

/** One badge for the whole project: worst (highest) ahead/behind/dirty across its sessions — usually all identical, since they usually share a cwd. */
function aggregateGit(statuses: (GitStatus | undefined)[]): GitStatus | undefined {
  const present = statuses.filter((g): g is GitStatus => !!g);
  if (!present.length) {
    return undefined;
  }
  return {
    ahead: Math.max(...present.map((g) => g.ahead)),
    behind: Math.max(...present.map((g) => g.behind)),
    dirty: Math.max(...present.map((g) => g.dirty)),
  };
}

/** The folder name, plus its parent's name (`acme-storefront · demo-projects`) when two projects share a name. */
export function projectLabels(roots: string[]): Map<string, string> {
  const base = (root: string) => path.basename(root) || root;
  const counts = new Map<string, number>();
  for (const root of roots) {
    counts.set(base(root).toLowerCase(), (counts.get(base(root).toLowerCase()) ?? 0) + 1);
  }
  return new Map(roots.map((root) => [root, (counts.get(base(root).toLowerCase()) ?? 0) > 1 ? `${base(root)} · ${path.basename(path.dirname(root))}` : base(root)]));
}

/** Stable partition: items with active sessions first. */
function activeFirst<T>(items: T[], active: (item: T) => boolean): { active: T[]; rest: T[] } {
  return { active: items.filter(active), rest: items.filter((i) => !active(i)) };
}

/**
 * Folders (in their manual order) then top-level projects. Projects never moved (`K`/`J`) default to
 * a fixed alphabetical order, or most recent activity first when `opts.recentProjectsFirst` is on.
 * Collapsed nodes hide their children. In the "active" view, groups with running/waiting sessions
 * come first, and an "idle / done" divider separates the rest.
 */
export function buildTree(sessions: DeckSession[], tree: TreePrefs, opts: TreeOptions): BuiltTree {
  const buckets = new Map<string, ProjectBucket>();
  for (const s of sessions) {
    let bucket = buckets.get(s.projectKey);
    if (!bucket) {
      bucket = { key: s.projectKey, root: s.projectRoot, all: [], visible: [] };
      buckets.set(s.projectKey, bucket);
    }
    bucket.all.push(s);
    if (opts.include(s)) {
      bucket.visible.push(s);
    }
  }
  const latest = (b: ProjectBucket) => Math.max(...b.all.map((s) => s.mtime));
  const labels = projectLabels([...buckets.values()].map((bucket) => bucket.root));
  const defaultOrder = opts.recentProjectsFirst
    ? [...buckets.values()].sort((a, b) => latest(b) - latest(a)).map((b) => b.key)
    : [...buckets.values()].sort((a, b) => (labels.get(a.root) ?? a.root).localeCompare(labels.get(b.root) ?? b.root)).map((b) => b.key);
  const arranged = arrangeProjects(tree, defaultOrder);
  const collapsed = new Set(tree.collapsed);
  const counts = (list: DeckSession[]) => {
    const categories = list.map(opts.categoryOf);
    return { count: list.length, running: categories.filter((c) => c === 'running').length, waiting: categories.filter((c) => c === 'waiting').length };
  };
  const shownProjects = (keys: string[]) => keys.map((k) => buckets.get(k)!).filter((b) => b.visible.length > 0 || !opts.filtering);
  const projectActive = (b: ProjectBucket) => b.visible.some((s) => isActive(opts.categoryOf(s)));

  const rows: TreeRow[] = [];
  const containers = new Map<string, string[]>();
  let hotkey = 1;
  const nextHotkey = () => (hotkey <= 9 ? hotkey++ : undefined);

  const emitProject = (b: ProjectBucket, depth: number) => {
    const isCollapsed = collapsed.has(projectNodeKey(b.key));
    rows.push({
      kind: 'project',
      projectKey: b.key,
      label: labels.get(b.root) ?? b.root,
      root: b.root,
      ...counts(b.visible),
      collapsed: isCollapsed,
      depth,
      hotkey: depth === 0 ? nextHotkey() : undefined,
      git: aggregateGit(b.visible.map(opts.gitOf)),
    });
    if (isCollapsed) {
      return;
    }
    const ordered = sortSessions(b.visible, tree.sort, opts.pinOf, (s) => s.mtime, opts.categoryOf);
    ordered.forEach((s, i) => rows.push({ kind: 'session', session: s, isLast: i === ordered.length - 1, depth: depth + 1, pin: opts.pinOf(s) }));
  };

  const folderItems = arranged.folders
    .map(({ folder, projects }) => ({ folder, projects: shownProjects(projects) }))
    .filter((f) => f.projects.length > 0 || !opts.filtering);
  const rootItems = shownProjects(arranged.root);

  const emitFolder = ({ folder, projects }: (typeof folderItems)[number]) => {
    const split = activeFirst(projects, projectActive);
    const ordered = tree.view === 'active' ? [...split.active, ...split.rest] : projects;
    containers.set(folder.id, ordered.map((b) => b.key));
    const isCollapsed = collapsed.has(folderNodeKey(folder.id));
    rows.push({
      kind: 'folder',
      folderId: folder.id,
      name: folder.name,
      ...counts(projects.flatMap((b) => b.visible)),
      collapsed: isCollapsed,
      hotkey: nextHotkey(),
    });
    if (!isCollapsed) {
      ordered.forEach((b) => emitProject(b, 1));
    }
  };

  if (tree.view === 'active') {
    const folders = activeFirst(folderItems, (f) => f.projects.some(projectActive));
    const projects = activeFirst(rootItems, projectActive);
    containers.set('', [...projects.active, ...projects.rest].map((b) => b.key));
    folders.active.forEach(emitFolder);
    projects.active.forEach((b) => emitProject(b, 0));
    if ((folders.active.length || projects.active.length) && (folders.rest.length || projects.rest.length)) {
      rows.push({ kind: 'divider', label: 'idle / done' });
    }
    folders.rest.forEach(emitFolder);
    projects.rest.forEach((b) => emitProject(b, 0));
  } else {
    containers.set('', rootItems.map((b) => b.key));
    folderItems.forEach(emitFolder);
    rootItems.forEach((b) => emitProject(b, 0));
  }
  return { rows, containers };
}
