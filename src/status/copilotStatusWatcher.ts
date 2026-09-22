import * as fs from 'fs';
import { getCopilotSessionEventsLogPath } from '../discovery/copilotStorage';
import { clearSessionStatus, writeSessionStatus, SessionStatus } from './sessionStatus';

/**
 * Copilot CLI's own `events.jsonl` event `type` strings (pulled from its bundled
 * `session-events.schema.json`), mapped to Session Deck's coarse status. Anything not listed here
 * is ignored — the status just stays whatever it last was, the same tolerance Claude's hook-based
 * tracking already has for hook payloads it doesn't recognize. `.completed` events map to
 * "running" (work resumes right after a permission/elicitation/input request is resolved) rather
 * than being ignored — a distinction Claude's own hook set can't make at all, since it has no
 * "resumed after approval" signal of its own.
 */
const RUNNING_EVENTS = new Set(['assistant.turn_start', 'permission.completed', 'elicitation.completed', 'user_input.completed']);
const WAITING_EVENTS = new Set(['permission.requested', 'elicitation.requested', 'user_input.requested']);
const DONE_EVENTS = new Set(['assistant.turn_end']);
const ERROR_EVENTS = new Set(['session.error']);
const SHUTDOWN_EVENTS = new Set(['session.shutdown']);

export type CopilotStatusAction = { kind: 'status'; status: SessionStatus } | { kind: 'clear' } | undefined;

/** Pure decision logic — see `CopilotStatusWatcher` for how it's actually applied. Unit-tested directly (`test/suite/copilotStatusWatcher.test.ts`) without touching the filesystem. */
export function classifyCopilotEvent(eventType: string): CopilotStatusAction {
  if (SHUTDOWN_EVENTS.has(eventType)) {
    return { kind: 'clear' };
  }
  if (ERROR_EVENTS.has(eventType)) {
    return { kind: 'status', status: 'error' };
  }
  if (WAITING_EVENTS.has(eventType)) {
    return { kind: 'status', status: 'waiting' };
  }
  if (DONE_EVENTS.has(eventType)) {
    return { kind: 'status', status: 'done' };
  }
  if (RUNNING_EVENTS.has(eventType)) {
    return { kind: 'status', status: 'running' };
  }
  return undefined;
}

/**
 * Splits a growing NDJSON stream's newly-read chunk into complete lines plus any trailing partial
 * line to carry into the next read — pure, so a chunk boundary landing mid-line is handled the
 * same regardless of whether the rest of that line arrives in this read or the next `poll()`.
 */
export function splitLines(carry: string, chunk: string): { lines: string[]; carry: string } {
  const combined = carry + chunk;
  const parts = combined.split('\n');
  const newCarry = parts.pop() ?? '';
  return { lines: parts, carry: newCarry };
}

interface TailState {
  offset: number;
  carry: string;
  watcher?: fs.FSWatcher;
  pendingPoll?: NodeJS.Timeout;
}

/** How long to wait for a resumed session's `events.jsonl` to exist before giving up — see `waitForFile`. In practice this is near-instant, since the file/directory already exists for any session that can be resumed at all; this only guards the rare case of Copilot CLI not having flushed it yet. */
const FILE_WAIT_TIMEOUT_MS = 15_000;
const FILE_WAIT_POLL_INTERVAL_MS = 500;

/**
 * Live status for Copilot CLI sessions, without Copilot's own opt-in `hooks` mechanism
 * (`.github/hooks/*.json` / the `hooks` setting) or any settings mutation: Copilot CLI already
 * appends every session's activity to `~/.copilot/session-state/<sessionId>/events.jsonl`
 * unconditionally, for its own IDE/resume support. `start()` tails only the bytes appended *after*
 * tracking begins — never backfills a session's history, the same "baseline on first sight"
 * reasoning as `sessionProvider.ts`'s auto-archive cap — and writes through the same
 * `status/sessionStatus.ts` file `reportStatus.ts` writes for Claude, so every downstream consumer
 * (the decoration provider, `WaitingNotifier`, the tree's tooltip) already works unchanged;
 * nothing about them is Copilot-aware.
 *
 * This reads Copilot CLI's own internal, undocumented on-disk format (not its public, versioned
 * `hooks` config) — chosen because it needs zero setup and no per-event subprocess spawn, at the
 * cost of being unversioned: if a future Copilot CLI release changes `events.jsonl`'s shape, this
 * degrades silently (status just stops updating for Copilot sessions) rather than breaking anything.
 */
export class CopilotStatusWatcher {
  private readonly tails = new Map<string, TailState>();

  constructor(private readonly log: (message: string) => void) {}

  start(sessionId: string): void {
    if (this.tails.has(sessionId)) {
      return;
    }
    const state: TailState = { offset: 0, carry: '' };
    this.tails.set(sessionId, state);
    this.waitForFile(sessionId, state, Date.now());
  }

  stop(sessionId: string): void {
    const state = this.tails.get(sessionId);
    if (!state) {
      return;
    }
    state.watcher?.close();
    if (state.pendingPoll) {
      clearTimeout(state.pendingPoll);
    }
    this.tails.delete(sessionId);
  }

  dispose(): void {
    for (const sessionId of [...this.tails.keys()]) {
      this.stop(sessionId);
    }
  }

  private waitForFile(sessionId: string, state: TailState, startedAt: number): void {
    const filePath = getCopilotSessionEventsLogPath(sessionId);
    if (fs.existsSync(filePath)) {
      this.attach(sessionId, filePath, state);
      return;
    }
    if (Date.now() - startedAt >= FILE_WAIT_TIMEOUT_MS) {
      this.log(`[copilot-status] Gave up waiting for ${filePath} to appear.`);
      this.tails.delete(sessionId);
      return;
    }
    state.pendingPoll = setTimeout(() => this.waitForFile(sessionId, state, startedAt), FILE_WAIT_POLL_INTERVAL_MS);
  }

  /** Starts at EOF, not offset 0 — only events appended from this point on are "live", mirroring Claude's hook model where resuming a session shows no status until the next real event. */
  private attach(sessionId: string, filePath: string, state: TailState): void {
    try {
      state.offset = fs.statSync(filePath).size;
    } catch {
      state.offset = 0;
    }
    try {
      state.watcher = fs.watch(filePath, () => this.poll(sessionId, filePath, state));
    } catch (error) {
      this.log(`[copilot-status] Failed to watch ${filePath}: ${String(error)}`);
    }
  }

  private poll(sessionId: string, filePath: string, state: TailState): void {
    let size: number;
    try {
      size = fs.statSync(filePath).size;
    } catch {
      return;
    }
    // A smaller size than last seen means the file was truncated/replaced from under us (not
    // expected in normal operation) — resync to the new end rather than re-reading from a stale
    // offset, which could otherwise throw or replay unrelated bytes as if they were new events.
    if (size < state.offset) {
      state.offset = size;
      return;
    }
    if (size === state.offset) {
      return;
    }

    let chunk: string;
    try {
      const fd = fs.openSync(filePath, 'r');
      const length = size - state.offset;
      const buffer = Buffer.alloc(length);
      fs.readSync(fd, buffer, 0, length, state.offset);
      fs.closeSync(fd);
      chunk = buffer.toString('utf8');
    } catch (error) {
      this.log(`[copilot-status] Failed to read ${filePath}: ${String(error)}`);
      return;
    }
    state.offset = size;

    const { lines, carry } = splitLines(state.carry, chunk);
    state.carry = carry;
    for (const line of lines) {
      this.handleLine(sessionId, line);
    }
  }

  private handleLine(sessionId: string, line: string): void {
    const trimmed = line.trim();
    if (!trimmed) {
      return;
    }
    let type: unknown;
    try {
      type = (JSON.parse(trimmed) as { type?: unknown }).type;
    } catch {
      return; // Tolerate a malformed/partial line — best-effort, matching this feature's informational (not load-bearing) role.
    }
    if (typeof type !== 'string') {
      return;
    }
    const action = classifyCopilotEvent(type);
    if (!action) {
      return;
    }
    if (action.kind === 'clear') {
      clearSessionStatus(sessionId);
    } else {
      writeSessionStatus(sessionId, action.status);
    }
  }
}
