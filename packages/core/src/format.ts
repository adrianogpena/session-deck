const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/** `just now`, `5m ago`, `2h 10m ago`, `3d 4h ago`. */
export function humanizeSince(timeMs: number, nowMs: number = Date.now()): string {
  const elapsed = Math.max(0, nowMs - timeMs);
  if (elapsed < MINUTE) {
    return 'just now';
  }
  if (elapsed < HOUR) {
    return `${Math.floor(elapsed / MINUTE)}m ago`;
  }
  if (elapsed < DAY) {
    const hours = Math.floor(elapsed / HOUR);
    const minutes = Math.floor((elapsed % HOUR) / MINUTE);
    return minutes ? `${hours}h ${minutes}m ago` : `${hours}h ago`;
  }
  const days = Math.floor(elapsed / DAY);
  const hours = Math.floor((elapsed % DAY) / HOUR);
  return hours ? `${days}d ${hours}h ago` : `${days}d ago`;
}
