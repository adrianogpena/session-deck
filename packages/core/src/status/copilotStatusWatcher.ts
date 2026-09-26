import * as fs from 'fs';
import { getCopilotSessionEventsLogPath } from '../discovery/copilotStorage';
import { clearSessionStatus, writeSessionStatus, SessionStatus } from './sessionStatus';

/**
 * Copilot CLI's `events.jsonl` event types mapped to Session Deck's coarse status. Unlisted types
 * are ignored (status stays as-is). Note: the built-in `ask_user` clarifying-question tool does NOT
 * fire `*.requested`/`*.completed` despite the schema — it goes through the generic
 * `tool.execution_start`/`_complete` pair like any other tool, distinguished by `data.toolName`.
 */
const ASK_USER_TOOL_NAME = 'ask_user';
const RUNNING_EVENTS = new Set(['assistant.turn_start', 'permission.completed', 'elicitation.completed', 'user_input.completed']);
const WAITING_EVENTS = new Set(['permission.requested', 'elicitation.requested', 'user_input.requested']);
const DONE_EVENTS = new Set(['assistant.turn_end']);
const ERROR_EVENTS = new Set(['session.error']);
const SHUTDOWN_EVENTS = new Set(['session.shutdown']);

export type CopilotStatusAction = { kind: 'status'; status: SessionStatus } | { kind: 'clear' } | undefined;

/** The subset of a parsed `events.jsonl` line `classifyCopilotEvent` actually looks at. */
export interface CopilotEventLike {
  type: string;
  data?: { toolName?: unknown };
}

/** Pure decision logic — see `CopilotStatusWatcher` for how it's actually applied. Unit-tested directly (`test/suite/copilotStatusWatcher.test.ts`) without touching the filesystem. */
export function classifyCopilotEvent(event: CopilotEventLike): CopilotStatusAction {
  const { type } = event;
  if (SHUTDOWN_EVENTS.has(type)) {
    return { kind: 'clear' };
  }
  if (ERROR_EVENTS.has(type)) {
    return { kind: 'status', status: 'error' };
  }
  if (type === 'tool.execution_start' && event.data?.toolName === ASK_USER_TOOL_NAME) {
    return { kind: 'status', status: 'waiting' };
  }
  if (type === 'tool.execution_complete' && event.data?.toolName === ASK_USER_TOOL_NAME) {
    return { kind: 'status', status: 'running' };
  }
  if (WAITING_EVENTS.has(type)) {
    return { kind: 'status', status: 'waiting' };
  }
  if (DONE_EVENTS.has(type)) {
    return { kind: 'status', status: 'done' };
  }
  if (RUNNING_EVENTS.has(type)) {
    return { kind: 'status', status: 'running' };
  }
  return undefined;
}

/** Splits a growing NDJSON chunk into complete lines plus a trailing partial line to carry into the next read. */
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

/** How long to wait for a resumed session's `events.jsonl` to appear before giving up. */
const FILE_WAIT_TIMEOUT_MS = 15_000;
const FILE_WAIT_POLL_INTERVAL_MS = 500;

/**
 * Live status for Copilot CLI sessions, tailing `events.jsonl` directly (zero setup, no hooks
 * config). `start()` tails only bytes appended after tracking begins — never backfills history.
 * Writes through the same status file `claudeProcessWatcher.ts` uses for Claude, so every
 * downstream consumer (decorations, `WaitingNotifier`, tooltips) works unchanged.
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

  /** Starts at EOF — only events appended from this point on are "live". */
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
    // File was truncated/replaced from under us — resync to the new end instead of a stale offset.
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
    let parsed: { type?: unknown; data?: unknown };
    try {
      parsed = JSON.parse(trimmed);
    } catch {
      return; // Tolerate a malformed/partial line.
    }
    if (typeof parsed.type !== 'string') {
      return;
    }
    const data = parsed.data && typeof parsed.data === 'object' ? (parsed.data as { toolName?: unknown }) : undefined;
    const action = classifyCopilotEvent({ type: parsed.type, data });
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
