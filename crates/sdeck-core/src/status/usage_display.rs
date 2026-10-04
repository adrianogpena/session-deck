//! Pure formatting for the usage rows, so the color thresholds and bar look stay in one place.

use std::collections::HashMap;

use chrono::{DateTime, Datelike, Days, Local, NaiveDate, TimeZone, Weekday};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageMetric {
    Context,
    FiveHour,
    SevenDay,
}

impl UsageMetric {
    pub fn label(self) -> &'static str {
        match self {
            Self::Context => "Context",
            Self::FiveHour => "5h",
            Self::SevenDay => "7d",
        }
    }

    /// `(warning, critical)`. Context fills up fastest and is cheapest to act on (compact/clear), so
    /// it turns warning/critical earliest. 5h is next most urgent; 7d is the least.
    fn thresholds(self) -> (f64, f64) {
        match self {
            Self::Context => (20.0, 50.0),
            Self::FiveHour => (50.0, 80.0),
            Self::SevenDay => (70.0, 90.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageSeverity {
    Ok,
    Warning,
    Critical,
}

pub fn usage_severity(metric: UsageMetric, percent: f64) -> UsageSeverity {
    let (warning, critical) = metric.thresholds();
    if percent >= critical {
        UsageSeverity::Critical
    } else if percent >= warning {
        UsageSeverity::Warning
    } else {
        UsageSeverity::Ok
    }
}

/// Characters wide a usage bar is drawn at, regardless of percent: just the fill point moves.
pub const USAGE_BAR_WIDTH: usize = 10;

pub fn render_usage_bar(percent: f64, width: usize) -> String {
    let clamped = percent.clamp(0.0, 100.0);
    let filled = ((clamped / 100.0) * width as f64).round() as usize;
    "█".repeat(filled) + &"░".repeat(width - filled)
}

/// `YYYY-MM-DD`: the key the seven-day history is indexed by.
pub fn local_date_key(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

fn weekday_label(d: NaiveDate) -> &'static str {
    match d.weekday() {
        Weekday::Mon => "Mo",
        Weekday::Tue => "Tu",
        Weekday::Wed => "We",
        Weekday::Thu => "Th",
        Weekday::Fri => "Fr",
        Weekday::Sat => "Sa",
        Weekday::Sun => "Su",
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DailySpend {
    /// Two-letter weekday abbreviation: Mo, Tu, We, Th, Fr, Sa, Su.
    pub label: &'static str,
    /// Percent of the 7d quota spent that day; `None` when no reading was recorded that day.
    pub percent: Option<f64>,
}

/// The last `days` calendar days ending at `today`, most recent first, from a `date -> percent of
/// the 7d quota spent that day` map. Days with nothing recorded stay in the list with no percent.
pub fn recent_daily_spend(spent: &HashMap<String, f64>, today: NaiveDate, days: u64) -> Vec<DailySpend> {
    (0..days)
        .filter_map(|back| today.checked_sub_days(Days::new(back)))
        .map(|d| DailySpend {
            label: weekday_label(d),
            percent: spent.get(&local_date_key(d)).copied(),
        })
        .collect()
}

/// What a 7d reading adds to the day it was taken on, given the previous reading: the increase, or
/// the whole new reading when it went down (the quota reset, so usage restarted from 0). With no
/// previous reading there is nothing to compare against, so nothing is attributed.
pub fn spend_since(previous: Option<f64>, current: f64) -> f64 {
    match previous {
        None => 0.0,
        Some(prev) if current >= prev => current - prev,
        Some(_) => current,
    }
}

/// Day-by-day spend rebuilt from end-of-day cumulative readings (`date -> percent`), for history
/// recorded before spend was tracked per sample. Also returns the latest reading.
pub fn spend_from_cumulative(history: &HashMap<String, f64>) -> (HashMap<String, f64>, Option<f64>) {
    let mut dates: Vec<&String> = history.keys().collect();
    dates.sort();
    let mut spent = HashMap::new();
    let mut previous = None;
    for date in dates {
        let value = history[date];
        spent.insert(date.clone(), spend_since(previous, value));
        previous = Some(value);
    }
    (spent, previous)
}

/// `8:30 PM`, or `20:30` with `use_24_hour`. `with_weekday` prefixes `Mon ` (the 7d quota's reset
/// can be days out, so a bare time of day is ambiguous there).
pub fn format_reset_time(epoch_ms: i64, use_24_hour: bool, with_weekday: bool) -> String {
    let Some(date) = Local.timestamp_millis_opt(epoch_ms).single() else {
        return String::new();
    };
    format_reset_time_at(date, use_24_hour, with_weekday)
}

fn format_reset_time_at(date: DateTime<Local>, use_24_hour: bool, with_weekday: bool) -> String {
    let weekday = if with_weekday { "%a " } else { "" };
    let time = if use_24_hour { "%H:%M" } else { "%-I:%M %p" };
    date.format(&format!("{weekday}{time}")).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn history(entries: &[(&str, f64)]) -> HashMap<String, f64> {
        entries.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn local_date_key_is_zero_padded() {
        assert_eq!(local_date_key(day(2026, 1, 7)), "2026-01-07");
        assert_eq!(local_date_key(day(2026, 11, 30)), "2026-11-30");
    }

    #[test]
    fn lists_the_requested_days_newest_first_marking_unrecorded_ones() {
        let h = history(&[("2026-01-06", 20.0), ("2026-01-04", 3.5)]);
        let got: Vec<_> = recent_daily_spend(&h, day(2026, 1, 7), 5)
            .into_iter()
            .map(|d| (d.label, d.percent))
            .collect();
        assert_eq!(
            got,
            [
                ("We", None),
                ("Tu", Some(20.0)),
                ("Mo", None),
                ("Su", Some(3.5)),
                ("Sa", None)
            ]
        );
    }

    #[test]
    fn an_increase_is_spend_and_a_drop_is_a_reset_counting_the_new_reading() {
        assert_eq!(spend_since(Some(10.0), 22.0), 12.0);
        assert_eq!(spend_since(Some(70.0), 5.0), 5.0);
        assert_eq!(spend_since(Some(5.0), 5.0), 0.0);
        assert_eq!(spend_since(None, 40.0), 0.0);
    }

    #[test]
    fn cumulative_history_is_converted_to_per_day_spend() {
        let h = history(&[("2026-01-05", 12.0), ("2026-01-06", 20.0), ("2026-01-07", 4.0)]);
        let (spent, last) = spend_from_cumulative(&h);
        assert_eq!(last, Some(4.0));
        assert_eq!(spent["2026-01-05"], 0.0);
        assert_eq!(spent["2026-01-06"], 8.0);
        assert_eq!(spent["2026-01-07"], 4.0);
        assert_eq!(spend_from_cumulative(&HashMap::new()), (HashMap::new(), None));
    }

    #[test]
    fn severity_thresholds_per_metric() {
        assert_eq!(usage_severity(UsageMetric::Context, 19.0), UsageSeverity::Ok);
        assert_eq!(usage_severity(UsageMetric::Context, 20.0), UsageSeverity::Warning);
        assert_eq!(
            usage_severity(UsageMetric::Context, 50.0),
            UsageSeverity::Critical
        );
        assert_eq!(
            usage_severity(UsageMetric::FiveHour, 79.0),
            UsageSeverity::Warning
        );
        assert_eq!(usage_severity(UsageMetric::SevenDay, 69.0), UsageSeverity::Ok);
        assert_eq!(
            usage_severity(UsageMetric::SevenDay, 90.0),
            UsageSeverity::Critical
        );
    }

    #[test]
    fn usage_bar_clamps_and_rounds() {
        assert_eq!(render_usage_bar(0.0, USAGE_BAR_WIDTH), "░".repeat(10));
        assert_eq!(
            render_usage_bar(45.0, USAGE_BAR_WIDTH),
            "█".repeat(5) + &"░".repeat(5)
        );
        assert_eq!(render_usage_bar(150.0, 4), "████");
        assert_eq!(render_usage_bar(-5.0, 4), "░░░░");
    }

    #[test]
    fn reset_time_formats() {
        let at = Local.with_ymd_and_hms(2026, 1, 5, 20, 30, 0).unwrap();
        assert_eq!(format_reset_time_at(at, false, false), "8:30 PM");
        assert_eq!(format_reset_time_at(at, true, false), "20:30");
        assert_eq!(format_reset_time_at(at, false, true), "Mon 8:30 PM");
        assert_eq!(format_reset_time(at.timestamp_millis(), true, true), "Mon 20:30");
    }
}
