import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { DatabaseSync } from 'node:sqlite';

/** The `copilot` CLI's own home directory — session history lives in `session-store.db` here; there's no `copilot sessions list` command, so reading it directly is the only way to enumerate sessions. */
export function getCopilotHomeDir(): string {
  return path.join(os.homedir(), '.copilot');
}

function getSessionStorePath(): string {
  return path.join(getCopilotHomeDir(), 'session-store.db');
}

/** Copilot CLI's own live per-session activity log. Internal, undocumented format — treat its shape as best-effort, not a stable contract. */
export function getCopilotSessionEventsLogPath(sessionId: string): string {
  return path.join(getCopilotHomeDir(), 'session-state', sessionId, 'events.jsonl');
}

export function copilotStoreExists(): boolean {
  return fs.existsSync(getSessionStorePath());
}

export interface CopilotSessionRow {
  id: string;
  cwd: string;
  repository: string | null;
  branch: string | null;
  summary: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface CopilotTurn {
  turnIndex: number;
  userMessage: string | null;
  assistantResponse: string | null;
  timestamp: string;
}

/** Fresh read-only connection per call, not cached: `copilot` writes this DB in WAL mode, so a stale connection could miss newer sessions. Must open the real path in place, not a copy — a copy misses rows still sitting in the `-wal` file. */
function withDb<T>(fn: (db: DatabaseSync) => T): T {
  const db = new DatabaseSync(getSessionStorePath(), { readOnly: true });
  try {
    return fn(db);
  } finally {
    db.close();
  }
}

/**
 * Every real Copilot CLI session, newest first. "Real" excludes a session with no prompt yet:
 * unlike Claude Code, Copilot inserts a `sessions` row immediately at startup, before anything is
 * typed. Filtered via `copilotSessionHasStarted` (events.jsonl), not a SQL condition on `turns` —
 * `turns` only gets written once the `copilot` process exits, so it's not a live signal.
 */
export function listCopilotSessions(): CopilotSessionRow[] {
  if (!copilotStoreExists()) {
    return [];
  }
  const rows = withDb((db) => {
    const result = db
      .prepare(
        'SELECT id, cwd, repository, branch, summary, created_at AS createdAt, updated_at AS updatedAt FROM sessions ORDER BY updated_at DESC'
      )
      .all();
    return result as unknown as CopilotSessionRow[];
  });
  return rows.filter((row) => copilotSessionHasStarted(row.id));
}

/** Sessions confirmed to have a real `user.message` event — cached permanently (monotonic fact, never re-checked or cleared) to avoid re-reading a long session's event log on every refresh. */
const sessionsConfirmedStarted = new Set<string>();

function copilotSessionHasStarted(sessionId: string): boolean {
  if (sessionsConfirmedStarted.has(sessionId)) {
    return true;
  }
  let content: string;
  try {
    content = fs.readFileSync(getCopilotSessionEventsLogPath(sessionId), 'utf8');
  } catch {
    return false; // No events file yet (or unreadable) — nothing has happened in this session yet.
  }
  const hasUserMessage = content
    .split('\n')
    .some((line) => {
      const trimmed = line.trim();
      if (!trimmed) {
        return false;
      }
      try {
        return (JSON.parse(trimmed) as { type?: unknown }).type === 'user.message';
      } catch {
        return false; // Tolerate a partial trailing line mid-write.
      }
    });
  if (hasUserMessage) {
    sessionsConfirmedStarted.add(sessionId);
  }
  return hasUserMessage;
}

/** A session's turns in order — for rendering a read-only "transcript" the same way `claudeStorage.ts`'s functions do for Claude. */
export function listCopilotTurns(sessionId: string): CopilotTurn[] {
  if (!copilotStoreExists()) {
    return [];
  }
  return withDb((db) => {
    const rows = db
      .prepare(
        'SELECT turn_index AS turnIndex, user_message AS userMessage, assistant_response AS assistantResponse, timestamp FROM turns WHERE session_id = ? ORDER BY turn_index'
      )
      .all(sessionId);
    return rows as unknown as CopilotTurn[];
  });
}

/** Every turn's user/assistant text concatenated, for "Search Sessions" — the Copilot analogue of `claudeStorage.ts`'s `readSessionSearchText`. */
export function copilotSessionSearchText(sessionId: string): string {
  const parts: string[] = [];
  for (const turn of listCopilotTurns(sessionId)) {
    if (turn.userMessage) {
      parts.push(turn.userMessage);
    }
    if (turn.assistantResponse) {
      parts.push(turn.assistantResponse);
    }
  }
  return parts.join('\n');
}

/** For "Copy Last Response" — the most recent turn with a non-empty assistant reply, last-to-first so a trailing empty/pending turn doesn't shadow the real last answer. */
export function lastCopilotAssistantResponse(sessionId: string): string | undefined {
  const turns = listCopilotTurns(sessionId);
  for (let i = turns.length - 1; i >= 0; i--) {
    const response = turns[i].assistantResponse;
    if (response) {
      return response;
    }
  }
  return undefined;
}
