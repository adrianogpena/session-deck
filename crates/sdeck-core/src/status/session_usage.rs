use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use chrono::{DateTime, Days, Local, NaiveDate, TimeZone};
use serde_json::{Map, Value};

use super::session_status::{ensure_session_status_dir, session_status_dir};
use super::usage_display::{local_date_key, seven_day_daily_spend, DailySpend};
use crate::commands::session_id::is_safe_session_id;

/// Context window and rate-limit usage for one Claude session, as last reported by its own
/// statusLine hook (`~/.claude/statusline-command.sh`, which drops this file). The 5h/7d numbers are
/// account-wide; `account_email` tells readings from different accounts apart. Only
/// `context_percent` is specific to this one session.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionUsageRecord {
    pub context_percent: Option<f64>,
    pub five_hour_percent: Option<f64>,
    /// Epoch ms.
    pub five_hour_resets_at: Option<i64>,
    pub seven_day_percent: Option<f64>,
    /// Epoch ms.
    pub seven_day_resets_at: Option<i64>,
    /// Epoch ms, when this record was last written.
    pub updated_at: i64,
    /// Which Claude.ai account was logged in when the statusLine hook wrote this record, if known.
    pub account_email: Option<String>,
}

/// The account-wide part of a [`SessionUsageRecord`].
#[derive(Debug, Clone, PartialEq)]
pub struct RateLimitUsage {
    pub five_hour_percent: Option<f64>,
    pub five_hour_resets_at: Option<i64>,
    pub seven_day_percent: Option<f64>,
    pub seven_day_resets_at: Option<i64>,
    pub updated_at: i64,
}

fn usage_file_path(session_id: &str) -> Option<PathBuf> {
    // Not a real session id: refuse rather than read outside the status dir.
    is_safe_session_id(session_id).then(|| session_status_dir().join(format!("{session_id}.usage.json")))
}

fn num(value: Option<&Value>) -> Option<f64> {
    value.and_then(Value::as_f64).filter(|n| n.is_finite())
}

/// `None` for an unparseable or too-old file missing `updatedAt`.
fn parse_usage_record(raw: &str) -> Option<SessionUsageRecord> {
    let parsed: Value = serde_json::from_str(raw).ok()?;
    let record = parsed.as_object()?;
    let ms = |key: &str| num(record.get(key)).map(|n| n as i64);
    Some(SessionUsageRecord {
        context_percent: num(record.get("contextPercent")),
        five_hour_percent: num(record.get("fiveHourPercent")),
        five_hour_resets_at: ms("fiveHourResetsAt"),
        seven_day_percent: num(record.get("sevenDayPercent")),
        seven_day_resets_at: ms("sevenDayResetsAt"),
        updated_at: ms("updatedAt")?,
        account_email: record
            .get("accountEmail")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
    })
}

/// `None` means no usage has been recorded for this session yet.
pub fn read_session_usage(session_id: &str) -> Option<SessionUsageRecord> {
    // A missing file or a transient partial write racing the read both read as "nothing yet".
    parse_usage_record(&fs::read_to_string(usage_file_path(session_id)?).ok()?)
}

/// The freshest 5h/7d reading for `account_email`, scanning every session's usage file: these
/// numbers are account-wide, so an idle session shouldn't make the quota look emptier than it is.
///
/// Readings tagged with a different account are skipped, so switching accounts never shows the
/// other account's quota. Untagged readings are still considered (excluding them would blank the
/// view rather than attribute them wrong). Also records today's 7d sample for the daily-spend row.
pub fn read_latest_rate_limit_usage(account_email: Option<&str>) -> Option<RateLimitUsage> {
    let entries = fs::read_dir(session_status_dir()).ok()?;
    let mut latest: Option<RateLimitUsage> = None;
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().ends_with(".usage.json") {
            continue;
        }
        // Deleted between listing and reading, or a transient partial write.
        let Some(record) = fs::read_to_string(entry.path())
            .ok()
            .and_then(|raw| parse_usage_record(&raw))
        else {
            continue;
        };
        if record.five_hour_percent.is_none() && record.seven_day_percent.is_none() {
            continue;
        }
        if let (Some(account), Some(tagged)) = (account_email, record.account_email.as_deref()) {
            if tagged != account {
                continue;
            }
        }
        if latest.as_ref().is_none_or(|l| record.updated_at > l.updated_at) {
            latest = Some(RateLimitUsage {
                five_hour_percent: record.five_hour_percent,
                five_hour_resets_at: record.five_hour_resets_at,
                seven_day_percent: record.seven_day_percent,
                seven_day_resets_at: record.seven_day_resets_at,
                updated_at: record.updated_at,
            });
        }
    }
    if let Some(l) = &latest {
        if let (Some(percent), Some(at)) = (
            l.seven_day_percent,
            Local.timestamp_millis_opt(l.updated_at).single(),
        ) {
            record_seven_day_usage_sample(account_email, percent, at);
        }
    }
    latest
}

/// `<status dir>/seven-day-history.json`: `account email -> date -> cumulative 7d percent used`.
fn seven_day_history_path() -> PathBuf {
    session_status_dir().join("seven-day-history.json")
}

/// Key for readings written before the history was scoped by account, or with no known account.
const UNKNOWN_ACCOUNT_KEY: &str = "unknown";

type History = Map<String, Value>;

fn read_seven_day_history_file() -> History {
    let Some(Value::Object(file)) = fs::read_to_string(seven_day_history_path())
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
    else {
        return History::new();
    };
    file.into_iter()
        .filter_map(|(account, dates)| {
            // A pre-account-scoping shape (`date -> percent` at the top level) has nothing to carry over.
            let Value::Object(dates) = dates else {
                return None;
            };
            let per_date: Map<String, Value> = dates
                .into_iter()
                .filter(|(_, v)| num(Some(v)).is_some())
                .collect();
            Some((account, Value::Object(per_date)))
        })
        .collect()
}

/// A bit over a week, so Monday's entry survives through next Sunday.
const SEVEN_DAY_HISTORY_RETENTION_DAYS: u64 = 9;

/// Whole numbers stay integers in the file, as the TS version writes them.
fn json_number(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        Value::from(n as i64)
    } else {
        Value::from(n)
    }
}

/// Persists today's latest 7d reading under `account_email`, so the week's day-by-day spend
/// survives restarts. A no-op when today's stored reading already matches, so frequent polling
/// doesn't turn into frequent disk writes.
pub fn record_seven_day_usage_sample(account_email: Option<&str>, percent: f64, at: DateTime<Local>) {
    let mut file = read_seven_day_history_file();
    let account = account_email.unwrap_or(UNKNOWN_ACCOUNT_KEY).to_string();
    let mut history = match file.remove(&account) {
        Some(Value::Object(h)) => h,
        _ => Map::new(),
    };
    let today = at.date_naive();
    let key = local_date_key(today);
    if num(history.get(&key)) == Some(percent) {
        file.insert(account, Value::Object(history));
        return;
    }
    history.insert(key, json_number(percent));
    let cutoff = local_date_key(today - Days::new(SEVEN_DAY_HISTORY_RETENTION_DAYS));
    history.retain(|k, _| *k >= cutoff);
    file.insert(account, Value::Object(history));
    // Best-effort: a lost sample just means that day shows up blank later.
    if ensure_session_status_dir().is_ok() {
        let _ = fs::write(seven_day_history_path(), Value::Object(file).to_string());
    }
}

/// This week's (Monday through `today`) day-by-day spend against `account_email`'s 7d quota.
pub fn read_seven_day_daily_spend(account_email: Option<&str>, today: NaiveDate) -> Vec<DailySpend> {
    let file = read_seven_day_history_file();
    let history: HashMap<String, f64> = file
        .get(account_email.unwrap_or(UNKNOWN_ACCOUNT_KEY))
        .and_then(Value::as_object)
        .map(|h| {
            h.iter()
                .filter_map(|(k, v)| Some((k.clone(), num(Some(v))?)))
                .collect()
        })
        .unwrap_or_default();
    seven_day_daily_spend(&history, today)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_env::EnvGuard;
    use serde_json::json;

    fn fixture() -> (EnvGuard, tempfile::TempDir) {
        let g = EnvGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("SESSION_DECK_STATUS_DIR", tmp.path().join("status"));
        (g, tmp)
    }

    fn at(y: i32, m: u32, d: u32, h: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
    }

    fn mon() -> DateTime<Local> {
        at(2026, 1, 5, 9)
    }

    fn tue() -> DateTime<Local> {
        at(2026, 1, 6, 9)
    }

    fn spend(label: &'static str, percent: f64) -> DailySpend {
        DailySpend { label, percent }
    }

    fn write_usage(session_id: &str, record: Value) {
        ensure_session_status_dir().unwrap();
        fs::write(usage_file_path(session_id).unwrap(), record.to_string()).unwrap();
    }

    #[test]
    fn a_days_reading_survives_a_restart() {
        let (_g, _tmp) = fixture();
        record_seven_day_usage_sample(None, 12.0, mon());
        record_seven_day_usage_sample(None, 20.0, tue());
        assert_eq!(
            read_seven_day_daily_spend(None, tue().date_naive()),
            [spend("Mo", 12.0), spend("Tu", 8.0)]
        );
    }

    #[test]
    fn unchanged_value_on_the_same_day_does_not_rewrite() {
        let (_g, _tmp) = fixture();
        record_seven_day_usage_sample(None, 30.0, mon());
        let path = seven_day_history_path();
        fs::write(&path, "{\"unknown\":{\"2026-01-05\":30},\"legacy\":1}").unwrap();
        record_seven_day_usage_sample(None, 30.0, mon() + chrono::Duration::minutes(1));
        assert!(fs::read_to_string(&path).unwrap().contains("legacy"));
        record_seven_day_usage_sample(None, 31.0, mon());
        assert!(!fs::read_to_string(&path).unwrap().contains("legacy"));
    }

    #[test]
    fn prunes_readings_outside_the_retention_window() {
        let (_g, _tmp) = fixture();
        record_seven_day_usage_sample(None, 5.0, at(2025, 1, 1, 0));
        record_seven_day_usage_sample(None, 40.0, mon());
        let history: Value =
            serde_json::from_str(&fs::read_to_string(seven_day_history_path()).unwrap()).unwrap();
        assert!(history["unknown"].get("2025-01-01").is_none());
        assert_eq!(history["unknown"]["2026-01-05"], json!(40));
    }

    #[test]
    fn history_is_scoped_per_account() {
        let (_g, _tmp) = fixture();
        let day = mon().date_naive();
        record_seven_day_usage_sample(Some("personal@example.com"), 15.0, mon());
        assert_eq!(
            read_seven_day_daily_spend(Some("personal@example.com"), day),
            [spend("Mo", 15.0)]
        );
        record_seven_day_usage_sample(Some("work@example.com"), 60.0, mon());
        assert_eq!(
            read_seven_day_daily_spend(Some("work@example.com"), day),
            [spend("Mo", 60.0)]
        );
        assert_eq!(
            read_seven_day_daily_spend(Some("personal@example.com"), day),
            [spend("Mo", 15.0)]
        );
    }

    #[test]
    fn reads_a_session_usage_record_and_rejects_bad_ids_or_missing_updated_at() {
        let (_g, _tmp) = fixture();
        write_usage(
            "s1",
            json!({"contextPercent": 42, "updatedAt": 1000, "accountEmail": ""}),
        );
        assert_eq!(
            read_session_usage("s1"),
            Some(SessionUsageRecord {
                context_percent: Some(42.0),
                five_hour_percent: None,
                five_hour_resets_at: None,
                seven_day_percent: None,
                seven_day_resets_at: None,
                updated_at: 1000,
                account_email: None,
            })
        );
        write_usage("s2", json!({"contextPercent": 42}));
        assert_eq!(read_session_usage("s2"), None);
        assert_eq!(read_session_usage("missing"), None);
        assert_eq!(read_session_usage("../x"), None);
    }

    #[test]
    fn latest_rate_limit_skips_other_accounts_and_keeps_untagged() {
        let (_g, _tmp) = fixture();
        write_usage(
            "mine",
            json!({"fiveHourPercent": 10, "sevenDayPercent": 20, "updatedAt": 1000, "accountEmail": "me@x.com"}),
        );
        write_usage(
            "other",
            json!({"fiveHourPercent": 90, "sevenDayPercent": 95, "updatedAt": 3000, "accountEmail": "work@x.com"}),
        );
        write_usage("ctx-only", json!({"contextPercent": 5, "updatedAt": 4000}));
        let latest = read_latest_rate_limit_usage(Some("me@x.com")).unwrap();
        assert_eq!((latest.five_hour_percent, latest.updated_at), (Some(10.0), 1000));

        write_usage("untagged", json!({"fiveHourPercent": 30, "updatedAt": 2000}));
        let latest = read_latest_rate_limit_usage(Some("me@x.com")).unwrap();
        assert_eq!((latest.five_hour_percent, latest.updated_at), (Some(30.0), 2000));

        let latest = read_latest_rate_limit_usage(Some("work@x.com")).unwrap();
        assert_eq!(latest.seven_day_percent, Some(95.0));
    }

    #[test]
    fn latest_rate_limit_records_a_seven_day_sample_under_that_account() {
        let (_g, _tmp) = fixture();
        let updated = tue();
        write_usage(
            "s",
            json!({"sevenDayPercent": 25, "updatedAt": updated.timestamp_millis(), "accountEmail": "me@x.com"}),
        );
        read_latest_rate_limit_usage(Some("me@x.com"));
        assert_eq!(
            read_seven_day_daily_spend(Some("me@x.com"), updated.date_naive()),
            [spend("Tu", 25.0)]
        );
        assert!(read_seven_day_daily_spend(Some("work@x.com"), updated.date_naive()).is_empty());
    }
}
