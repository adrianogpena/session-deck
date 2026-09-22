import * as vscode from 'vscode';
import * as fs from 'fs';
import * as path from 'path';
import { normalizeFsPath } from '../discovery/pathUtils';

/**
 * A `projects` entry can be a bare root path, or an object naming it and carrying its per-project
 * overrides — `name` (and everything below) is what makes this file hand-editable.
 */
export interface WorkspaceProjectEntry {
  root: string;
  name?: string;
  /** Skip the "Skip Permissions" confirmation dialog for *this* project specifically, regardless of the global `sessionDeck.confirmDangerousSkipPermissions` setting — for a project you've already decided to trust. Never makes a project *more* confirmed than the global default; there's no way to force confirmation back on for one project while it's off globally. */
  dangerouslySkipPermissions?: boolean;
  /** Per-project override of the default cap (`MAX_SESSIONS_PER_PROJECT_VIEW` in `sessionProvider.ts`) on how many active sessions this project shows before the rest auto-archive. */
  maxSessionsShown?: number;
  /** A literal emoji prefixed onto the project's tree label — takes priority over `color` if both are set. */
  emoji?: string;
  /** A named color swatch (see `COLOR_SWATCH_EMOJI` in `sessionProvider.ts`) prefixed onto the project's tree label — a plain-text swatch rather than a `ThemeColor`'d icon, since a `TreeItem.iconPath` colored that way washes out to the row's foreground color when selected (the same reason session status uses a `FileDecoration` instead of its icon). */
  color?: string;
  /** Soft-hide: keeps the entry (and whatever `name`/other overrides it carries) in this file, just leaves it out of the tree — unlike "Remove Project", which deletes the entry outright. Only reachable back to visible by hand-editing this file (there's a "Hide" action, deliberately no "Unhide" one — a hidden project isn't shown anywhere to attach that action's context menu to). */
  hidden?: boolean;
}

interface WorkspaceProjectConfigFile {
  projects: (string | WorkspaceProjectEntry)[];
}

const CONFIG_RELATIVE_PATH = path.join('.vscode', 'session-deck.json');

function configPathForFolder(folder: vscode.WorkspaceFolder): string {
  return path.join(folder.uri.fsPath, CONFIG_RELATIVE_PATH);
}

function normalizeEntry(entry: string | WorkspaceProjectEntry): WorkspaceProjectEntry {
  return typeof entry === 'string' ? { root: entry } : entry;
}

/** `undefined` means this specific folder has no `.vscode/session-deck.json` (or it's malformed) — distinct from `readWorkspaceProjectEntries()`'s workspace-wide merge below. */
function readEntriesFromFile(configPath: string): WorkspaceProjectEntry[] | undefined {
  if (!fs.existsSync(configPath)) {
    return undefined;
  }
  try {
    const parsed = JSON.parse(fs.readFileSync(configPath, 'utf8')) as WorkspaceProjectConfigFile;
    return Array.isArray(parsed.projects) ? parsed.projects.map(normalizeEntry) : [];
  } catch {
    // Malformed file: behave as "not configured" rather than crash the tree view.
    return undefined;
  }
}

function writeEntriesToFile(configPath: string, entries: WorkspaceProjectEntry[]): void {
  fs.mkdirSync(path.dirname(configPath), { recursive: true });
  fs.writeFileSync(configPath, `${JSON.stringify({ projects: entries }, null, 2)}\n`, 'utf8');
}

/**
 * For opening/bootstrapping one specific folder's file directly (the "Edit Project List" button,
 * after the caller has already resolved which folder — see `pickTargetWorkspaceFolder` in
 * `extension.ts`). Defaults to the first workspace folder when none is given, which is exactly
 * today's single-root behavior for every existing caller that doesn't pass one.
 */
export function getConfigFsPath(folder?: vscode.WorkspaceFolder): string | undefined {
  const target = folder ?? vscode.workspace.workspaceFolders?.[0];
  return target ? configPathForFolder(target) : undefined;
}

/**
 * A plain (non-`RelativePattern`) glob string — VS Code applies a bare string pattern passed to
 * `createFileSystemWatcher` against *every* open workspace folder automatically (one underlying
 * watcher per folder, under the hood), unlike a `RelativePattern`, which is pinned to a single base.
 * That's exactly what's wanted here: live-refresh when *any* workspace folder's
 * `.vscode/session-deck.json` is created/edited/deleted by hand, not just the first one.
 */
export function getConfigGlobPattern(): string | undefined {
  return vscode.workspace.workspaceFolders?.length ? CONFIG_RELATIVE_PATH.replace(/\\/g, '/') : undefined;
}

/** Case-insensitive on Windows: a hand-typed entry and a freshly git-resolved root can differ only by casing. */
function matchesRoot(entry: WorkspaceProjectEntry, rootPath: string): boolean {
  return normalizeFsPath(entry.root) === normalizeFsPath(rootPath);
}

/**
 * `undefined` means *no workspace folder at all* has a `.vscode/session-deck.json` — the sidebar
 * shows nothing until at least one exists. Once any folder has one — even as `{ "projects": [] }`
 * — every open folder's own file (if it has one) contributes to this merged, de-duplicated list
 * (by normalized root, first occurrence wins if the same root is somehow listed in more than one
 * folder's file); this is the workspace-wide view every simple "just give me the projects" caller
 * wants (`sessionProvider.ts`, Add Project's "already added" filter). A `name` on an entry overrides
 * the default (last path segment) display name, same as always.
 */
export function readWorkspaceProjectEntries(): WorkspaceProjectEntry[] | undefined {
  const folders = vscode.workspace.workspaceFolders ?? [];
  const merged: WorkspaceProjectEntry[] = [];
  const seenRoots = new Set<string>();
  let anyConfigured = false;

  for (const folder of folders) {
    const entries = readEntriesFromFile(configPathForFolder(folder));
    if (entries === undefined) {
      continue;
    }
    anyConfigured = true;
    for (const entry of entries) {
      const key = normalizeFsPath(entry.root);
      if (seenRoots.has(key)) {
        continue;
      }
      seenRoots.add(key);
      merged.push(entry);
    }
  }

  return anyConfigured ? merged : undefined;
}

/** This one specific folder's own entries — `undefined` if it has no file yet (or it's malformed). Used to decide whether "Edit Project List" needs to bootstrap an empty file for the folder the user picked. */
export function readWorkspaceProjectEntriesForFolder(folder: vscode.WorkspaceFolder): WorkspaceProjectEntry[] | undefined {
  return readEntriesFromFile(configPathForFolder(folder));
}

/** This project's raw config entry, whatever overrides it carries, from whichever workspace folder's file actually has it — `undefined` if it isn't on any list. */
export function getWorkspaceProjectEntry(rootPath: string): WorkspaceProjectEntry | undefined {
  return (readWorkspaceProjectEntries() ?? []).find((e) => matchesRoot(e, rootPath));
}

/** Finds which workspace folder's file (if any) currently has an entry for `rootPath` — the shared lookup every write on an *existing* root (rename/remove/hide) uses, so it patches the file that actually owns the entry rather than guessing. */
function findEntryLocation(
  rootPath: string
): { configPath: string; entries: WorkspaceProjectEntry[]; index: number } | undefined {
  for (const folder of vscode.workspace.workspaceFolders ?? []) {
    const configPath = configPathForFolder(folder);
    const entries = readEntriesFromFile(configPath);
    if (!entries) {
      continue;
    }
    const index = entries.findIndex((e) => matchesRoot(e, rootPath));
    if (index !== -1) {
      return { configPath, entries, index };
    }
  }
  return undefined;
}

/**
 * `name` is optional but should normally be passed — an explicit name makes a
 * newly-added entry self-documenting and immediately editable, matching what
 * the "Remove"/"Edit Project List" bootstrap paths already write, rather than a
 * bare `{ "root": "..." }` that only happens to *display* using the default
 * (last path segment) name.
 *
 * `targetFolder` is which workspace folder's file the new entry is written into — required when
 * there's more than one folder open (the caller, `extension.ts`'s `addProject`, resolves it via
 * `pickTargetWorkspaceFolder` first); defaults to the single folder when there's only one, so every
 * existing single-root caller keeps working unchanged.
 */
export async function addProjectToWorkspaceList(
  rootPath: string,
  name: string | undefined,
  targetFolder?: vscode.WorkspaceFolder
): Promise<void> {
  const folder = targetFolder ?? vscode.workspace.workspaceFolders?.[0];
  if (!folder) {
    throw new Error('No workspace folder is open.');
  }
  const configPath = configPathForFolder(folder);
  const current = readEntriesFromFile(configPath) ?? [];
  if (!current.some((e) => matchesRoot(e, rootPath))) {
    current.push(name ? { root: rootPath, name } : { root: rootPath });
  }
  writeEntriesToFile(configPath, current);
}

export async function removeProjectFromWorkspaceList(rootPath: string): Promise<void> {
  const location = findEntryLocation(rootPath);
  if (!location) {
    return; // Not on any list — nothing to do, matches the previous no-op-if-absent behavior.
  }
  location.entries.splice(location.index, 1);
  writeEntriesToFile(location.configPath, location.entries);
}

/**
 * Sets/overwrites the display name for an entry already on some workspace folder's
 * list. Returns false (does nothing) if the project isn't on any list — the
 * caller decides what to do then (e.g. fall back to the global rename).
 */
export function setWorkspaceProjectName(rootPath: string, name: string): boolean {
  return updateWorkspaceProjectEntry(rootPath, { name });
}

/** Sets/clears an entry's `hidden` flag ("Hide Project") — see {@link WorkspaceProjectEntry.hidden}. Same "not on any list" behavior as {@link setWorkspaceProjectName}. */
export function setWorkspaceProjectHidden(rootPath: string, hidden: boolean): boolean {
  return updateWorkspaceProjectEntry(rootPath, { hidden: hidden || undefined });
}

function updateWorkspaceProjectEntry(rootPath: string, patch: Partial<WorkspaceProjectEntry>): boolean {
  const location = findEntryLocation(rootPath);
  if (!location) {
    return false;
  }
  location.entries[location.index] = { ...location.entries[location.index], ...patch };
  writeEntriesToFile(location.configPath, location.entries);
  return true;
}

/**
 * Replaces one specific folder's whole list, creating its `.vscode/session-deck.json` if it
 * doesn't exist yet. Used to bootstrap a file — e.g. the first time "Edit Project List" is used for
 * a folder that doesn't have one yet — instead of silently falling back to a global hide.
 * `targetFolder` defaults to the single workspace folder, same reasoning as `addProjectToWorkspaceList`.
 */
export function writeWorkspaceProjectEntries(entries: WorkspaceProjectEntry[], targetFolder?: vscode.WorkspaceFolder): void {
  const folder = targetFolder ?? vscode.workspace.workspaceFolders?.[0];
  if (!folder) {
    throw new Error('No workspace folder is open.');
  }
  writeEntriesToFile(configPathForFolder(folder), entries);
}
