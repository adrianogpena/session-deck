const MINUTE: u64 = 60_000;
const HOUR: u64 = 60 * MINUTE;
const DAY: u64 = 24 * HOUR;

/// `just now`, `5m ago`, `2h 10m ago`, `3d 4h ago`. Times are epoch milliseconds.
pub fn humanize_since(time_ms: i64, now_ms: i64) -> String {
    let elapsed = (now_ms - time_ms).max(0) as u64;
    if elapsed < MINUTE {
        return "just now".to_string();
    }
    if elapsed < HOUR {
        return format!("{}m ago", elapsed / MINUTE);
    }
    if elapsed < DAY {
        let hours = elapsed / HOUR;
        let minutes = (elapsed % HOUR) / MINUTE;
        return if minutes > 0 {
            format!("{hours}h {minutes}m ago")
        } else {
            format!("{hours}h ago")
        };
    }
    let days = elapsed / DAY;
    let hours = (elapsed % DAY) / HOUR;
    if hours > 0 {
        format!("{days}d {hours}h ago")
    } else {
        format!("{days}d ago")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_790_000_000_000;
    const MIN: i64 = 60_000;

    #[test]
    fn humanize_since_buckets_elapsed_time() {
        assert_eq!(humanize_since(NOW - 30_000, NOW), "just now");
        assert_eq!(humanize_since(NOW - 5 * MIN, NOW), "5m ago");
        assert_eq!(humanize_since(NOW - 130 * MIN, NOW), "2h 10m ago");
        assert_eq!(humanize_since(NOW - 120 * MIN, NOW), "2h ago");
        assert_eq!(
            humanize_since(NOW - (3 * 24 * 60 + 4 * 60) * MIN, NOW),
            "3d 4h ago"
        );
        assert_eq!(humanize_since(NOW - 3 * 24 * 60 * MIN, NOW), "3d ago");
    }

    #[test]
    fn humanize_since_treats_a_future_time_as_just_now() {
        assert_eq!(humanize_since(NOW + 10 * MIN, NOW), "just now");
    }
}
