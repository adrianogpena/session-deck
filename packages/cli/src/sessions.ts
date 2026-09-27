import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import {
  decodeProjectPath,
  DeckStore,
  getClaudeProjectsDir,
  getSessionStatusDir,
  isSessionStatusUnseen,
  listProjectDirNames,
  listSessionFiles,
  mapWithConcurrency,
  normalizeFsPath,
  readSessionMeta,
  readSessionStatus,
  resolveProjectRoot,
} from '@session-deck/core';
import { oneLine } from './ansi';
import type { StatusCategory } from './filters';
import type { LiveSession } from './liveSession';

const MAX_SESSIONS = 30;

export interface DeckSession {
  /** `null` until a brand-new session's id shows up in Claude's pid file. */
  id: string | null;
  file?: string;
  cwd: string;
  /** Git root of `cwd` (worktrees and subfolders share one), and its `normalizeFsPath` key, as in the extension. */
  projectRoot: string;
  projectKey: string;
  /** Claude's own title (`/rename`, AI title) or the first prompt. See {@link displayTitle} for what's shown. */
  title: string;
  mtime: number;
  live?: LiveSession;
  /** `undefined` = not loaded, `null` = loading. */
  lastResponse?: string | null;
  titleRefreshing?: boolean;
}

/**
 * For a session open in another terminal, the status of that other process (see {@link ClaudeProcs.isElsewhere}).
 * `done`: finished a turn the user hasn't seen yet. `error`: an error only visible on the screen
 * (e.g. a failed sign-in). `exited`: the agent process ended with an error.
 */
export type SessionStatus = 'running' | 'waiting' | 'done' | 'idle' | 'starting' | 'error' | 'exited' | 'stopped';

const CLAUDE_STATUS: Record<string, SessionStatus> = { busy: 'running', shell: 'running', waiting: 'waiting', idle: 'idle' };

interface ClaudeProc {
  pid: number;
  sessionId: string;
  status: string;
}

/** A Session Deck name override (set in either front end) wins over Claude's own title. */
export function displayTitle(s: DeckSession, store: DeckStore): string {
  return (s.id && store.getSession(s.id)?.name) || s.title;
}

/** The most recent Claude sessions on disk that have a prompt and aren't archived, merged with the live ones (kept, with their PTYs). */
export async function discoverSessions(current: DeckSession[], store: DeckStore): Promise<DeckSession[]> {
  const files: { full: string; dir: string; id: string; mtime: number }[] = [];
  for (const dir of listProjectDirNames()) {
    for (const f of listSessionFiles(dir)) {
      const full = path.join(getClaudeProjectsDir(), dir, f);
      try {
        files.push({ full, dir, id: f.slice(0, -'.jsonl'.length), mtime: fs.statSync(full).mtimeMs });
      } catch {
        // vanished mid-scan
      }
    }
  }
  files.sort((a, b) => b.mtime - a.mtime);

  const found: DeckSession[] = [];
  for (const f of files) {
    if (found.length >= MAX_SESSIONS) {
      break;
    }
    if (store.getSession(f.id)?.archived) {
      continue;
    }
    const meta = await readSessionMeta(f.full);
    if (!meta.firstPrompt) {
      continue; // empty/aborted sessions
    }
    const cwd = meta.cwd || decodeProjectPath(f.dir);
    found.push({ id: f.id, file: f.full, cwd, projectRoot: cwd, projectKey: normalizeFsPath(cwd), title: oneLine(meta.title || meta.firstPrompt), mtime: f.mtime });
  }
  await assignProjects(found);

  // Known sessions are updated in place, not replaced: the UI tracks the selection (and the previous
  // session) by object, and live ones carry their PTY. Live sessions not found on disk (new, or older
  // than the cutoff) stay listed.
  const byId = new Map(current.filter((s) => s.id).map((s) => [s.id, s]));
  const merged = found.map((s) => {
    const existing = byId.get(s.id);
    if (!existing) {
      return s;
    }
    if (existing.mtime !== s.mtime && !existing.live) {
      existing.lastResponse = undefined; // transcript changed since it was loaded
    }
    const { title, file, mtime, cwd, projectRoot, projectKey } = s;
    return Object.assign(existing, { title, file, mtime, cwd, projectRoot, projectKey });
  });
  return [...current.filter((s) => s.live && !merged.includes(s)), ...merged];
}

const GIT_RESOLVE_CONCURRENCY = 8;

/** Sets each session's project to the git root of its cwd (cached per cwd by `resolveProjectRoot`). */
async function assignProjects(sessions: DeckSession[]): Promise<void> {
  const roots = await mapWithConcurrency(sessions, GIT_RESOLVE_CONCURRENCY, (s) => resolveProjectRoot(s.cwd));
  sessions.forEach((s, i) => {
    s.projectRoot = roots[i].root;
    s.projectKey = normalizeFsPath(roots[i].root);
  });
}

function isAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

/**
 * Every live `claude` process on the machine, from Claude's own `~/.claude/sessions/<pid>.json` files,
 * plus which sessions have an unseen "done"/"error" (Session Deck's status files + the shared seen
 * marks). Both are read once per {@link poll}, not on every frame.
 */
export class ClaudeProcs {
  private bySession = new Map<string, ClaudeProc>();
  private byPid = new Map<number, ClaudeProc>();
  private unseen = new Map<string, 'done' | 'error'>();

  poll(): void {
    this.pollUnseen();
    this.pollProcesses();
  }

  private pollUnseen(): void {
    this.unseen = new Map();
    let names: string[];
    try {
      names = fs.readdirSync(getSessionStatusDir()).filter((n) => n.endsWith('.json'));
    } catch {
      return; // no status written yet
    }
    for (const name of names) {
      const id = name.slice(0, -'.json'.length);
      const status = readSessionStatus(id)?.status;
      if ((status === 'done' || status === 'error') && isSessionStatusUnseen(id)) {
        this.unseen.set(id, status);
      }
    }
  }

  private pollProcesses(): void {
    const dir = path.join(os.homedir(), '.claude', 'sessions');
    this.bySession = new Map();
    this.byPid = new Map();
    let names: string[];
    try {
      names = fs.readdirSync(dir).filter((n) => n.endsWith('.json'));
    } catch {
      names = [];
    }
    for (const n of names) {
      try {
        const p = JSON.parse(fs.readFileSync(path.join(dir, n), 'utf8'));
        if (typeof p.pid === 'number' && typeof p.sessionId === 'string' && isAlive(p.pid)) {
          const rec = { pid: p.pid, sessionId: p.sessionId, status: String(p.status) };
          this.bySession.set(p.sessionId, rec);
          this.byPid.set(p.pid, rec);
        }
      } catch {
        // partial write
      }
    }
  }

  forPid(pid: number): ClaudeProc | undefined {
    return this.byPid.get(pid);
  }

  statusOf(s: DeckSession): SessionStatus {
    const unseen = s.id ? this.unseen.get(s.id) : undefined;
    // An idle agent whose last turn the user hasn't seen yet is "done", i.e. waiting for a look.
    const fromProcess = (rec: ClaudeProc): SessionStatus => {
      const status = CLAUDE_STATUS[rec.status] || 'idle';
      return status === 'idle' && unseen === 'done' ? 'done' : status;
    };
    if (s.live) {
      if (s.live.exited) {
        return 'exited';
      }
      if (s.live.screenError) {
        return 'error';
      }
      const rec = this.byPid.get(s.live.pid);
      return rec ? fromProcess(rec) : 'starting';
    }
    const external = s.id ? this.bySession.get(s.id) : undefined;
    if (external) {
      return fromProcess(external);
    }
    return unseen === 'done' ? 'done' : unseen === 'error' ? 'exited' : 'stopped';
  }

  /** Not running here, but a `claude` process elsewhere (another terminal, VS Code) has it open. */
  isElsewhere(s: DeckSession): boolean {
    return !s.live && !!s.id && this.bySession.has(s.id);
  }

  /** "done" counts as waiting (it's waiting for a look); a screen error or an error exit counts as error. */
  categoryOf(s: DeckSession): StatusCategory {
    const status = this.statusOf(s);
    switch (status) {
      case 'starting':
        return 'running';
      case 'done':
        return 'waiting';
      case 'exited':
        return 'error';
      default:
        return status;
    }
  }
}

/** Live sessions get re-titled as Claude updates its AI title. `readSessionMeta` is mtime-cached, so an unchanged transcript costs one stat. */
export function refreshLiveTitle(s: DeckSession, onChange: () => void): void {
  if (!s.id || s.titleRefreshing) {
    return;
  }
  if (!s.file) {
    const id = s.id;
    const dir = listProjectDirNames().find((d) => fs.existsSync(path.join(getClaudeProjectsDir(), d, `${id}.jsonl`)));
    if (!dir) {
      return; // brand-new session, nothing written yet
    }
    s.file = path.join(getClaudeProjectsDir(), dir, `${id}.jsonl`);
  }
  try {
    s.mtime = fs.statSync(s.file).mtimeMs; // for the "5m ago" column
  } catch {
    // vanished; keep the last known time
  }
  s.titleRefreshing = true;
  readSessionMeta(s.file)
    .then((meta) => {
      const title = meta.title || meta.firstPrompt;
      if (title && oneLine(title) !== s.title) {
        s.title = oneLine(title);
        onChange();
      }
    })
    .finally(() => {
      s.titleRefreshing = false;
    });
}

/**
 * The records Claude's own `/rename` writes, for a session that isn't running: `custom-title` is what
 * `/resume` and the extension list, `agent-name` is what a resumed Claude shows on its input box
 * border. Writing only the first leaves the old name there.
 */
export function appendRenameRecords(file: string, sessionId: string, name: string): void {
  let prefix = '';
  try {
    const fd = fs.openSync(file, 'r');
    try {
      const { size } = fs.fstatSync(fd);
      const last = Buffer.alloc(1);
      if (size > 0 && fs.readSync(fd, last, 0, 1, size - 1) === 1 && last[0] !== 0x0a) {
        prefix = '\n'; // never glue our record onto a partial last line
      }
    } finally {
      fs.closeSync(fd);
    }
  } catch {
    // unreadable: append anyway, the write below reports real errors
  }
  const records = [
    { type: 'custom-title', customTitle: name, sessionId },
    { type: 'agent-name', agentName: name, sessionId },
  ];
  fs.appendFileSync(file, prefix + records.map((r) => `${JSON.stringify(r)}\n`).join(''));
}
