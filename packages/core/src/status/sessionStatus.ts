import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';

export type SessionStatus = 'running' | 'waiting' | 'done' | 'error';

export interface SessionStatusRecord {
  status: SessionStatus;
  updatedAt: number;
}

/** One small JSON status file per session id, regardless of agent — Session Deck's own scratch cache, not part of Claude Code's real config despite the path. */
export function getSessionStatusDir(): string {
  return path.join(os.homedir(), '.claude', 'session-deck-status');
}

export function ensureSessionStatusDir(): void {
  fs.mkdirSync(getSessionStatusDir(), { recursive: true });
}

/** Removes a session's status file. Idempotent — a no-op if there's nothing to clear. */
export function clearSessionStatus(sessionId: string): void {
  try {
    fs.rmSync(path.join(getSessionStatusDir(), `${sessionId}.json`), { force: true });
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

/** "done"/"error" are one-time notifications: selecting the session clears the dot, tracked here by exact `updatedAt`, in memory only (not persisted across VS Code restarts). "waiting" isn't tracked — it only clears via a real new status write. */
const acknowledgedAt = new Map<string, number>();

export function acknowledgeSessionStatus(sessionId: string): void {
  const status = readSessionStatus(sessionId);
  if (status?.status === 'done' || status?.status === 'error') {
    acknowledgedAt.set(sessionId, status.updatedAt);
  }
}

/** What the UI should actually display — see {@link acknowledgeSessionStatus}. */
export function readEffectiveSessionStatus(sessionId: string): SessionStatusRecord | undefined {
  const status = readSessionStatus(sessionId);
  if ((status?.status === 'done' || status?.status === 'error') && acknowledgedAt.get(sessionId) === status.updatedAt) {
    return undefined;
  }
  return status;
}
