import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { defaultTreePrefs, isDefaultTree, parseTreePrefs, SessionPin, TreePrefs } from './treePrefs';

/** Per-session preferences shared by every Session Deck front end (VS Code extension, terminal UI), keyed by session id. */
export interface SessionPrefs {
  /** Session Deck's own name override. Wins over the agent's own title. */
  name?: string;
  archived?: boolean;
  /** Kept at the top or bottom of its project, above/below the sort order. */
  pin?: SessionPin;
  /** `updatedAt` of the "done"/"error" status the user has seen (see `acknowledgeSessionStatus`). A newer one is unseen. */
  seenAt?: number;
  /** Free-form labels, this session only (not shared with its project's other sessions). Empty lists are dropped. */
  tags?: string[];
}

export type ThemePreference = 'dark' | 'light' | 'system';

/** Terminal UI preferences. The extension ignores these, since VS Code has its own theme and layout. */
export interface UiPrefs {
  theme?: ThemePreference;
  /** Sessions panel width, as a percentage of the terminal width. */
  sidebarPct?: number;
}

export interface DeckStateFile {
  version: 1;
  sessions: Record<string, SessionPrefs>;
  ui?: UiPrefs;
  tree?: TreePrefs;
  /**
   * Project keys removed from the list (see `DeckStore.setProjectHidden`). A session discovered later
   * for one of these keys — even one started outside Session Deck — stays off the list; the project
   * reappears only once it's added again (e.g. the terminal UI's `p`, "add a project").
   */
  hiddenProjects?: string[];
}

const THEMES: readonly ThemePreference[] = ['dark', 'light', 'system'];
export const SIDEBAR_PCT_MIN = 15;
export const SIDEBAR_PCT_MAX = 70;

/** `SESSION_DECK_HOME` overrides the location (tests, or keeping a separate state while developing). */
export function getDeckHomeDir(): string {
  return process.env.SESSION_DECK_HOME || path.join(os.homedir(), '.session-deck');
}

export function getDeckStatePath(): string {
  return path.join(getDeckHomeDir(), 'state.json');
}

function emptyState(): DeckStateFile {
  return { version: 1, sessions: {} };
}

/** `undefined` for anything that isn't a readable state file. Invalid entries are dropped rather than failing the whole file. */
export function parseDeckState(raw: string): DeckStateFile | undefined {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return undefined;
  }
  if (typeof parsed !== 'object' || parsed === null) {
    return undefined;
  }
  const { sessions, ui, tree, hiddenProjects } = parsed as {
    sessions?: unknown;
    ui?: unknown;
    tree?: unknown;
    hiddenProjects?: unknown;
  };
  const state = emptyState();
  const uiPrefs = parseUiPrefs(ui);
  if (uiPrefs) {
    state.ui = uiPrefs;
  }
  const treePrefs = parseTreePrefs(tree);
  if (treePrefs) {
    state.tree = treePrefs;
  }
  if (Array.isArray(hiddenProjects)) {
    const keys = [...new Set(hiddenProjects.filter((k): k is string => typeof k === 'string'))];
    if (keys.length) {
      state.hiddenProjects = keys;
    }
  }
  if (typeof sessions !== 'object' || sessions === null) {
    return state;
  }
  for (const [id, value] of Object.entries(sessions as Record<string, unknown>)) {
    if (typeof value !== 'object' || value === null) {
      continue;
    }
    const { name, archived, pin, seenAt, tags } = value as Record<string, unknown>;
    const prefs: SessionPrefs = {};
    if (typeof name === 'string' && name.trim()) {
      prefs.name = name;
    }
    if (archived === true) {
      prefs.archived = true;
    }
    if (pin === 'top' || pin === 'bottom') {
      prefs.pin = pin;
    }
    if (typeof seenAt === 'number' && seenAt > 0) {
      prefs.seenAt = seenAt;
    }
    if (Array.isArray(tags)) {
      const normalized = normalizeTags(tags);
      if (normalized.length) {
        prefs.tags = normalized;
      }
    }
    if (Object.keys(prefs).length) {
      state.sessions[id] = prefs;
    }
  }
  return state;
}

function parseUiPrefs(ui: unknown): UiPrefs | undefined {
  if (typeof ui !== 'object' || ui === null) {
    return undefined;
  }
  const { theme, sidebarPct } = ui as Record<string, unknown>;
  const prefs: UiPrefs = {};
  if (THEMES.includes(theme as ThemePreference)) {
    prefs.theme = theme as ThemePreference;
  }
  if (typeof sidebarPct === 'number' && sidebarPct >= SIDEBAR_PCT_MIN && sidebarPct <= SIDEBAR_PCT_MAX) {
    prefs.sidebarPct = sidebarPct;
  }
  return Object.keys(prefs).length ? prefs : undefined;
}

/** Trims, drops blanks, and dedupes (exact match) while keeping first-seen order. */
function normalizeTags(tags: readonly unknown[]): string[] {
  const seen = new Set<string>();
  const result: string[] = [];
  for (const t of tags) {
    if (typeof t !== 'string') continue;
    const trimmed = t.trim();
    if (trimmed && !seen.has(trimmed)) {
      seen.add(trimmed);
      result.push(trimmed);
    }
  }
  return result;
}

/** Pure: merges `patch` into the UI prefs. An `undefined` or out-of-range value clears that field. */
export function applyUiPatch(state: DeckStateFile, patch: Partial<UiPrefs>): DeckStateFile {
  const next: DeckStateFile = { ...state };
  delete next.ui;
  const ui = parseUiPrefs({ ...state.ui, ...patch });
  return ui ? { ...next, ui } : next;
}

/** Pure: a `undefined`/`false`/blank value in `patch` clears that field, and an entry left with no fields is removed. */
export function applySessionPatch(state: DeckStateFile, sessionId: string, patch: Partial<SessionPrefs>): DeckStateFile {
  const next: SessionPrefs = { ...state.sessions[sessionId] };
  if ('name' in patch) {
    if (patch.name && patch.name.trim()) {
      next.name = patch.name;
    } else {
      delete next.name;
    }
  }
  if ('archived' in patch) {
    if (patch.archived) {
      next.archived = true;
    } else {
      delete next.archived;
    }
  }
  if ('pin' in patch) {
    if (patch.pin) {
      next.pin = patch.pin;
    } else {
      delete next.pin;
    }
  }
  if ('seenAt' in patch) {
    if (patch.seenAt) {
      next.seenAt = patch.seenAt;
    } else {
      delete next.seenAt;
    }
  }
  if ('tags' in patch) {
    const normalized = normalizeTags(patch.tags ?? []);
    if (normalized.length) {
      next.tags = normalized;
    } else {
      delete next.tags;
    }
  }
  const sessions = { ...state.sessions };
  if (Object.keys(next).length) {
    sessions[sessionId] = next;
  } else {
    delete sessions[sessionId];
  }
  return { ...state, sessions };
}

/** Pure: adds or removes a project key from the hidden list, dropping the field entirely once it's empty. */
export function applyHiddenProjectPatch(state: DeckStateFile, projectKey: string, hidden: boolean): DeckStateFile {
  const keys = new Set(state.hiddenProjects ?? []);
  if (hidden) {
    keys.add(projectKey);
  } else {
    keys.delete(projectKey);
  }
  const next: DeckStateFile = { ...state };
  if (keys.size) {
    next.hiddenProjects = [...keys];
  } else {
    delete next.hiddenProjects;
  }
  return next;
}

const RENAME_RETRIES = 10;
const RENAME_RETRY_MS = 20;

/**
 * `~/.session-deck/state.json`, shared by the extension and the terminal UI, which can both run at
 * once. There's no lock: each write re-reads the file, applies only its own patch, and swaps the result
 * in atomically (temp file + rename), so readers never see a half-written file and at worst an edit
 * made in the same few milliseconds by the other front end is lost.
 */
export class DeckStore {
  private cache?: { mtimeMs: number; size: number; state: DeckStateFile };

  constructor(readonly filePath: string = getDeckStatePath()) {}

  /** Cached by mtime+size, so calling it per tree row costs one `stat`. */
  read(): DeckStateFile {
    let stat: fs.Stats;
    try {
      stat = fs.statSync(this.filePath);
    } catch {
      return emptyState();
    }
    if (this.cache && this.cache.mtimeMs === stat.mtimeMs && this.cache.size === stat.size) {
      return this.cache.state;
    }
    let raw: string;
    try {
      raw = fs.readFileSync(this.filePath, 'utf8');
    } catch {
      return this.cache?.state ?? emptyState();
    }
    const state = parseDeckState(raw) ?? emptyState();
    this.cache = { mtimeMs: stat.mtimeMs, size: stat.size, state };
    return state;
  }

  getSession(sessionId: string): SessionPrefs | undefined {
    return this.read().sessions[sessionId];
  }

  getUi(): UiPrefs {
    return this.read().ui ?? {};
  }

  async updateUi(patch: Partial<UiPrefs>): Promise<void> {
    await this.writeAtomic(applyUiPatch(this.readForWrite(), patch));
  }

  getTree(): TreePrefs {
    return this.read().tree ?? defaultTreePrefs();
  }

  getHiddenProjects(): string[] {
    return this.read().hiddenProjects ?? [];
  }

  /** Hides (or brings back) a project by key. See {@link DeckStateFile.hiddenProjects}. */
  async setProjectHidden(projectKey: string, hidden: boolean): Promise<void> {
    await this.writeAtomic(applyHiddenProjectPatch(this.readForWrite(), projectKey, hidden));
  }

  /** Every distinct tag in use across all sessions, alphabetically, for the tag summary at the bottom of the tree. */
  getAllTags(): string[] {
    const tags = new Set<string>();
    for (const prefs of Object.values(this.read().sessions)) {
      prefs.tags?.forEach((t) => tags.add(t));
    }
    return [...tags].sort((a, b) => a.localeCompare(b));
  }

  /** `change` gets the tree as it is on disk right now (not the cached copy), so edits from the other front end are kept. */
  async updateTree(change: (tree: TreePrefs) => TreePrefs): Promise<void> {
    const state = this.readForWrite();
    const tree = change(state.tree ?? defaultTreePrefs());
    const next: DeckStateFile = { ...state, tree };
    if (isDefaultTree(tree)) {
      delete next.tree;
    }
    await this.writeAtomic(next);
  }

  async updateSession(sessionId: string, patch: Partial<SessionPrefs>): Promise<void> {
    await this.updateSessions({ [sessionId]: patch });
  }

  /** Several patches in one write (e.g. a migration). */
  async updateSessions(patches: Record<string, Partial<SessionPrefs>>): Promise<void> {
    let state = this.readForWrite();
    for (const [id, patch] of Object.entries(patches)) {
      state = applySessionPatch(state, id, patch);
    }
    await this.writeAtomic(state);
  }

  /** Fires (debounced) whenever the file changes, from this process or another front end. */
  watch(listener: () => void): { dispose(): void } {
    const dir = path.dirname(this.filePath);
    const base = path.basename(this.filePath);
    fs.mkdirSync(dir, { recursive: true });
    let timer: NodeJS.Timeout | undefined;
    const watcher = fs.watch(dir, (_event, filename) => {
      if (filename && filename.toString() !== base) {
        return; // temp files, backups
      }
      clearTimeout(timer);
      timer = setTimeout(listener, 100);
    });
    return {
      dispose: () => {
        clearTimeout(timer);
        watcher.close();
      },
    };
  }

  /** Bypasses the cache. A file that exists but can't be parsed is kept aside as a backup instead of being overwritten. */
  private readForWrite(): DeckStateFile {
    let raw: string;
    try {
      raw = fs.readFileSync(this.filePath, 'utf8');
    } catch {
      return emptyState();
    }
    const state = parseDeckState(raw);
    if (state) {
      return state;
    }
    try {
      fs.renameSync(this.filePath, `${this.filePath}.corrupt-${Date.now()}`);
    } catch {
      // best effort; the write below replaces it either way
    }
    return emptyState();
  }

  private async writeAtomic(state: DeckStateFile): Promise<void> {
    fs.mkdirSync(path.dirname(this.filePath), { recursive: true });
    const tmp = `${this.filePath}.${process.pid}.${Date.now()}.tmp`;
    fs.writeFileSync(tmp, `${JSON.stringify(state, null, 2)}\n`, 'utf8');
    // On Windows the rename fails with EPERM/EBUSY while another process has the file open for a read.
    for (let attempt = 0; ; attempt++) {
      try {
        fs.renameSync(tmp, this.filePath);
        this.cache = undefined;
        return;
      } catch (err) {
        const code = (err as NodeJS.ErrnoException).code;
        if (attempt >= RENAME_RETRIES || (code !== 'EPERM' && code !== 'EBUSY' && code !== 'EACCES')) {
          try {
            fs.unlinkSync(tmp);
          } catch {
            // ignore
          }
          throw err;
        }
        await new Promise((resolve) => setTimeout(resolve, RENAME_RETRY_MS));
      }
    }
  }
}
