import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import {
  decodeProjectPath,
  DeckStore,
  getClaudeProjectsDir,
  listProjectDirNames,
  listSessionFiles,
  readSessionMeta,
} from '@session-deck/core';
import { oneLine } from './ansi';
import type { LiveSession } from './liveSession';

const MAX_SESSIONS = 30;

export interface DeckSession {
  /** `null` until a brand-new session's id shows up in Claude's pid file. */
  id: string | null;
  file?: string;
  cwd: string;
  /** Claude's own title (`/rename`, AI title) or the first prompt. See {@link displayTitle} for what's shown. */
  title: string;
  mtime: number;
  live?: LiveSession;
  /** `undefined` = not loaded, `null` = loading. */
  lastResponse?: string | null;
  titleRefreshing?: boolean;
}

export type SidebarRow = { kind: 'group'; label: string } | { kind: 'session'; session: DeckSession };

export type SessionStatus = 'running' | 'waiting' | 'idle' | 'starting' | 'exited' | 'elsewhere' | 'stopped';

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
    found.push({ id: f.id, file: f.full, cwd: meta.cwd || decodeProjectPath(f.dir), title: oneLine(meta.title || meta.firstPrompt), mtime: f.mtime });
  }

  const liveOnes = current.filter((s) => s.live);
  for (const s of found) {
    const existing = liveOnes.find((l) => l.id === s.id);
    if (existing) {
      Object.assign(existing, { title: s.title, file: s.file, mtime: s.mtime });
    }
  }
  return [...liveOnes, ...found.filter((s) => !liveOnes.some((l) => l.id === s.id))];
}

/** Grouped by working folder, in first-seen (most recent) order. */
export function buildRows(sessions: DeckSession[]): SidebarRow[] {
  const groups = new Map<string, { label: string; items: DeckSession[] }>();
  for (const s of sessions) {
    const key = s.cwd.toLowerCase();
    let group = groups.get(key);
    if (!group) {
      group = { label: path.basename(s.cwd) || s.cwd, items: [] };
      groups.set(key, group);
    }
    group.items.push(s);
  }
  const rows: SidebarRow[] = [];
  for (const g of groups.values()) {
    rows.push({ kind: 'group', label: g.label });
    for (const s of g.items) {
      rows.push({ kind: 'session', session: s });
    }
  }
  return rows;
}

function isAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

/** Every live `claude` process on the machine, from Claude's own `~/.claude/sessions/<pid>.json` files. */
export class ClaudeProcs {
  private bySession = new Map<string, ClaudeProc>();
  private byPid = new Map<number, ClaudeProc>();

  poll(): void {
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
    if (s.live) {
      if (s.live.exited) {
        return 'exited';
      }
      const rec = this.byPid.get(s.live.pid);
      if (!rec) {
        return 'starting';
      }
      return ({ busy: 'running', shell: 'running', waiting: 'waiting', idle: 'idle' } as Record<string, SessionStatus>)[rec.status] || 'idle';
    }
    if (s.id && this.bySession.has(s.id)) {
      return 'elsewhere';
    }
    return 'stopped';
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
