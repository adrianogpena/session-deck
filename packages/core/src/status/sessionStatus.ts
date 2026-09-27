import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';
import { DeckStore } from '../store/deckStore';

export type SessionStatus = 'running' | 'waiting' | 'done' | 'error';

export interface SessionStatusRecord {
  status: SessionStatus;
  updatedAt: number;
}

/** One small JSON status file per session id, regardless of agent — Session Deck's own scratch cache, not part of Claude Code's real config despite the path. */
export function getSessionStatusDir(): string {
  // SESSION_DECK_STATUS_DIR: tests (and a separate development setup) keep off the real directory.
  return process.env.SESSION_DECK_STATUS_DIR || path.join(os.homedir(), '.claude', 'session-deck-status');
}

export function ensureSessionStatusDir(): void {
  fs.mkdirSync(getSessionStatusDir(), { recursive: true });
}

/** Removes a session's status file. Idempotent — a no-op if there's nothing to clear. */
export function clearSessionStatus(sessionId: string): void {
  const dir = getSessionStatusDir();
  try {
    fs.rmSync(path.join(dir, `${sessionId}.json`), { force: true });
    for (const name of fs.readdirSync(dir)) {
      if (name.startsWith(`${sessionId}.`) && name.endsWith('.notified')) {
        fs.rmSync(path.join(dir, name), { force: true }); // see claimNotification
      }
    }
  } catch {
    // Best-effort.
  }
}

/** Writes a session's status directly — the general case both `markSessionError` and `copilotStatusWatcher.ts` build on. */
export function writeSessionStatus(sessionId: string, status: SessionStatus): void {
  ensureSessionStatusDir();
  const filePath = path.join(getSessionStatusDir(), `${sessionId}.json`);
  fs.writeFileSync(filePath, JSON.stringify({ status, updatedAt: Date.now() }), 'utf8');
}

/** Inferred from the terminal command's exit code (see `terminalService.ts`) — neither agent's live status has a distinct "error" value. */
export function markSessionError(sessionId: string): void {
  writeSessionStatus(sessionId, 'error');
}

/** `undefined` means "no status" — nothing's happened yet, or the process already exited. */
export function readSessionStatus(sessionId: string): SessionStatusRecord | undefined {
  const filePath = path.join(getSessionStatusDir(), `${sessionId}.json`);
  try {
    const parsed = JSON.parse(fs.readFileSync(filePath, 'utf8'));
    if (
      parsed &&
      (parsed.status === 'running' || parsed.status === 'waiting' || parsed.status === 'done' || parsed.status === 'error')
    ) {
      return { status: parsed.status, updatedAt: typeof parsed.updatedAt === 'number' ? parsed.updatedAt : 0 };
    }
  } catch {
    // Missing file (no status yet) or a transient partial write from a watcher racing a read.
  }
  return undefined;
}

let store: DeckStore | undefined;

/** Created on first use, so a `SESSION_DECK_HOME` set beforehand (tests, dev setup) is honored. */
function deckStore(): DeckStore {
  if (!store) {
    store = new DeckStore();
  }
  return store;
}

/** "done" and "error" wait to be seen; "running" and "waiting" are current states, never "seen". */
function needsSeeing(status: SessionStatusRecord | undefined): status is SessionStatusRecord {
  return status?.status === 'done' || status?.status === 'error';
}

/**
 * Marks the session's current "done"/"error" as seen, by its exact `updatedAt`. Kept in the shared
 * store, so it survives restarts and both front ends agree. "waiting" isn't tracked: it only clears
 * via a real new status write.
 */
export async function acknowledgeSessionStatus(sessionId: string): Promise<void> {
  const status = readSessionStatus(sessionId);
  if (needsSeeing(status) && deckStore().getSession(sessionId)?.seenAt !== status.updatedAt) {
    await deckStore().updateSession(sessionId, { seenAt: status.updatedAt });
  }
}

/** Whether the session has a "done"/"error" the user hasn't seen yet. */
export function isSessionStatusUnseen(sessionId: string): boolean {
  const status = readSessionStatus(sessionId);
  return needsSeeing(status) && deckStore().getSession(sessionId)?.seenAt !== status.updatedAt;
}

/**
 * "Mark as unread": a seen "done"/"error" becomes unseen again. A session with no status at all gets
 * a fresh "done"; one that's running or waiting is already asking for attention, so it's left alone.
 */
export async function markSessionUnseen(sessionId: string): Promise<void> {
  const status = readSessionStatus(sessionId);
  if (needsSeeing(status)) {
    await deckStore().updateSession(sessionId, { seenAt: undefined });
  } else if (!status) {
    writeSessionStatus(sessionId, 'done');
  }
}

/** What the UI should actually display: a seen "done"/"error" reads as no status. */
export function readEffectiveSessionStatus(sessionId: string): SessionStatusRecord | undefined {
  const status = readSessionStatus(sessionId);
  if (needsSeeing(status) && deckStore().getSession(sessionId)?.seenAt === status.updatedAt) {
    return undefined;
  }
  return status;
}

/**
 * Claims the right to notify about this exact status (`updatedAt`), once across every process: the
 * extension and the terminal UI can both be watching the same session. Creating the marker file is
 * atomic (`wx`), so exactly one caller gets `true`. Older markers for the session are removed.
 */
export function claimNotification(sessionId: string, updatedAt: number): boolean {
  ensureSessionStatusDir();
  const dir = getSessionStatusDir();
  const marker = `${sessionId}.${updatedAt}.notified`;
  try {
    fs.closeSync(fs.openSync(path.join(dir, marker), 'wx'));
  } catch {
    return false; // already claimed (or the directory is unwritable: better silent than duplicated)
  }
  try {
    for (const name of fs.readdirSync(dir)) {
      if (name.startsWith(`${sessionId}.`) && name.endsWith('.notified') && name !== marker) {
        fs.rmSync(path.join(dir, name), { force: true });
      }
    }
  } catch {
    // best effort
  }
  return true;
}
