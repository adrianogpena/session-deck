import * as fs from 'fs';
import * as path from 'path';
import { isSafeSessionId } from '../commands/sessionId';
import { getSessionStatusDir, SessionStatus } from './sessionStatus';

/** Both pitago's plain notification log and z4-oriel's alert center independently settled on this cap. */
export const ALERT_LOG_CAP = 200;

/** One status change, logged whether or not it also fired a desktop toast — see `WaitingNotifier`. */
export interface AlertEntry {
  sessionId: string;
  status: SessionStatus;
  label: string;
  project?: string;
  /** The status record's own `updatedAt` — also what dedupes the same transition logged by both front ends. */
  at: number;
}

function alertsDir(): string {
  return path.join(getSessionStatusDir(), 'alerts');
}

function alertFile(sessionId: string, at: number): string {
  return path.join(alertsDir(), `${sessionId}.${at}.json`);
}

function parseAlert(dir: string, name: string): AlertEntry | undefined {
  try {
    const parsed = JSON.parse(fs.readFileSync(path.join(dir, name), 'utf8'));
    if (
      parsed &&
      typeof parsed.sessionId === 'string' &&
      typeof parsed.label === 'string' &&
      typeof parsed.at === 'number' &&
      (parsed.status === 'running' || parsed.status === 'waiting' || parsed.status === 'done' || parsed.status === 'error')
    ) {
      return {
        sessionId: parsed.sessionId,
        status: parsed.status,
        label: parsed.label,
        project: typeof parsed.project === 'string' ? parsed.project : undefined,
        at: parsed.at,
      };
    }
  } catch {
    // A transient partial write from a writer racing a read, or a file removed by a concurrent prune.
  }
  return undefined;
}

/** Keeps at most `ALERT_LOG_CAP` entries, oldest removed first. */
function prune(): void {
  const dir = alertsDir();
  let names: string[];
  try {
    names = fs.readdirSync(dir);
  } catch {
    return;
  }
  if (names.length <= ALERT_LOG_CAP) {
    return;
  }
  const entries = names
    .map((name) => ({ name, entry: parseAlert(dir, name) }))
    .filter((e): e is { name: string; entry: AlertEntry } => !!e.entry);
  entries.sort((a, b) => b.entry.at - a.entry.at);
  for (const { name } of entries.slice(ALERT_LOG_CAP)) {
    fs.rmSync(path.join(dir, name), { force: true });
  }
}

/**
 * Appends one status transition to the shared alert history, deduped by session id + `at` so the
 * extension and the terminal UI logging the very same transition only produce one entry (the atomic
 * `wx` write lets exactly one caller win). Best-effort, like the rest of the status dir: a failure
 * (already logged, or an unwritable directory) is silently dropped rather than thrown.
 */
export function appendAlert(entry: AlertEntry): void {
  if (!isSafeSessionId(entry.sessionId)) {
    return;
  }
  try {
    fs.mkdirSync(alertsDir(), { recursive: true });
    fs.writeFileSync(alertFile(entry.sessionId, entry.at), JSON.stringify(entry), { flag: 'wx' });
  } catch {
    return;
  }
  prune();
}

/** The alert history, newest first, capped at `limit`. */
export function readAlerts(limit: number = ALERT_LOG_CAP): AlertEntry[] {
  const dir = alertsDir();
  let names: string[];
  try {
    names = fs.readdirSync(dir);
  } catch {
    return [];
  }
  const entries = names.map((name) => parseAlert(dir, name)).filter((e): e is AlertEntry => !!e);
  entries.sort((a, b) => b.at - a.at);
  return entries.slice(0, limit);
}
