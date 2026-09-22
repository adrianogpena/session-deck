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

/**
 * Copilot CLI's own live per-session activity log — appended to unconditionally for every session
 * (used for its own IDE/resume support), independent of the `hooks` config entirely. `status/copilotStatusWatcher.ts`
 * tails this for live status instead of requiring Copilot's opt-in hooks system, since this needs no setup.
 * An internal, undocumented format (not the public `hooks` API) — confirmed by reading the CLI's own bundled
 * `session-events.schema.json` — so treat its shape as best-effort, not a stable contract.
 */
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

/**
 * Every *real* Copilot CLI session across every project, newest first — the Copilot analogue of
 * `claudeStorage.ts`'s directory scan, and the input `sessionProvider.ts` groups by resolved git
 * root. "Real" excludes a session with no actual prompt yet: unlike Claude Code (which, per
 * observed behavior, writes nothing discoverable until a session's first real prompt), Copilot CLI
 * inserts a session's `sessions` row immediately at startup — confirmed live: creating a new
 * session via "+ New Session" made it appear in the tree instantly, before typing anything, which
 * is exactly the "opened by mistake and closed" case Claude Code's own later-write timing already
 * avoids for free. Filtering here (once) rather than per-caller keeps that behavior symmetric with
 * Claude's for every consumer — the main tree, the archive cap, the Explorer view, search — since
 * this is the only place any of them ever list Copilot sessions from (confirmed: nothing else calls
 * this function).
 *
 * The check itself is `copilotSessionHasStarted`, not a SQL condition on `sessions`/`turns` —
 * `turns` turned out to not be a live signal at all: confirmed live, a session that had already had
 * a real prompt answered still had zero rows in `turns` for it until the `copilot` process was
 * actually exited, so filtering on it left a session invisible for the entire time it was actually
 * being used (the opposite of the goal). `events.jsonl` (already proven live — status tracking
 * depends on it updating in real time) is the reliable signal instead.
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

/**
 * Once a session's `events.jsonl` is confirmed to contain a real `user.message` event, it's
 * remembered permanently rather than re-checked on every call — this is a monotonic fact (a
 * session that's been used once stays used forever), so re-reading its (potentially large, for a
 * long-running session) event log on every tree refresh would be pure waste. A session not yet
 * confirmed stays cheap to (re-)check too, since an unused session's log is still small. Never
 * cleared by "Session Deck: Refresh Sessions" for the same reason: there is nothing here that could
 * have gone stale and need a fresh read, unlike the mtime-keyed caches in `claudeStorage.ts`.
 */
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
        return false; // Tolerate a partial trailing line mid-write — best-effort, matching copilotStatusWatcher.ts's tolerance for the same file.
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
