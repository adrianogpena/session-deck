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
    /// Percent of the 7d quota spent that day.
    pub percent: f64,
}

/// This calendar week's (Monday through `today`, weekends included) day-by-day spend against the 7d
/// quota, from a `date -> cumulative percent used` history. A day's spend is the jump from the
/// previous recorded day's reading, clamped at 0 since the 7d window is rolling (usage can age out).
/// Days with no recorded reading are left out, so gaps fold into the next recorded day's delta.
pub fn seven_day_daily_spend(history: &HashMap<String, f64>, today: NaiveDate) -> Vec<DailySpend> {
    let monday = today - Days::new(u64::from(today.weekday().num_days_from_monday()));
    let mut days = Vec::new();
    let mut prev_percent = 0.0;
    for d in monday.iter_days().take_while(|d| *d <= today) {
        if let Some(&value) = history.get(&local_date_key(d)) {
            days.push(DailySpend {
                label: weekday_label(d),
                percent: (value - prev_percent).max(0.0),
            });
            prev_percent = value;
        }
    }
    days
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

    fn spend(label: &'static str, percent: f64) -> DailySpend {
        DailySpend { label, percent }
    }

    const WED: (i32, u32, u32) = (2026, 1, 7);

    #[test]
    fn local_date_key_is_zero_padded() {
        assert_eq!(local_date_key(day(2026, 1, 7)), "2026-01-07");
        assert_eq!(local_date_key(day(2026, 11, 30)), "2026-11-30");
    }

    #[test]
    fn cumulative_readings_become_per_day_deltas_with_monday_baselined_at_0() {
        let h = history(&[("2026-01-05", 12.0), ("2026-01-06", 20.0), ("2026-01-07", 35.0)]);
        assert_eq!(
            seven_day_daily_spend(&h, day(WED.0, WED.1, WED.2)),
            [spend("Mo", 12.0), spend("Tu", 8.0), spend("We", 15.0)]
        );
    }

    #[test]
    fn stops_at_today() {
        let h = history(&[("2026-01-05", 12.0), ("2026-01-06", 20.0), ("2026-01-08", 50.0)]);
        let labels: Vec<_> = seven_day_daily_spend(&h, day(2026, 1, 5))
            .iter()
            .map(|d| d.label)
            .collect();
        assert_eq!(labels, ["Mo"]);
    }

    #[test]
    fn includes_weekends_and_skips_unrecorded_days() {
        let h = history(&[("2026-01-05", 10.0), ("2026-01-10", 40.0), ("2026-01-11", 55.0)]);
        assert_eq!(
            seven_day_daily_spend(&h, day(2026, 1, 11)),
            [spend("Mo", 10.0), spend("Sa", 30.0), spend("Su", 15.0)]
        );
    }

    #[test]
    fn clamps_a_drop_in_the_rolling_total_to_0() {
        let h = history(&[("2026-01-05", 40.0), ("2026-01-06", 25.0)]);
        assert_eq!(
            seven_day_daily_spend(&h, day(2026, 1, 6)),
            [spend("Mo", 40.0), spend("Tu", 0.0)]
        );
    }

    #[test]
    fn empty_history_gives_no_days() {
        assert!(seven_day_daily_spend(&HashMap::new(), day(WED.0, WED.1, WED.2)).is_empty());
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
