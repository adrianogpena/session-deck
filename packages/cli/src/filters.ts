/** What the filter pills count and toggle. `starting` counts as running, a crashed/exited agent as error. */
export type StatusCategory = 'running' | 'waiting' | 'idle' | 'error' | 'stopped';

export const STATUS_CATEGORIES: readonly StatusCategory[] = ['running', 'waiting', 'idle', 'error', 'stopped'];

export type TimeFilter = 'all' | 'today' | '3d' | '7d';

const TIME_FILTERS: readonly TimeFilter[] = ['all', 'today', '3d', '7d'];
const DAY_MS = 24 * 60 * 60 * 1000;

export function nextTimeFilter(current: TimeFilter): TimeFilter {
  return TIME_FILTERS[(TIME_FILTERS.indexOf(current) + 1) % TIME_FILTERS.length];
}

/** `today` means since local midnight. */
export function withinTimeFilter(mtimeMs: number, filter: TimeFilter, nowMs: number = Date.now()): boolean {
  if (filter === 'all') {
    return true;
  }
  if (filter === 'today') {
    const midnight = new Date(nowMs);
    midnight.setHours(0, 0, 0, 0);
    return mtimeMs >= midnight.getTime();
  }
  return nowMs - mtimeMs <= (filter === '3d' ? 3 : 7) * DAY_MS;
}

/** An empty set means no status filter (the "All" pill). */
export function matchesStatusFilter(category: StatusCategory, filter: ReadonlySet<StatusCategory>): boolean {
  return filter.size === 0 || filter.has(category);
}
