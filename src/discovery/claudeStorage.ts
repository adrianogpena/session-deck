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
 * Reverses Claude Code's "-"-for-path-separator folder-name encoding. Lossy (a literal "-" is
 * indistinguishable from an encoded separator), so only used as a last resort when no session has
 * a recorded `cwd` to read the real path from instead.
 */
export function decodeProjectPath(dirName: string): string {
  return dirName.replace(/^([A-Za-z])--/, '$1:\\').replace(/-/g, '\\');
}

/** `content` may be a plain string, an array of content blocks, or a single block object — handles all three, keeping only text/thinking. */
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

/** Filters out synthetic "user" turns (slash-command echoes, usage notices, etc.) that aren't real prompts. */
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

/** Combined, cached read of a session's starting `cwd` and first prompt. Cached by mtime so an unchanged session is never re-parsed. */
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

/** Full session content (all prompts + assistant text) for the "Search Sessions" command — cached separately from {@link readSessionMeta} since it's a heavier read only needed on search. */
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

/** The last assistant reply's text, for "Copy Last Response" — not cached, since it's a one-off action. */
export async function readLastAssistantResponse(filePath: string): Promise<string | undefined> {
  const rl = readline.createInterface({ input: fs.createReadStream(filePath), crlfDelay: Infinity });
  let last: string | undefined;

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
      if (record.type === 'assistant' && message?.role === 'assistant') {
        const text = extractAssistantDisplayText(message.content).trim();
        if (text) {
          last = text;
        }
      }
    }
  } finally {
    rl.close();
  }

  return last;
}

function truncate(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}
