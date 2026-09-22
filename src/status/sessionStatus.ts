import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';

export type SessionStatus = 'running' | 'waiting' | 'done' | 'error';

export interface SessionStatusRecord {
  status: SessionStatus;
  updatedAt: number;
}

/**
 * Where per-session status files live — one small JSON file per session id, regardless of agent.
 * Written by `reportStatus.ts` (run as a Claude Code hook, outside this extension) for Claude
 * sessions, and by `status/copilotStatusWatcher.ts` (running in-process, no hook needed) for
 * Copilot ones. Physically still under `~/.claude/` for historical reasons — it predates Copilot
 * support — but the directory itself is Session Deck's own scratch cache, not part of Claude
 * Code's real config, so there was nothing to actually migrate once a second agent started writing
 * here too.
 */
export function getSessionStatusDir(): string {
  return path.join(os.homedir(), '.claude', 'session-deck-status');
}

export function ensureSessionStatusDir(): void {
  fs.mkdirSync(getSessionStatusDir(), { recursive: true });
}

/**
 * Removes a session's status file directly — for when Session Deck itself
 * notices the underlying process is gone (its terminal closed) without Claude
 * Code necessarily getting a chance to fire its own `SessionEnd` hook first
 * (e.g. the terminal was force-closed while a task was running: VS Code kills
 * the process tree outright, no graceful shutdown). Mirrors exactly what
 * `reportStatus.ts`'s `SessionEnd` handler does, so a status dot never gets
 * stuck showing "running" forever for a process that's actually dead.
 * Idempotent — a no-op if there was nothing to clear (e.g. it already exited
 * gracefully and the hook beat this to it).
 */
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

/**
 * Written directly by `terminalService.ts` (unlike most Claude status, which comes from
 * `reportStatus.ts`'s hook payloads) — Claude Code's hooks don't expose a distinct failure signal, so
 * this is inferred instead from the exit code of the `claude` command run in a session's terminal. See
 * `terminalService.ts`'s `onDidEndTerminalShellExecution` listener for the (best-effort, heuristic)
 * detection itself — still the only error signal for Claude, but for Copilot it's now a backstop behind
 * `copilotStatusWatcher.ts`'s more precise `session.error` event, which fires within the same command
 * and thus resolves first.
 */
export function markSessionError(sessionId: string): void {
  writeSessionStatus(sessionId, 'error');
}

/**
 * `undefined` means "no status" — either tracking isn't enabled, this session
 * predates it, or `reportStatus.ts` just cleared the file on `SessionEnd`
 * (the `claude` process actually exited, so there's no live state left to show).
 */
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
    // Missing file (no status yet) or a transient partial write from the hook script racing a read.
  }
  return undefined;
}

/**
 * "done" (🟢) and "error" (🔴✗) are both purely informational — something happened, sitting there for
 * you to notice — unlike "waiting" (🟡), which is a real pending request that only clears when you
 * actually resolve it (approving a permission, typing something) and thus gets a fresh hook event of
 * its own. Once you've selected such a session in the tree, there's nothing further to inform you of,
 * so it should read as "no status" (blank) until a *newer* event supersedes it — tracked here by exact
 * `updatedAt`, in memory only (deliberately not persisted: "have I looked at this since it finished" is
 * scoped to the current VS Code session).
 */
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
