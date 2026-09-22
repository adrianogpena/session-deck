import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { clearSessionStatus, writeSessionStatus, SessionStatus } from './sessionStatus';

/**
 * Claude Code's own live per-process status directory — one small JSON file per running `claude`
 * process, named `<pid>.json`, containing (among other fields) `sessionId`, `status`, and
 * `statusUpdatedAt`. Confirmed live against this very extension-development session's own process:
 * its file's `status` field and `statusUpdatedAt` timestamp changed in real time as tool calls
 * happened, with no user prompt submitted in between. An internal, undocumented format (not
 * Claude Code's public `hooks` feature) — the exact status enum (`"busy"`/`"shell"`/`"idle"`/
 * `"waiting"`) was found by string-searching the compiled `claude.exe` binary itself, since there's
 * no public schema for it the way Copilot ships `session-events.schema.json`. Treat its shape as
 * best-effort, not a stable contract, same caveat as `copilotStorage.ts`'s equivalent.
 */
export function getClaudeSessionsStateDir(): string {
  return path.join(os.homedir(), '.claude', 'sessions');
}

/**
 * `busy`/`shell` (running a shell command mid-turn) both map to "running" — `SessionStatus` has no
 * finer distinction than Claude's own hook-based tracking had either. `waiting` is a genuine,
 * precise signal for "needs your input" (a permission prompt, `ask_user`-equivalent, etc.) that the
 * old hook-based tracking could only approximate via a filtered `Notification` hook. `idle` is the
 * resting state once a turn completes — Session Deck's "done". Anything else (an unrecognized
 * future status value) is ignored, same tolerance the hook-based system had for payloads it didn't
 * recognize.
 */
const STATUS_MAP: Readonly<Record<string, SessionStatus>> = {
  busy: 'running',
  shell: 'running',
  waiting: 'waiting',
  idle: 'done',
};

/** Pure mapping — unit-tested directly (`test/suite/claudeProcessWatcher.test.ts`) without touching the filesystem. */
export function classifyClaudeProcessStatus(status: string): SessionStatus | undefined {
  return STATUS_MAP[status];
}

interface ClaudeProcessFile {
  pid: number;
  sessionId: string;
  status: string;
}

function parseClaudeProcessFile(raw: string): ClaudeProcessFile | undefined {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return undefined; // Tolerate a partial write mid-save — best-effort, matching copilotStatusWatcher.ts's tolerance for its own live file.
  }
  if (typeof parsed !== 'object' || parsed === null) {
    return undefined;
  }
  const { pid, sessionId, status } = parsed as Record<string, unknown>;
  if (typeof pid !== 'number' || typeof sessionId !== 'string' || typeof status !== 'string') {
    return undefined;
  }
  return { pid, sessionId, status };
}

/**
 * Whether `pid` still refers to a live process — `process.kill(pid, 0)` is the standard
 * cross-platform (including Windows) liveness probe: signal `0` sends nothing, it just probes
 * whether the OS would let a real signal through, throwing if the process doesn't exist. Needed
 * because a `<pid>.json` file isn't guaranteed to be cleaned up the moment its process exits
 * (confirmed nothing in the binary's own strings points at a reliable delete-on-exit path) — so a
 * stale file left behind by a crashed/killed process should read as "gone", not as whatever status
 * it last reported before dying.
 */
function isProcessAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

/**
 * Live status for Claude Code sessions, without its `hooks` mechanism (`UserPromptSubmit`/`Stop`/
 * `Notification`/`SessionEnd`) or any settings mutation — replacing that entirely, per the same
 * zero-setup philosophy `copilotStatusWatcher.ts` already established for Copilot. One watcher for
 * the extension's whole lifetime (not started/stopped per terminal): `getClaudeSessionsStateDir()`
 * holds one file per *any* running `claude` process on the machine, not just ones Session Deck
 * itself opened — matching the old hook system's machine-wide scope, since hooks were configured
 * globally in `~/.claude/settings.json` too.
 *
 * Deliberately only calls `writeSessionStatus` on an actual status *change* (tracked in
 * `lastWritten`), not on every directory-change tick: `fs.watch` can fire more than once for a
 * single logical write, and unconditionally rewriting an unchanged status would refresh its
 * `updatedAt` every time, which would defeat `sessionStatus.ts`'s acknowledge-by-exact-timestamp
 * matching for "done" — a session you'd already acknowledged would keep reappearing as unseen.
 *
 * A session id's *first* observed status is a baseline, never displayed — `lastWritten` only
 * records it internally, `writeSessionStatus` isn't called. Needed because a freshly resumed
 * session's process reports `idle` (→ "done") from the moment it starts, before any prompt has
 * been typed — without this, every just-opened session would immediately show green. `clear()`
 * (called when a pid dies or its file disappears) removes the session from `lastWritten` too, so
 * resuming that same session again later gets its own fresh baseline rather than inheriting a
 * stale one from the previous process.
 */
export class ClaudeProcessWatcher {
  private watcher?: fs.FSWatcher;
  private readonly lastWritten = new Map<string, SessionStatus>();
  /** Session ids currently backed by a live pid file — used to notice one disappearing entirely between scans, the closest equivalent to the old `SessionEnd` hook. */
  private knownSessionIds = new Set<string>();

  constructor(private readonly log: (message: string) => void) {}

  start(): void {
    const dir = getClaudeSessionsStateDir();
    try {
      fs.mkdirSync(dir, { recursive: true });
    } catch (error) {
      this.log(`[claude-status] Failed to ensure ${dir} exists: ${String(error)}`);
      return;
    }
    this.scan();
    try {
      this.watcher = fs.watch(dir, () => this.scan());
    } catch (error) {
      this.log(`[claude-status] Failed to watch ${dir}: ${String(error)}`);
    }
  }

  dispose(): void {
    this.watcher?.close();
  }

  private scan(): void {
    const dir = getClaudeSessionsStateDir();
    let entries: string[];
    try {
      entries = fs.readdirSync(dir).filter((name) => name.endsWith('.json'));
    } catch (error) {
      this.log(`[claude-status] Failed to read ${dir}: ${String(error)}`);
      return;
    }

    const currentSessionIds = new Set<string>();
    for (const entry of entries) {
      let raw: string;
      try {
        raw = fs.readFileSync(path.join(dir, entry), 'utf8');
      } catch {
        continue; // Deleted between readdir and readFile — a harmless race, just skip it this pass.
      }
      const parsed = parseClaudeProcessFile(raw);
      if (!parsed) {
        continue;
      }
      currentSessionIds.add(parsed.sessionId);

      if (!isProcessAlive(parsed.pid)) {
        this.clear(parsed.sessionId);
        continue;
      }
      const status = classifyClaudeProcessStatus(parsed.status);
      if (!status) {
        continue;
      }
      if (!this.lastWritten.has(parsed.sessionId)) {
        // First sight of this session id is a baseline, not an event — same reasoning as
        // `WaitingNotifier`/the auto-archive cap elsewhere in this codebase. Needed here
        // specifically because a freshly resumed session's process starts in `idle` (Session
        // Deck's "done") before any prompt has been typed, which would otherwise show every
        // just-opened session as green immediately — reported live.
        this.lastWritten.set(parsed.sessionId, status);
        continue;
      }
      if (this.lastWritten.get(parsed.sessionId) !== status) {
        writeSessionStatus(parsed.sessionId, status);
        this.lastWritten.set(parsed.sessionId, status);
      }
    }

    // A session id that had a pid file last scan but has none at all now — its process file is
    // gone, the closest available signal to "the session actually ended".
    for (const sessionId of this.knownSessionIds) {
      if (!currentSessionIds.has(sessionId)) {
        this.clear(sessionId);
      }
    }
    this.knownSessionIds = currentSessionIds;
  }

  private clear(sessionId: string): void {
    clearSessionStatus(sessionId);
    this.lastWritten.delete(sessionId);
  }
}
