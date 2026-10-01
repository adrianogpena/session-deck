import * as fs from 'fs';
import * as readline from 'readline';
import { extractAssistantDisplayText, extractText, isDisplayableUserPrompt } from './claudeStorage';

/** One entry in a session's structured trace: a real user prompt, an assistant reply, or one tool call (request + its result, once seen). */
export interface TraceStep {
  kind: 'user' | 'assistant' | 'tool';
  /** Short line for the step list — a truncated prompt/reply, or `toolName(primary argument)`. */
  label: string;
  /** Full text shown once the step is selected: the prompt/reply, or the tool's name, input and result. */
  detail: string;
  /** Set once a matching `tool_result` block arrives marked `is_error`. */
  isError?: boolean;
}

const LABEL_MAX = 72;

function truncate(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}

function oneLine(text: string): string {
  return truncate(text.replace(/\s+/g, ' ').trim(), LABEL_MAX);
}

/** The tool's one argument most worth showing next to its name, e.g. `Read(src/app.ts)` or `Bash(npm test)`. */
function primaryArg(input: unknown): string | undefined {
  if (!input || typeof input !== 'object') {
    return undefined;
  }
  const obj = input as Record<string, unknown>;
  const value = obj.file_path ?? obj.path ?? obj.command ?? obj.pattern ?? obj.query ?? obj.url ?? obj.description;
  return typeof value === 'string' ? value : undefined;
}

function toolLabel(name: string, input: unknown): string {
  const arg = primaryArg(input);
  return arg ? `${name}(${oneLine(arg)})` : name;
}

/**
 * Parses one Claude transcript's raw `.jsonl` lines into an ordered {@link TraceStep} list — a request
 * for tool calls is paired with its result (matched by `tool_use_id`) into one step, since pitago's
 * trajectory view treats a call and the result it got as a single step. Pure, so it's unit-tested
 * directly without touching the filesystem; {@link readSessionTrace} is the filesystem-reading wrapper.
 */
export function buildTraceFromLines(lines: readonly string[]): TraceStep[] {
  const steps: TraceStep[] = [];
  const toolSteps = new Map<string, TraceStep>();

  for (const line of lines) {
    const trimmed = line.trim();
    if (!trimmed) {
      continue;
    }
    let record: Record<string, unknown>;
    try {
      record = JSON.parse(trimmed);
    } catch {
      continue;
    }
    const message = record.message as { role?: string; content?: unknown } | undefined;

    if (record.type === 'user' && message?.role === 'user') {
      const content = message.content;
      if (Array.isArray(content)) {
        for (const block of content) {
          if (!block || typeof block !== 'object' || (block as Record<string, unknown>).type !== 'tool_result') {
            continue;
          }
          const b = block as Record<string, unknown>;
          const step = typeof b.tool_use_id === 'string' ? toolSteps.get(b.tool_use_id) : undefined;
          if (!step) {
            continue;
          }
          const resultText = extractText(b.content).trim() || '(empty)';
          if (b.is_error) {
            step.isError = true;
          }
          step.detail += `\n\n${b.is_error ? 'Error:' : 'Result:'}\n${resultText}`;
        }
      }
      const text = extractText(content).trim();
      if (text && isDisplayableUserPrompt(text)) {
        steps.push({ kind: 'user', label: oneLine(text), detail: text });
      }
      continue;
    }

    if (record.type === 'assistant' && message?.role === 'assistant') {
      const content = message.content;
      const text = extractAssistantDisplayText(content).trim();
      if (text) {
        steps.push({ kind: 'assistant', label: oneLine(text), detail: text });
      }
      if (Array.isArray(content)) {
        for (const block of content) {
          if (!block || typeof block !== 'object' || (block as Record<string, unknown>).type !== 'tool_use') {
            continue;
          }
          const b = block as Record<string, unknown>;
          const name = typeof b.name === 'string' ? b.name : 'tool';
          const step: TraceStep = { kind: 'tool', label: toolLabel(name, b.input), detail: `${name}\n${JSON.stringify(b.input, null, 2)}` };
          steps.push(step);
          if (typeof b.id === 'string') {
            toolSteps.set(b.id, step);
          }
        }
      }
    }
  }
  return steps;
}

/** Reads and parses a whole Claude transcript file into its trace — see {@link buildTraceFromLines}. */
export async function readSessionTrace(filePath: string): Promise<TraceStep[]> {
  const rl = readline.createInterface({ input: fs.createReadStream(filePath), crlfDelay: Infinity });
  const lines: string[] = [];
  try {
    for await (const line of rl) {
      lines.push(line);
    }
  } finally {
    rl.close();
  }
  return buildTraceFromLines(lines);
}
