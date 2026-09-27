import * as fs from 'fs';
import * as path from 'path';
import { getDeckHomeDir } from './deckStore';

/** A deleted session, kept restorable in `~/.session-deck/trash/` until it's older than {@link TRASH_TTL_MS}. */
export interface TrashedSession {
  sessionId: string;
  agent: 'claude';
  title: string;
  /** Where the transcript lived, so it can go back there. */
  originalPath: string;
  trashedAt: number;
}

export const TRASH_TTL_MS = 30 * 24 * 60 * 60 * 1000;

export function getTrashDir(): string {
  return path.join(getDeckHomeDir(), 'trash');
}

function manifestPath(): string {
  return path.join(getTrashDir(), 'manifest.json');
}

/** Claude keeps a folder named after the session next to some transcripts (tool results, subagent logs). */
function companionDir(transcriptPath: string): string {
  return transcriptPath.replace(/\.jsonl$/, '');
}

function trashedTranscript(sessionId: string): string {
  return path.join(getTrashDir(), `${sessionId}.jsonl`);
}

/** Newest first. Tolerant of a missing or damaged manifest (reads as empty). */
export function listTrash(): TrashedSession[] {
  try {
    const parsed = JSON.parse(fs.readFileSync(manifestPath(), 'utf8'));
    return (Array.isArray(parsed) ? parsed : [])
      .filter((e): e is TrashedSession => typeof e?.sessionId === 'string' && typeof e?.originalPath === 'string' && typeof e?.trashedAt === 'number')
      .sort((a, b) => b.trashedAt - a.trashedAt);
  } catch {
    return [];
  }
}

function writeTrash(entries: TrashedSession[]): void {
  fs.mkdirSync(getTrashDir(), { recursive: true });
  const tmp = `${manifestPath()}.${process.pid}.tmp`;
  fs.writeFileSync(tmp, `${JSON.stringify(entries, null, 2)}\n`, 'utf8');
  fs.renameSync(tmp, manifestPath());
}

/** Rename, falling back to copy + remove across drives. */
function move(from: string, to: string): void {
  fs.mkdirSync(path.dirname(to), { recursive: true });
  try {
    fs.renameSync(from, to);
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code !== 'EXDEV') {
      throw err;
    }
    fs.cpSync(from, to, { recursive: true });
    fs.rmSync(from, { recursive: true, force: true });
  }
}

/** Moves a Claude session's transcript (and its companion folder, if any) into the trash. */
export function trashClaudeSession(transcriptPath: string, sessionId: string, title: string): TrashedSession {
  const entry: TrashedSession = { sessionId, agent: 'claude', title, originalPath: transcriptPath, trashedAt: Date.now() };
  move(transcriptPath, trashedTranscript(sessionId));
  if (fs.existsSync(companionDir(transcriptPath))) {
    move(companionDir(transcriptPath), path.join(getTrashDir(), sessionId));
  }
  writeTrash([entry, ...listTrash().filter((e) => e.sessionId !== sessionId)]);
  return entry;
}

/** Puts a trashed session back where it was. Refuses (throws) rather than overwrite a transcript that's there again. */
export function restoreSession(sessionId: string): TrashedSession {
  const entries = listTrash();
  const entry = entries.find((e) => e.sessionId === sessionId);
  if (!entry) {
    throw new Error('That session is no longer in the trash.');
  }
  if (fs.existsSync(entry.originalPath)) {
    throw new Error('A transcript with the same id already exists where this one was.');
  }
  move(trashedTranscript(sessionId), entry.originalPath);
  const companion = path.join(getTrashDir(), sessionId);
  if (fs.existsSync(companion)) {
    move(companion, companionDir(entry.originalPath));
  }
  writeTrash(entries.filter((e) => e !== entry));
  return entry;
}

/** Permanently removes trashed sessions older than `ttlMs`. Returns how many were removed. */
export function purgeTrash(nowMs: number = Date.now(), ttlMs: number = TRASH_TTL_MS): number {
  const entries = listTrash();
  const expired = entries.filter((e) => nowMs - e.trashedAt > ttlMs);
  if (expired.length === 0) {
    return 0;
  }
  for (const e of expired) {
    fs.rmSync(trashedTranscript(e.sessionId), { force: true });
    fs.rmSync(path.join(getTrashDir(), e.sessionId), { recursive: true, force: true });
  }
  writeTrash(entries.filter((e) => !expired.includes(e)));
  return expired.length;
}
