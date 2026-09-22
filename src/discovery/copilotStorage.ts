import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { DatabaseSync } from 'node:sqlite';

/**
 * The `copilot` CLI's own home directory (`~/.copilot`) — a standalone tool, independent of VS Code,
 * exactly like `~/.claude` for Claude Code. Its session history lives in a SQLite database here, not
 * loose files: there's no `copilot sessions list` command (checked `copilot --help` in full; `sessions`
 * only has an `import` subcommand), so reading `session-store.db` directly is the only way to enumerate
 * sessions at all.
 */
export function getCopilotHomeDir(): string {
  return path.join(os.homedir(), '.copilot');
}

function getSessionStorePath(): string {
  return path.join(getCopilotHomeDir(), 'session-store.db');
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

/**
 * Opens a fresh, read-only connection per call rather than keeping one open — mirrors
 * `claudeStorage.ts`'s "just re-read, no persistent handle" approach, and matters more here: the actual
 * `copilot` CLI writes this database in WAL mode, so a stale/cached connection could miss sessions
 * written after it was opened. `readOnly: true` still sees WAL-mode writes from another process (SQLite
 * supports concurrent readers); confirmed directly — a *copy* of just the `.db` file misses rows still
 * sitting in the `-wal` file, but opening the real path in place (not a copy) picks them up correctly.
 */
function withDb<T>(fn: (db: DatabaseSync) => T): T {
  const db = new DatabaseSync(getSessionStorePath(), { readOnly: true });
  try {
    return fn(db);
  } finally {
    db.close();
  }
}

/** Every Copilot CLI session across every project, newest first — the Copilot analogue of `claudeStorage.ts`'s directory scan, and the input `sessionProvider.ts` groups by resolved git root. */
export function listCopilotSessions(): CopilotSessionRow[] {
  if (!copilotStoreExists()) {
    return [];
  }
  return withDb((db) => {
    const rows = db
      .prepare(
        'SELECT id, cwd, repository, branch, summary, created_at AS createdAt, updated_at AS updatedAt FROM sessions ORDER BY updated_at DESC'
      )
      .all();
    return rows as unknown as CopilotSessionRow[];
  });
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
