/**
 * Pure formatting shared by every front end's usage view (VS Code extension tree, terminal UI list) —
 * kept here so the color thresholds and bar look stay identical between them.
 */

export type UsageMetric = 'context' | 'fiveHour' | 'sevenDay';

export const USAGE_METRIC_LABELS: Record<UsageMetric, string> = { context: 'Context', fiveHour: '5h', sevenDay: '7d' };

export type UsageSeverity = 'ok' | 'warning' | 'critical';

/**
 * Context fills up fastest and is cheapest to act on (compact/clear), so it turns warning/critical
 * earliest. 5h is next most urgent; 7d is the least (there's a whole week to spread it over).
 */
const USAGE_SEVERITY_THRESHOLDS: Record<UsageMetric, { warning: number; critical: number }> = {
  context: { warning: 20, critical: 50 },
  fiveHour: { warning: 50, critical: 80 },
  sevenDay: { warning: 70, critical: 90 },
};

export function usageSeverity(metric: UsageMetric, percent: number): UsageSeverity {
  const { warning, critical } = USAGE_SEVERITY_THRESHOLDS[metric];
  if (percent >= critical) {
    return 'critical';
  }
  if (percent >= warning) {
    return 'warning';
  }
  return 'ok';
}

/** Characters wide a usage bar is drawn at, regardless of percent — just the fill point moves. */
export const USAGE_BAR_WIDTH = 10;

export function renderUsageBar(percent: number, width: number = USAGE_BAR_WIDTH): string {
  const clamped = Math.max(0, Math.min(100, percent));
  const filled = Math.round((clamped / 100) * width);
  return '█'.repeat(filled) + '░'.repeat(width - filled);
}

/** `YYYY-MM-DD` in local time — the key `recordSevenDayUsageSample`/`sevenDayDailySpend` index history by. */
export function localDateKey(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${y}-${m}-${day}`;
}

/** Local midnight of the Monday on or before `d`. */
function mondayOf(d: Date): Date {
  const date = new Date(d.getFullYear(), d.getMonth(), d.getDate());
  const day = date.getDay(); // 0 = Sun .. 6 = Sat
  date.setDate(date.getDate() + (day === 0 ? -6 : 1 - day));
  return date;
}

const WEEKDAY_LABELS = ['Su', 'Mo', 'Tu', 'We', 'Th', 'Fr', 'Sa']; // indexed by Date#getDay()

export interface DailySpend {
  /** Two-letter weekday abbreviation: Mo, Tu, We, Th, Fr, Sa, Su. */
  label: string;
  /** Percent of the 7d quota spent that day. */
  percent: number;
}

/**
 * This calendar week's (Monday through `now`, weekends included) day-by-day spend against the 7d quota,
 * from a `date -> cumulative percent used` history as recorded by `recordSevenDayUsageSample`. A day's
 * spend is the jump from the previous recorded day's reading to its own, clamped at 0 since the 7d window
 * is rolling — usage can age out and the total can drop even with no new activity, which isn't "negative
 * spend". Days nobody recorded a reading for (Session Deck wasn't running) are left out rather than guessed
 * at, so gaps silently fold into the next recorded day's delta.
 */
export function sevenDayDailySpend(history: Record<string, number>, now: Date = new Date()): DailySpend[] {
  const monday = mondayOf(now);
  const days: DailySpend[] = [];
  let prevPercent = 0;
  for (let i = 0; i < 7; i++) {
    const d = new Date(monday);
    d.setDate(d.getDate() + i);
    if (d > now) {
      break;
    }
    const value = history[localDateKey(d)];
    if (value !== undefined) {
      days.push({ label: WEEKDAY_LABELS[d.getDay()], percent: Math.max(0, value - prevPercent) });
      prevPercent = value;
    }
  }
  return days;
}

/**
 * `8:30 PM`, or `20:30` with `use24Hour` — see `DeckConfig.ui.use24HourClock`. `withWeekday` prefixes
 * `Mon ` (the 7d quota's reset can be days out, so a bare time of day is ambiguous there, unlike 5h's).
 */
export function formatResetTime(epochMs: number, use24Hour: boolean, withWeekday = false): string {
  const date = new Date(epochMs);
  const weekdayPrefix = withWeekday ? `${date.toLocaleDateString('en-US', { weekday: 'short' })} ` : '';
  return weekdayPrefix + date.toLocaleString('en-US', {
    hour: use24Hour ? '2-digit' : 'numeric',
    minute: '2-digit',
    hour12: !use24Hour,
  });
}
