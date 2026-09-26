import * as vscode from 'vscode';
import * as fs from 'fs';
import * as path from 'path';
import { normalizeFsPath, expandHome } from '@session-deck/core';

/** A `projects` entry can be a bare root path, or an object naming it and carrying per-project overrides. */
export interface WorkspaceProjectEntry {
  root: string;
  name?: string;
  /** Skip the "Skip Permissions" confirmation dialog for this project only. Never overrides the global setting to require *more* confirmation. */
  dangerouslySkipPermissions?: boolean;
  /** Per-project override of `MAX_SESSIONS_PER_PROJECT_VIEW`. */
  maxSessionsShown?: number;
  /** A literal emoji prefixed onto the project's tree label — takes priority over `color` if both are set. */
  emoji?: string;
  /** A named color swatch (see `COLOR_SWATCH_EMOJI`) prefixed onto the label — plain text, not a colored icon, since `TreeItem.iconPath` colors wash out when a row is selected. */
  color?: string;
  /** Soft-hide: keeps the entry in this file but leaves it out of the tree. No "Unhide" action — reachable only by hand-editing. */
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
  const normalized = typeof entry === 'string' ? { root: entry } : entry;
  return { ...normalized, root: expandHome(normalized.root) };
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

/** Defaults to the first workspace folder when none is given. */
export function getConfigFsPath(folder?: vscode.WorkspaceFolder): string | undefined {
  const target = folder ?? vscode.workspace.workspaceFolders?.[0];
  return target ? configPathForFolder(target) : undefined;
}

/** A plain glob string, not a `RelativePattern` — VS Code applies it across every open workspace folder, not just one. */
export function getConfigGlobPattern(): string | undefined {
  return vscode.workspace.workspaceFolders?.length ? CONFIG_RELATIVE_PATH.replace(/\\/g, '/') : undefined;
}

/** Case-insensitive on Windows: a hand-typed entry and a freshly git-resolved root can differ only by casing. */
function matchesRoot(entry: WorkspaceProjectEntry, rootPath: string): boolean {
  return normalizeFsPath(entry.root) === normalizeFsPath(rootPath);
}

/** `undefined` means no workspace folder has a `.vscode/session-deck.json` yet. Otherwise, every open folder's file (if any) is merged into one de-duplicated list, by normalized root. */
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

/** This one specific folder's own entries — `undefined` if it has no file yet (or it's malformed). */
export function readWorkspaceProjectEntriesForFolder(folder: vscode.WorkspaceFolder): WorkspaceProjectEntry[] | undefined {
  return readEntriesFromFile(configPathForFolder(folder));
}

/** This project's raw config entry, whatever overrides it carries, from whichever workspace folder's file actually has it — `undefined` if it isn't on any list. */
export function getWorkspaceProjectEntry(rootPath: string): WorkspaceProjectEntry | undefined {
  return (readWorkspaceProjectEntries() ?? []).find((e) => matchesRoot(e, rootPath));
}

/** Finds which workspace folder's file (if any) currently has an entry for `rootPath`. */
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

/** `targetFolder` defaults to the single workspace folder when there's only one; required when more than one is open. */
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

/** Returns false (does nothing) if the project isn't on any list. */
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

/** Replaces one folder's whole list, creating its `.vscode/session-deck.json` if it doesn't exist yet. */
export function writeWorkspaceProjectEntries(entries: WorkspaceProjectEntry[], targetFolder?: vscode.WorkspaceFolder): void {
  const folder = targetFolder ?? vscode.workspace.workspaceFolders?.[0];
  if (!folder) {
    throw new Error('No workspace folder is open.');
  }
  writeEntriesToFile(configPathForFolder(folder), entries);
}
