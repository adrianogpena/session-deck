import * as fs from 'fs';
import * as path from 'path';
import { getDeckHomeDir } from './deckStore';
import { SessionStatus } from '../status/sessionStatus';

/** Overrides for one agent's launch command, see {@link DeckConfig.tools}. */
export interface ToolConfig {
  /** `false` skips discovering and starting this agent's sessions entirely (its `n`/`N` key flashes instead). Default: true. */
  enabled?: boolean;
  /** Replaces the executable Session Deck spawns (a bare name resolved on PATH, or a full path). Skips the default `where.exe` lookup on Windows. */
  command?: string;
  /** Extra arguments appended after the ones Session Deck builds itself (`--resume <id>`, etc). */
  args?: string[];
}

export interface DeckConfig {
  ui: {
    /** How many of the most recent sessions the terminal UI loads from disk. Default: 30. */
    maxSessionsListed: number;
    /** Desktop notifications for waiting/finished/error sessions. Default: on. */
    notifications: boolean;
    /** Which statuses `notifications` fires for. Default: waiting, done, error (not running). */
    notifyStatuses: SessionStatus[];
    /**
     * Top-level projects you haven't manually reordered (`K`/`J`) sort by most-recent-activity when
     * `true`, so they shuffle as sessions become active — or alphabetically, a fixed order, when
     * `false`. Default: false.
     */
    recentProjectsFirst: boolean;
  };
  tools: {
    claude: ToolConfig;
    copilot: ToolConfig;
  };
  trash: {
    /** Deleted sessions older than this are purged from `~/.session-deck/trash/` at startup. Default: 30. */
    retentionDays: number;
  };
}

const DEFAULT_MAX_SESSIONS_LISTED = 30;
const DEFAULT_NOTIFY_STATUSES: readonly SessionStatus[] = ['waiting', 'done', 'error'];
const VALID_STATUSES: readonly SessionStatus[] = ['running', 'waiting', 'done', 'error'];
const DEFAULT_TRASH_RETENTION_DAYS = 30;

function defaultDeckConfig(): DeckConfig {
  return {
    ui: {
      maxSessionsListed: DEFAULT_MAX_SESSIONS_LISTED,
      notifications: true,
      notifyStatuses: [...DEFAULT_NOTIFY_STATUSES],
      recentProjectsFirst: false,
    },
    tools: { claude: {}, copilot: {} },
    trash: { retentionDays: DEFAULT_TRASH_RETENTION_DAYS },
  };
}

/** `~/.session-deck/config.json` — editable by hand, or from the terminal UI's config popup (`C`). */
export function getDeckConfigPath(): string {
  return path.join(getDeckHomeDir(), 'config.json');
}

function parseToolConfig(raw: unknown): ToolConfig {
  if (typeof raw !== 'object' || raw === null) {
    return {};
  }
  const { enabled, command, args } = raw as Record<string, unknown>;
  const config: ToolConfig = {};
  if (typeof enabled === 'boolean') {
    config.enabled = enabled;
  }
  if (typeof command === 'string' && command.trim()) {
    config.command = command.trim();
  }
  if (Array.isArray(args) && args.every((a): a is string => typeof a === 'string')) {
    config.args = args;
  }
  return config;
}

/** Tolerant: a missing, malformed or partial file behaves like an empty one — every setting falls back to its default. */
export function parseDeckConfig(raw: string): DeckConfig {
  const config = defaultDeckConfig();
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return config;
  }
  if (typeof parsed !== 'object' || parsed === null) {
    return config;
  }
  const { ui, tools, trash } = parsed as Record<string, unknown>;
  if (typeof ui === 'object' && ui !== null) {
    const { maxSessionsListed, notifications, notifyStatuses, recentProjectsFirst } = ui as Record<string, unknown>;
    if (typeof maxSessionsListed === 'number' && Number.isInteger(maxSessionsListed) && maxSessionsListed > 0) {
      config.ui.maxSessionsListed = maxSessionsListed;
    }
    if (typeof notifications === 'boolean') {
      config.ui.notifications = notifications;
    }
    if (Array.isArray(notifyStatuses) && notifyStatuses.every((s): s is SessionStatus => VALID_STATUSES.includes(s as SessionStatus))) {
      config.ui.notifyStatuses = notifyStatuses;
    }
    if (typeof recentProjectsFirst === 'boolean') {
      config.ui.recentProjectsFirst = recentProjectsFirst;
    }
  }
  if (typeof tools === 'object' && tools !== null) {
    const { claude, copilot } = tools as Record<string, unknown>;
    config.tools.claude = parseToolConfig(claude);
    config.tools.copilot = parseToolConfig(copilot);
  }
  if (typeof trash === 'object' && trash !== null) {
    const { retentionDays } = trash as Record<string, unknown>;
    if (typeof retentionDays === 'number' && Number.isInteger(retentionDays) && retentionDays > 0) {
      config.trash.retentionDays = retentionDays;
    }
  }
  return config;
}

/** Reads and parses `~/.session-deck/config.json`. A missing file returns the defaults, same as an empty one. */
export function readDeckConfig(filePath: string = getDeckConfigPath()): DeckConfig {
  let raw: string;
  try {
    raw = fs.readFileSync(filePath, 'utf8');
  } catch {
    return defaultDeckConfig();
  }
  return parseDeckConfig(raw);
}

/** Writes every field's resolved value, defaults included, so the file always shows what's actually in effect. */
export function writeDeckConfig(config: DeckConfig, filePath: string = getDeckConfigPath()): void {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, `${JSON.stringify(config, null, 2)}\n`, 'utf8');
}
