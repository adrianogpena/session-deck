import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';
import * as readline from 'readline';

export function getClaudeProjectsDir(): string {
  return path.join(os.homedir(), '.claude', 'projects');
}

/** One folder Claude Code created under ~/.claude/projects for a distinct cwd it was launched from. */
export function listProjectDirNames(): string[] {
  const root = getClaudeProjectsDir();
  if (!fs.existsSync(root)) {
    return [];
  }
  return fs
    .readdirSync(root, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name);
}

export function listSessionFiles(dirName: string): string[] {
  const dir = path.join(getClaudeProjectsDir(), dirName);
  if (!fs.existsSync(dir)) {
    return [];
  }
  return fs.readdirSync(dir).filter((f) => f.endsWith('.jsonl'));
}

/**
 * Claude Code encodes a project's absolute path into its storage folder name by
 * replacing path separators (and other non-alphanumeric characters) with "-",
 * e.g. "C:\Users\me\app" -> "C--Users-me-app". That's lossy — a literal "-" in a
 * real folder name is indistinguishable from an encoded separator — so this is
 * only used as a last resort when a folder has no session with a recorded `cwd`
 * to read the real path from instead.
 */
export function decodeProjectPath(dirName: string): string {
  return dirName.replace(/^([A-Za-z])--/, '$1:\\').replace(/-/g, '\\');
}

/**
 * Extracts display text from a message `content` field. Claude Code represents
 * it as a plain string, an array of content blocks (text/thinking/tool_use/
 * tool_result/image/...), or occasionally a single block object — this handles
 * all three uniformly, joining `text`/`thinking` blocks and ignoring the rest
 * (tool_use, tool_result, images contribute nothing to display text).
 */
export function extractText(content: unknown): string {
  if (typeof content === 'string') {
    return content;
  }
  if (Array.isArray(content)) {
    return content
      .map((part) => extractTextFromPart(part))
      .filter((part) => part.length > 0)
      .join('\n');
  }
  if (content && typeof content === 'object') {
    return extractTextFromPart(content);
  }
  return '';
}

function extractTextFromPart(part: unknown): string {
  if (typeof part === 'string') {
    return part;
  }
  if (!part || typeof part !== 'object') {
    return '';
  }
  const block = part as Record<string, unknown>;
  if (typeof block.text === 'string') {
    return block.text;
  }
  if (typeof block.thinking === 'string') {
    return block.thinking;
  }
  return '';
}

/** Assistant text for display purposes only: `text` blocks, deliberately excluding `thinking` for readability. */
export function extractAssistantDisplayText(content: unknown): string {
  if (!Array.isArray(content)) {
    return '';
  }
  return content
    .filter((block): block is { type: string; text: string } => {
      return !!block && typeof block === 'object' && (block as Record<string, unknown>).type === 'text';
    })
    .map((block) => block.text)
    .join('\n');
}

const HIDDEN_USER_PROMPT_PREFIXES = [
  '<local-command-caveat>',
  '<command-name>',
  '<command-message>',
  '<command-args>',
  '<local-command-stdout>',
  '<local-command-stderr>',
  '<local-command-exit-code>',
  '<usage>',
  'agentId:',
];

/**
 * Filters out synthetic "user" turns that aren't real prompts — slash-command
 * echoes, usage-limit notices, sidechain/subagent markers — so a session's
 * title/search text surfaces what was actually typed, not plumbing.
 */
export function isDisplayableUserPrompt(rawPrompt: string): boolean {
  const normalized = rawPrompt.replace(/\s+/g, ' ').trim();
  if (!normalized) {
    return false;
  }
  return !HIDDEN_USER_PROMPT_PREFIXES.some((prefix) => normalized.startsWith(prefix));
}

export interface SessionMeta {
  cwd?: string;
  firstPrompt?: string;
}

const sessionMetaCache = new Map<string, { mtimeMs: number; meta: SessionMeta }>();

/**
 * Combined, cached read of a session's starting `cwd` and title-worthy first
 * prompt, in a single pass over the file. Cached by mtime so an unchanged
 * session is never re-parsed on refresh — significant once there are more than
 * a handful of sessions, since every tree refresh previously re-scanned every
 * session's transcript from scratch.
 */
export async function readSessionMeta(filePath: string): Promise<SessionMeta> {
  let mtimeMs: number;
  try {
    mtimeMs = fs.statSync(filePath).mtimeMs;
  } catch {
    return {};
  }

  const cached = sessionMetaCache.get(filePath);
  if (cached && cached.mtimeMs === mtimeMs) {
    return cached.meta;
  }

  const meta = await parseSessionMeta(filePath);
  sessionMetaCache.set(filePath, { mtimeMs, meta });
  return meta;
}

/** Call on a manual refresh so an externally-edited transcript is never served from a stale cache. */
export function clearSessionMetaCache(): void {
  sessionMetaCache.clear();
}

async function parseSessionMeta(filePath: string): Promise<SessionMeta> {
  const rl = readline.createInterface({ input: fs.createReadStream(filePath), crlfDelay: Infinity });
  let cwd: string | undefined;
  let firstPrompt: string | undefined;

  try {
    for await (const line of rl) {
      if (cwd && firstPrompt) {
        break;
      }
      if (!line.trim()) {
        continue;
      }

      let record: Record<string, unknown>;
      try {
        record = JSON.parse(line);
      } catch {
        continue;
      }

      if (!cwd && typeof record.cwd === 'string') {
        cwd = record.cwd;
      }

      if (!firstPrompt && record.type === 'user') {
        const message = record.message as { role?: string; content?: unknown } | undefined;
        if (message?.role === 'user') {
          const text = extractText(message.content).trim();
          if (text && isDisplayableUserPrompt(text)) {
            firstPrompt = truncate(text.replace(/\s+/g, ' '), 80);
          }
        }
      }
    }
  } finally {
    rl.close();
  }

  return { cwd, firstPrompt };
}

const searchTextCache = new Map<string, { mtimeMs: number; text: string }>();

/**
 * Full session content (all displayable user prompts + assistant text, not just
 * the first prompt), for the "Search Sessions" command. Deliberately not part
 * of {@link readSessionMeta} — it's a heavier read that's only ever needed when
 * a search actually runs, not on every tree refresh — and is cached separately
 * by mtime for the same reason.
 */
export async function readSessionSearchText(filePath: string): Promise<string> {
  let mtimeMs: number;
  try {
    mtimeMs = fs.statSync(filePath).mtimeMs;
  } catch {
    return '';
  }

  const cached = searchTextCache.get(filePath);
  if (cached && cached.mtimeMs === mtimeMs) {
    return cached.text;
  }

  const text = await parseSessionSearchText(filePath);
  searchTextCache.set(filePath, { mtimeMs, text });
  return text;
}

export function clearSearchTextCache(): void {
  searchTextCache.clear();
}

async function parseSessionSearchText(filePath: string): Promise<string> {
  const rl = readline.createInterface({ input: fs.createReadStream(filePath), crlfDelay: Infinity });
  const parts: string[] = [];

  try {
    for await (const line of rl) {
      if (!line.trim()) {
        continue;
      }
      let record: Record<string, unknown>;
      try {
        record = JSON.parse(line);
      } catch {
        continue;
      }

      const message = record.message as { role?: string; content?: unknown } | undefined;
      if (record.type === 'user' && message?.role === 'user') {
        const text = extractText(message.content).trim();
        if (text && isDisplayableUserPrompt(text)) {
          parts.push(text);
        }
      } else if (record.type === 'assistant' && message?.role === 'assistant') {
        const text = extractAssistantDisplayText(message.content).trim();
        if (text) {
          parts.push(text);
        }
      }
    }
  } finally {
    rl.close();
  }

  return parts.join('\n');
}

function truncate(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}
