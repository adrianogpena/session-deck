import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { clearSessionStatus, writeSessionStatus, SessionStatus } from './sessionStatus';

/** Claude Code's own live per-process status directory — one `<pid>.json` file per running `claude` process, with `sessionId`/`status`/`statusUpdatedAt`. Undocumented format — treat as best-effort, not a stable contract. */
export function getClaudeSessionsStateDir(): string {
  return path.join(os.homedir(), '.claude', 'sessions');
}

/** `busy`/`shell` both map to "running". `waiting` means "needs your input". `idle` is the resting state once a turn completes — "done". Anything else is ignored. */
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
    return undefined; // Tolerate a partial write mid-save.
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

/** Whether `pid` still refers to a live process — `process.kill(pid, 0)` probes without sending a real signal. A `<pid>.json` file isn't guaranteed to be cleaned up on exit, so a stale file should read as "gone". */
function isProcessAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

/**
 * Live status for Claude Code sessions, zero setup — one watcher for the extension's lifetime,
 * covering every running `claude` process. Only writes on an actual status *change* (rewriting an
 * unchanged status would defeat "done"'s acknowledge-by-timestamp matching). A session's first
 * observed status is a baseline, never displayed — a freshly resumed process starts `idle`.
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
        // First sight is a baseline, not an event — see the class doc.
        this.lastWritten.set(parsed.sessionId, status);
        continue;
      }
      if (this.lastWritten.get(parsed.sessionId) !== status) {
        writeSessionStatus(parsed.sessionId, status);
        this.lastWritten.set(parsed.sessionId, status);
      }
    }

    // A session id with a pid file last scan but none now — its process ended.
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
