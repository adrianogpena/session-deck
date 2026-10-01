import * as fs from 'fs';
import { extractAssistantDisplayText, extractText, isDisplayableUserPrompt } from '../discovery/claudeStorage';
import { splitLines } from './copilotStatusWatcher';

/** One displayable turn pulled from a Claude transcript line, for the live preview pane. */
export interface TailedTurn {
  role: 'user' | 'assistant';
  text: string;
}

/**
 * Parses one `.jsonl` transcript line into a displayable turn, or `undefined` for anything else
 * (tool calls, hidden slash-command echoes, unparseable/partial lines). Pure — unit-tested directly
 * without touching the filesystem.
 */
export function parseTranscriptLine(line: string): TailedTurn | undefined {
  const trimmed = line.trim();
  if (!trimmed) {
    return undefined;
  }
  let record: Record<string, unknown>;
  try {
    record = JSON.parse(trimmed);
  } catch {
    return undefined;
  }
  const message = record.message as { role?: string; content?: unknown } | undefined;
  if (record.type === 'user' && message?.role === 'user') {
    const text = extractText(message.content).trim();
    return text && isDisplayableUserPrompt(text) ? { role: 'user', text } : undefined;
  }
  if (record.type === 'assistant' && message?.role === 'assistant') {
    const text = extractAssistantDisplayText(message.content).trim();
    return text ? { role: 'assistant', text } : undefined;
  }
  return undefined;
}

/** How far back to backfill from the end of the file on attach — enough recent turns without parsing a multi-MB transcript from the start. */
const BACKFILL_WINDOW_BYTES = 64 * 1024;
/** Bounds the in-memory preview buffer so a long-running elsewhere session can't grow it without limit. */
const MAX_TAILED_TURNS = 50;

/**
 * Live, read-only preview of one Claude transcript file that's being written to by a process Session
 * Deck didn't start (another terminal, VS Code's own terminal). Backfills the last
 * {@link BACKFILL_WINDOW_BYTES} immediately on attach, then tails bytes appended after that — same
 * incremental-read idiom as `CopilotStatusWatcher`, but accumulating displayable turns instead of
 * discarding them after classifying a status.
 *
 * Single-slot: only the currently-previewed session ever needs this, so attaching to a new file
 * detaches the previous one rather than tracking a map of sessions like `CopilotStatusWatcher` does.
 */
export class ClaudeTranscriptTailer {
  private filePath?: string;
  private offset = 0;
  private carry = '';
  private turns: TailedTurn[] = [];
  private watcher?: fs.FSWatcher;
  private onUpdate?: (turns: TailedTurn[]) => void;

  constructor(private readonly log: (message: string) => void) {}

  attach(filePath: string, onUpdate: (turns: TailedTurn[]) => void): void {
    if (this.filePath === filePath) {
      return;
    }
    this.detach();
    this.filePath = filePath;
    this.onUpdate = onUpdate;
    this.backfill(filePath);
    try {
      this.watcher = fs.watch(filePath, () => this.poll(filePath));
    } catch (error) {
      this.log(`[live-preview] Failed to watch ${filePath}: ${String(error)}`);
    }
  }

  detach(): void {
    this.watcher?.close();
    this.watcher = undefined;
    this.filePath = undefined;
    this.onUpdate = undefined;
    this.offset = 0;
    this.carry = '';
    this.turns = [];
  }

  dispose(): void {
    this.detach();
  }

  private backfill(filePath: string): void {
    let size: number;
    try {
      size = fs.statSync(filePath).size;
    } catch {
      return;
    }
    const length = Math.min(BACKFILL_WINDOW_BYTES, size);
    if (length > 0) {
      try {
        const fd = fs.openSync(filePath, 'r');
        const buffer = Buffer.alloc(length);
        fs.readSync(fd, buffer, 0, length, size - length);
        fs.closeSync(fd);
        const lines = buffer.toString('utf8').split('\n');
        // The window's first line is usually cut mid-record when it doesn't cover the whole file
        // (same caveat as claudeStorage's readSessionTitle) — skip it rather than fail to parse it.
        if (length < size) {
          lines.shift();
        }
        for (const line of lines) {
          this.ingest(line);
        }
      } catch (error) {
        this.log(`[live-preview] Failed to read ${filePath}: ${String(error)}`);
      }
    }
    this.offset = size;
    this.emit();
  }

  private poll(filePath: string): void {
    if (this.filePath !== filePath) {
      return;
    }
    let size: number;
    try {
      size = fs.statSync(filePath).size;
    } catch {
      return;
    }
    if (size < this.offset) {
      this.offset = size; // truncated/replaced from under us
      return;
    }
    if (size === this.offset) {
      return;
    }
    let chunk: string;
    try {
      const fd = fs.openSync(filePath, 'r');
      const length = size - this.offset;
      const buffer = Buffer.alloc(length);
      fs.readSync(fd, buffer, 0, length, this.offset);
      fs.closeSync(fd);
      chunk = buffer.toString('utf8');
    } catch (error) {
      this.log(`[live-preview] Failed to read ${filePath}: ${String(error)}`);
      return;
    }
    this.offset = size;
    const { lines, carry } = splitLines(this.carry, chunk);
    this.carry = carry;
    let changed = false;
    for (const line of lines) {
      changed = this.ingest(line) || changed;
    }
    if (changed) {
      this.emit();
    }
  }

  private ingest(line: string): boolean {
    const turn = parseTranscriptLine(line);
    if (!turn) {
      return false;
    }
    this.turns.push(turn);
    if (this.turns.length > MAX_TAILED_TURNS) {
      this.turns.shift();
    }
    return true;
  }

  private emit(): void {
    this.onUpdate?.([...this.turns]);
  }
}
