import * as fs from 'fs';
import * as path from 'path';
import { ensureSessionStatusDir, getSessionStatusDir } from './sessionStatus';
import { isSafeSessionId } from '../commands/sessionId';
import { DailySpend, localDateKey, sevenDayDailySpend } from './usageDisplay';

/**
 * Context window and account-wide rate-limit usage for one Claude session, as last reported by its own
 * statusLine hook invocation — see `~/.claude/statusline-command.sh`, which Session Deck doesn't own but
 * extends to drop this file alongside writing the status line itself. `fiveHourPercent`/`sevenDayPercent`
 * are account-wide (the same regardless of which session's statusLine wrote them most recently); only
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
}

function usageFilePath(sessionId: string): string | undefined {
  if (!isSafeSessionId(sessionId)) {
    return undefined; // not a real session id — refuse rather than read/write outside the status dir
  }
  return path.join(getSessionStatusDir(), `${sessionId}.usage.json`);
}

const num = (value: unknown): number | undefined => (typeof value === 'number' && Number.isFinite(value) ? value : undefined);

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

/** `~/.claude/session-deck-status/seven-day-history.json` — `date -> cumulative 7d percent used`, account-wide like the readings it's built from. */
function sevenDayHistoryPath(): string {
  return path.join(getSessionStatusDir(), 'seven-day-history.json');
}

function readSevenDayHistoryFile(): Record<string, number> {
  try {
    const parsed: unknown = JSON.parse(fs.readFileSync(sevenDayHistoryPath(), 'utf8'));
    if (!parsed || typeof parsed !== 'object') {
      return {};
    }
    const history: Record<string, number> = {};
    for (const [key, value] of Object.entries(parsed as Record<string, unknown>)) {
      const n = num(value);
      if (n !== undefined) {
        history[key] = n;
      }
    }
    return history;
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
 * frequent disk writes.
 */
export function recordSevenDayUsageSample(percent: number, at: Date = new Date()): void {
  const history = readSevenDayHistoryFile();
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
  try {
    ensureSessionStatusDir();
    fs.writeFileSync(sevenDayHistoryPath(), JSON.stringify(history), 'utf8');
  } catch {
    // Best-effort — a lost sample just means that day shows up blank later.
  }
}

/** This week's (Monday through now) day-by-day spend against the 7d quota — see `sevenDayDailySpend`. */
export function readSevenDayDailySpend(now: Date = new Date()): DailySpend[] {
  return sevenDayDailySpend(readSevenDayHistoryFile(), now);
}
