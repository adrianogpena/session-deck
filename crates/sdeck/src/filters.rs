//! Port of `filters.ts`: what the filter pills count and toggle.

pub use sdeck_core::store::tree_prefs::SessionCategory as StatusCategory;

pub const STATUS_CATEGORIES: [StatusCategory; 5] = [
    StatusCategory::Running,
    StatusCategory::Waiting,
    StatusCategory::Idle,
    StatusCategory::Error,
    StatusCategory::Stopped,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeFilter {
    #[default]
    All,
    Today,
    ThreeDays,
    SevenDays,
}

impl TimeFilter {
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "all time",
            Self::Today => "today",
            Self::ThreeDays => "3 days",
            Self::SevenDays => "7 days",
        }
    }
}

/// Session counts per status category, for the header logo and the pills.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StatusCounts([usize; 5]);

impl StatusCounts {
    pub fn get(&self, category: StatusCategory) -> usize {
        self.0[Self::index(category)]
    }

    pub fn set(&mut self, category: StatusCategory, count: usize) {
        self.0[Self::index(category)] = count;
    }

    fn index(category: StatusCategory) -> usize {
        STATUS_CATEGORIES.iter().position(|&c| c == category).unwrap()
    }
}

impl TimeFilter {
    /// all → today → 3 days → 7 days → all.
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::Today,
            Self::Today => Self::ThreeDays,
            Self::ThreeDays => Self::SevenDays,
            Self::SevenDays => Self::All,
        }
    }
}

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// `Today` means since local midnight.
pub fn within_time_filter(mtime_ms: i64, filter: TimeFilter, now_ms: i64) -> bool {
    match filter {
        TimeFilter::All => true,
        TimeFilter::Today => mtime_ms >= local_midnight_ms(now_ms),
        TimeFilter::ThreeDays => now_ms - mtime_ms <= 3 * DAY_MS,
        TimeFilter::SevenDays => now_ms - mtime_ms <= 7 * DAY_MS,
    }
}

fn local_midnight_ms(now_ms: i64) -> i64 {
    use chrono::{Local, TimeZone};
    Local
        .timestamp_millis_opt(now_ms)
        .single()
        .and_then(|now| now.date_naive().and_hms_opt(0, 0, 0))
        .and_then(|midnight| Local.from_local_datetime(&midnight).earliest())
        .map_or(now_ms - DAY_MS, |m| m.timestamp_millis())
}

/// An empty filter means no status filter (the "All" pill).
pub fn matches_status_filter(category: StatusCategory, filter: &[StatusCategory]) -> bool {
    filter.is_empty() || filter.contains(&category)
}

/// The key that toggles a status category in the filter: `!` running, `@` waiting, `#` idle,
/// `&` error, `~` stopped.
pub fn filter_key_category(key: &str) -> Option<StatusCategory> {
    match key {
        "!" => Some(StatusCategory::Running),
        "@" => Some(StatusCategory::Waiting),
        "#" => Some(StatusCategory::Idle),
        "&" => Some(StatusCategory::Error),
        "~" => Some(StatusCategory::Stopped),
        _ => None,
    }
}

/// Adds `category` to the filter, or removes it when it's already there.
pub fn toggle_status_filter(filter: &mut Vec<StatusCategory>, category: StatusCategory) {
    match filter.iter().position(|&c| c == category) {
        Some(i) => {
            filter.remove(i);
        }
        None => filter.push(category),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, TimeZone};

    const HOUR: i64 = 60 * 60 * 1000;

    #[test]
    fn next_time_filter_cycles_all_today_3d_7d_all() {
        assert_eq!(TimeFilter::All.next(), TimeFilter::Today);
        assert_eq!(TimeFilter::Today.next(), TimeFilter::ThreeDays);
        assert_eq!(TimeFilter::ThreeDays.next(), TimeFilter::SevenDays);
        assert_eq!(TimeFilter::SevenDays.next(), TimeFilter::All);
    }

    #[test]
    fn within_time_filter_uses_local_midnight_for_today_and_rolling_windows_otherwise() {
        let now = Local
            .with_ymd_and_hms(2026, 9, 26, 10, 0, 0)
            .unwrap()
            .timestamp_millis();
        assert!(within_time_filter(now - 9 * HOUR, TimeFilter::Today, now)); // 01:00 today
        assert!(!within_time_filter(now - 11 * HOUR, TimeFilter::Today, now)); // 23:00 yesterday
        assert!(within_time_filter(now - 70 * HOUR, TimeFilter::ThreeDays, now));
        assert!(!within_time_filter(now - 80 * HOUR, TimeFilter::ThreeDays, now));
        assert!(within_time_filter(now - 160 * HOUR, TimeFilter::SevenDays, now));
        assert!(within_time_filter(0, TimeFilter::All, now));
    }

    #[test]
    fn an_empty_status_filter_matches_everything() {
        let both = [StatusCategory::Running, StatusCategory::Waiting];
        assert!(matches_status_filter(StatusCategory::Stopped, &[]));
        assert!(!matches_status_filter(StatusCategory::Stopped, &both));
        assert!(matches_status_filter(StatusCategory::Waiting, &both));
    }

    #[test]
    fn filter_keys_map_to_categories_and_toggle() {
        assert_eq!(filter_key_category("!"), Some(StatusCategory::Running));
        assert_eq!(filter_key_category("~"), Some(StatusCategory::Stopped));
        assert_eq!(filter_key_category("x"), None);
        let mut filter = vec![];
        toggle_status_filter(&mut filter, StatusCategory::Idle);
        toggle_status_filter(&mut filter, StatusCategory::Error);
        toggle_status_filter(&mut filter, StatusCategory::Idle);
        assert_eq!(filter, [StatusCategory::Error]);
    }
}
