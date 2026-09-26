import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

/** Per-session preferences shared by every Session Deck front end (VS Code extension, terminal UI), keyed by session id. */
export interface SessionPrefs {
  /** Session Deck's own name override. Wins over the agent's own title. */
  name?: string;
  archived?: boolean;
}

export interface DeckStateFile {
  version: 1;
  sessions: Record<string, SessionPrefs>;
}

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
  const sessions = (parsed as { sessions?: unknown }).sessions;
  const state = emptyState();
  if (typeof sessions !== 'object' || sessions === null) {
    return state;
  }
  for (const [id, value] of Object.entries(sessions as Record<string, unknown>)) {
    if (typeof value !== 'object' || value === null) {
      continue;
    }
    const { name, archived } = value as Record<string, unknown>;
    const prefs: SessionPrefs = {};
    if (typeof name === 'string' && name.trim()) {
      prefs.name = name;
    }
    if (archived === true) {
      prefs.archived = true;
    }
    if (Object.keys(prefs).length) {
      state.sessions[id] = prefs;
    }
  }
  return state;
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
  const sessions = { ...state.sessions };
  if (Object.keys(next).length) {
    sessions[sessionId] = next;
  } else {
    delete sessions[sessionId];
  }
  return { ...state, sessions };
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
