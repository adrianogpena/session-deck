import * as vscode from 'vscode';
import * as fs from 'fs';
import * as path from 'path';
import { normalizeFsPath } from './pathUtils';

/** A `projects` entry can be a bare root path, or an object naming it — `name` is what makes this file hand-editable. */
export interface WorkspaceProjectEntry {
  root: string;
  name?: string;
}

interface WorkspaceProjectConfigFile {
  projects: (string | WorkspaceProjectEntry)[];
}

const CONFIG_RELATIVE_PATH = path.join('.vscode', 'session-deck.json');

function getConfigPath(): string | undefined {
  const folder = vscode.workspace.workspaceFolders?.[0];
  return folder ? path.join(folder.uri.fsPath, CONFIG_RELATIVE_PATH) : undefined;
}

/** For opening the file directly (the "edit project list" view-title button). */
export function getConfigFsPath(): string | undefined {
  return getConfigPath();
}

export function getConfigRelativePattern(): vscode.RelativePattern | undefined {
  const folder = vscode.workspace.workspaceFolders?.[0];
  return folder ? new vscode.RelativePattern(folder, CONFIG_RELATIVE_PATH.replace(/\\/g, '/')) : undefined;
}

function normalizeEntry(entry: string | WorkspaceProjectEntry): WorkspaceProjectEntry {
  return typeof entry === 'string' ? { root: entry } : entry;
}

/**
 * `undefined` means "no `.vscode/session-deck.json` for this workspace" — the
 * sidebar shows nothing until one exists. Once it exists — even as
 * `{ "projects": [] }` — it's the sole source of truth for which roots show,
 * and a `name` on an entry overrides the default (last path segment) display
 * name.
 */
export function readWorkspaceProjectEntries(): WorkspaceProjectEntry[] | undefined {
  const configPath = getConfigPath();
  if (!configPath || !fs.existsSync(configPath)) {
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

/** Case-insensitive on Windows: a hand-typed entry and a freshly git-resolved root can differ only by casing. */
function matchesRoot(entry: WorkspaceProjectEntry, rootPath: string): boolean {
  return normalizeFsPath(entry.root) === normalizeFsPath(rootPath);
}

/**
 * `name` is optional but should normally be passed — an explicit name makes a
 * newly-added entry self-documenting and immediately editable, matching what
 * the "Remove"/"Edit Project List" bootstrap paths already write, rather than a
 * bare `{ "root": "..." }` that only happens to *display* using the default
 * (last path segment) name.
 */
export async function addProjectToWorkspaceList(rootPath: string, name?: string): Promise<void> {
  const current = readWorkspaceProjectEntries() ?? [];
  if (!current.some((e) => matchesRoot(e, rootPath))) {
    current.push(name ? { root: rootPath, name } : { root: rootPath });
  }
  writeWorkspaceProjectEntries(current);
}

export async function removeProjectFromWorkspaceList(rootPath: string): Promise<void> {
  const next = (readWorkspaceProjectEntries() ?? []).filter((e) => !matchesRoot(e, rootPath));
  writeWorkspaceProjectEntries(next);
}

/**
 * Sets/overwrites the display name for an entry already on this workspace's
 * list. Returns false (does nothing) if the project isn't on the list — the
 * caller decides what to do then (e.g. fall back to the global rename).
 */
export function setWorkspaceProjectName(rootPath: string, name: string): boolean {
  const entries = readWorkspaceProjectEntries();
  if (!entries) {
    return false;
  }
  const index = entries.findIndex((e) => matchesRoot(e, rootPath));
  if (index === -1) {
    return false;
  }
  entries[index] = { ...entries[index], name };
  writeWorkspaceProjectEntries(entries);
  return true;
}

/**
 * Replaces the whole list, creating `.vscode/session-deck.json` if it doesn't
 * exist yet. Used to bootstrap the file — e.g. the first time "Remove Project"
 * or "Edit Project List" is used in a workspace that doesn't have one yet,
 * seeded with everything currently visible — instead of silently falling back
 * to a global hide.
 */
export function writeWorkspaceProjectEntries(entries: WorkspaceProjectEntry[]): void {
  const configPath = getConfigPath();
  if (!configPath) {
    throw new Error('No workspace folder is open.');
  }
  fs.mkdirSync(path.dirname(configPath), { recursive: true });
  fs.writeFileSync(configPath, `${JSON.stringify({ projects: entries }, null, 2)}\n`, 'utf8');
}
