import * as fs from 'fs';
import * as path from 'path';
import { ensureSessionStatusDir, getSessionStatusDir } from './sessionStatus';
import { isSafeSessionId } from '../commands/sessionId';
import { currentAccountEmail } from './account';
import { DailySpend, localDateKey, sevenDayDailySpend } from './usageDisplay';

/**
 * Context window and rate-limit usage for one Claude session, as last reported by its own statusLine hook
 * invocation — see `~/.claude/statusline-command.sh`, which Session Deck doesn't own but extends to drop
 * this file alongside writing the status line itself. `fiveHourPercent`/`sevenDayPercent` are shared across
 * every session logged into the same account (the same regardless of which session's statusLine wrote them
 * most recently) — see `accountEmail` for how readings from a different account are told apart; only
 * `contextPercent` is specific to this one session's transcript.
 */
export interface SessionUsageRecord {
  contextPercent?: number;
  fiveHourPercent?: number;
  /** Epoch ms. */
  fiveHourResetsAt?: number;
  sevenDayPercent?: number;
  /** Epoch ms. */
  sevenDayResetsAt?: number;
  /** Epoch ms, when this record was last written. */
  updatedAt: number;
  /** Which Claude.ai account was logged in when the statusLine hook wrote this record, if known. */
  accountEmail?: string;
}

function usageFilePath(sessionId: string): string | undefined {
  if (!isSafeSessionId(sessionId)) {
    return undefined; // not a real session id — refuse rather than read/write outside the status dir
  }
  return path.join(getSessionStatusDir(), `${sessionId}.usage.json`);
}

const num = (value: unknown): number | undefined => (typeof value === 'number' && Number.isFinite(value) ? value : undefined);
const str = (value: unknown): string | undefined => (typeof value === 'string' && value.length > 0 ? value : undefined);

/** `undefined` for an unreadable, unparseable, or (pre-merge-fix) too-old file missing `updatedAt`. */
function parseUsageRecord(raw: string): SessionUsageRecord | undefined {
  const parsed: unknown = JSON.parse(raw);
  if (!parsed || typeof parsed !== 'object') {
    return undefined;
  }
  const record = parsed as Record<string, unknown>;
  const updatedAt = num(record.updatedAt);
  if (updatedAt === undefined) {
    return undefined;
  }
  return {
    contextPercent: num(record.contextPercent),
    fiveHourPercent: num(record.fiveHourPercent),
    fiveHourResetsAt: num(record.fiveHourResetsAt),
    sevenDayPercent: num(record.sevenDayPercent),
    sevenDayResetsAt: num(record.sevenDayResetsAt),
    updatedAt,
    accountEmail: str(record.accountEmail),
  };
}

/** `undefined` means no usage data has been recorded yet for this session (e.g. it's never been resumed since the statusLine hook was extended). */
export function readSessionUsage(sessionId: string): SessionUsageRecord | undefined {
  const filePath = usageFilePath(sessionId);
  if (!filePath) {
    return undefined;
  }
  try {
    return parseUsageRecord(fs.readFileSync(filePath, 'utf8'));
  } catch {
    return undefined; // missing file (no usage recorded yet) or a transient partial write racing a read
  }
}

/**
 * The freshest known 5h/7d rate-limit reading, account-wide — unlike `contextPercent`, these aren't
 * specific to any one session, so this scans every session's usage file (not just the focused one) and
 * returns whichever carries the most recent `updatedAt` among those that actually recorded a rate-limit
 * reading. A session whose own statusLine hasn't fired in a while (idle, or simply not the one open)
 * shouldn't make the account's shared 5h/7d quota look emptier than it is.
 *
 * Readings tagged with a *different* account than the one currently logged in are skipped — otherwise,
 * switching between a personal and a work login would show whichever account's session happened to write
 * most recently, including stale numbers left over from the account you just switched away from.
 * Untagged readings (written before this tagging existed, or when the account couldn't be determined) are
 * still considered, since excluding them would just blank the view out rather than attribute them wrong.
 */
export function readLatestRateLimitUsage():
  | Pick<SessionUsageRecord, 'fiveHourPercent' | 'fiveHourResetsAt' | 'sevenDayPercent' | 'sevenDayResetsAt' | 'updatedAt'>
  | undefined {
  const dir = getSessionStatusDir();
  let names: string[];
  try {
    names = fs.readdirSync(dir).filter((name) => name.endsWith('.usage.json'));
  } catch {
    return undefined;
  }

  const account = currentAccountEmail();
  let latest:
    | Pick<SessionUsageRecord, 'fiveHourPercent' | 'fiveHourResetsAt' | 'sevenDayPercent' | 'sevenDayResetsAt' | 'updatedAt'>
    | undefined;
  for (const name of names) {
    let record: SessionUsageRecord | undefined;
    try {
      record = parseUsageRecord(fs.readFileSync(path.join(dir, name), 'utf8'));
    } catch {
      continue; // deleted between readdir and read, or a transient partial write
    }
    if (!record || (record.fiveHourPercent === undefined && record.sevenDayPercent === undefined)) {
      continue; // this session has no rate-limit reading of its own (or none yet)
    }
    if (account && record.accountEmail && record.accountEmail !== account) {
      continue; // reading belongs to a different, known account — not this one's quota
    }
    if (!latest || record.updatedAt > latest.updatedAt) {
      latest = {
        fiveHourPercent: record.fiveHourPercent,
        fiveHourResetsAt: record.fiveHourResetsAt,
        sevenDayPercent: record.sevenDayPercent,
        sevenDayResetsAt: record.sevenDayResetsAt,
        updatedAt: record.updatedAt,
      };
    }
  }
  if (latest?.sevenDayPercent !== undefined) {
    recordSevenDayUsageSample(latest.sevenDayPercent, new Date(latest.updatedAt));
  }
  return latest;
}

/** `~/.claude/session-deck-status/seven-day-history.json` — `account email -> date -> cumulative 7d percent used`. Scoped by account so switching between a personal and a work login doesn't blend their histories together. */
function sevenDayHistoryPath(): string {
  return path.join(getSessionStatusDir(), 'seven-day-history.json');
}

/** Key for readings written before this file was scoped by account, or when the account can't be determined. */
const UNKNOWN_ACCOUNT_KEY = 'unknown';

function accountKey(): string {
  return currentAccountEmail() ?? UNKNOWN_ACCOUNT_KEY;
}

function readSevenDayHistoryFile(): Record<string, Record<string, number>> {
  try {
    const parsed: unknown = JSON.parse(fs.readFileSync(sevenDayHistoryPath(), 'utf8'));
    if (!parsed || typeof parsed !== 'object') {
      return {};
    }
    const file: Record<string, Record<string, number>> = {};
    for (const [account, dates] of Object.entries(parsed as Record<string, unknown>)) {
      if (!dates || typeof dates !== 'object') {
        continue; // pre-account-scoping shape (`date -> percent` at the top level) — nothing to carry over
      }
      const perDate: Record<string, number> = {};
      for (const [key, value] of Object.entries(dates as Record<string, unknown>)) {
        const n = num(value);
        if (n !== undefined) {
          perDate[key] = n;
        }
      }
      file[account] = perDate;
    }
    return file;
  } catch {
    return {}; // missing file (never recorded) or a transient partial write racing a read
  }
}

/** A bit over a week, so Monday's entry survives through next Sunday even if the app is only opened at week's end. */
const SEVEN_DAY_HISTORY_RETENTION_DAYS = 9;

/**
 * Persists today's latest 7d-quota reading so the week's day-by-day spend (`readSevenDayDailySpend`)
 * survives restarts. Called from `readLatestRateLimitUsage` on every fresh reading; a no-op when today's
 * stored reading already matches, so the frequent polling that drives that function doesn't turn into
 * frequent disk writes. Filed under the currently logged-in account, so a reading taken under one account
 * never shows up as that account's history after switching to another.
 */
export function recordSevenDayUsageSample(percent: number, at: Date = new Date()): void {
  const file = readSevenDayHistoryFile();
  const account = accountKey();
  const history = file[account] ?? {};
  const key = localDateKey(at);
  if (history[key] === percent) {
    return;
  }
  history[key] = percent;
  const cutoffKey = localDateKey(new Date(at.getFullYear(), at.getMonth(), at.getDate() - SEVEN_DAY_HISTORY_RETENTION_DAYS));
  for (const k of Object.keys(history)) {
    if (k < cutoffKey) {
      delete history[k];
    }
  }
  file[account] = history;
  try {
    ensureSessionStatusDir();
    fs.writeFileSync(sevenDayHistoryPath(), JSON.stringify(file), 'utf8');
  } catch {
    // Best-effort — a lost sample just means that day shows up blank later.
  }
}

/** This week's (Monday through now) day-by-day spend against the 7d quota, for the currently logged-in account — see `sevenDayDailySpend`. */
export function readSevenDayDailySpend(now: Date = new Date()): DailySpend[] {
  return sevenDayDailySpend(readSevenDayHistoryFile()[accountKey()] ?? {}, now);
}
