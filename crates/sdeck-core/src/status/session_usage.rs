use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Days, Local, NaiveDate, TimeZone};
use serde_json::{Map, Value};

use super::session_status::{ensure_session_status_dir, session_status_dir};
use super::usage_display::{
    local_date_key, recent_daily_spend, spend_from_cumulative, spend_since, DailySpend,
};
use crate::commands::session_id::is_safe_session_id;

/// Context window and rate-limit usage for one Claude session, as last reported by its own
/// statusLine hook (`~/.claude/statusline-command.sh`, which drops this file). The 5h/7d numbers are
/// account-wide; `account_email` tells readings from different accounts apart. Only
/// `context_percent` is specific to this one session.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionUsageRecord {
    pub context_percent: Option<f64>,
    /// The session's first context reading: what a request carries before any real conversation.
    pub startup_context_percent: Option<f64>,
    /// The model's display name and reasoning effort, as of the last statusLine report.
    pub model: Option<String>,
    pub effort: Option<String>,
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

/// How long after its last statusLine report a 5h/7d reading is drawn dimmed.
pub const RATE_LIMIT_STALE_AFTER_MS: i64 = 15 * 60 * 1000;

impl RateLimitUsage {
    /// Drops a window whose reset time has passed: its percentage belongs to the previous window,
    /// so showing it would overstate what's used.
    pub fn without_expired(mut self, now_ms: i64) -> Self {
        if self.five_hour_resets_at.is_some_and(|at| at <= now_ms) {
            self.five_hour_percent = None;
            self.five_hour_resets_at = None;
        }
        if self.seven_day_resets_at.is_some_and(|at| at <= now_ms) {
            self.seven_day_percent = None;
            self.seven_day_resets_at = None;
        }
        self
    }
}

/// Whether a 5h/7d reading last reported at `updated_at` (epoch ms) is old enough to draw dimmed.
pub fn is_rate_limit_stale(updated_at: i64, now_ms: i64) -> bool {
    now_ms - updated_at > RATE_LIMIT_STALE_AFTER_MS
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
    let text = |key: &str| {
        record
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    Some(SessionUsageRecord {
        context_percent: num(record.get("contextPercent")),
        startup_context_percent: num(record.get("startupContextPercent")),
        model: text("model"),
        effort: text("effort"),
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

/// Re-parses a session's usage file only when its modification time or size changed, so polling the
/// whole status dir stays a `read_dir` plus metadata calls instead of reading every file.
#[derive(Default)]
pub struct RateLimitScanner {
    cache: HashMap<PathBuf, (SystemTime, u64, Option<SessionUsageRecord>)>,
}

impl RateLimitScanner {
    /// The freshest 5h/7d reading for `account_email`, across every session's usage file: these
    /// numbers are account-wide, so an idle session shouldn't make the quota look emptier than it is.
    ///
    /// Readings tagged with a different account are skipped, so switching accounts never shows the
    /// other account's quota. Untagged readings are still considered (excluding them would blank the
    /// view rather than attribute them wrong). Also records today's 7d sample for the daily-spend row.
    pub fn latest(&mut self, account_email: Option<&str>) -> Option<RateLimitUsage> {
        let entries = fs::read_dir(session_status_dir()).ok()?;
        let mut seen = HashSet::new();
        let mut latest: Option<RateLimitUsage> = None;
        for entry in entries.flatten() {
            if !entry.file_name().to_string_lossy().ends_with(".usage.json") {
                continue;
            }
            let path = entry.path();
            // Deleted between listing and reading.
            let Some((modified, len)) = entry
                .metadata()
                .ok()
                .and_then(|m| Some((m.modified().ok()?, m.len())))
            else {
                continue;
            };
            seen.insert(path.clone());
            let unchanged = self
                .cache
                .get(&path)
                .is_some_and(|(at, size, _)| *at == modified && *size == len);
            if !unchanged {
                // A transient partial write reads as "nothing" until the next change.
                let record = fs::read_to_string(&path)
                    .ok()
                    .and_then(|raw| parse_usage_record(&raw));
                self.cache.insert(path.clone(), (modified, len, record));
            }
            let Some(record) = self.cache.get(&path).and_then(|(_, _, r)| r.as_ref()) else {
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
        self.cache.retain(|path, _| seen.contains(path));
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
}

/// One-off [`RateLimitScanner::latest`] with no cache.
pub fn read_latest_rate_limit_usage(account_email: Option<&str>) -> Option<RateLimitUsage> {
    RateLimitScanner::default().latest(account_email)
}

/// A bit over the 7d history retention: a session idle longer than this has nothing left worth showing.
const USAGE_FILE_MAX_AGE: Duration = Duration::from_secs(9 * 24 * 60 * 60);

/// Deletes per-session usage files not written for [`USAGE_FILE_MAX_AGE`]. A live session's
/// statusLine rewrites its file, so only abandoned sessions go. Best-effort.
pub fn prune_stale_usage_files() {
    let Ok(entries) = fs::read_dir(session_status_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().ends_with(".usage.json") {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|at| at.elapsed().ok())
            .is_some_and(|age| age > USAGE_FILE_MAX_AGE);
        if stale {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// `<status dir>/seven-day-history.json`:
/// `account email -> {"last": latest 7d percent seen, "days": {date: percent of the quota spent}}`.
fn seven_day_history_path() -> PathBuf {
    session_status_dir().join("seven-day-history.json")
}

/// Key for readings with no known account.
const UNKNOWN_ACCOUNT_KEY: &str = "unknown";

/// A bit over a week, so a few days past the window stay on file.
const SEVEN_DAY_HISTORY_RETENTION_DAYS: u64 = 9;

#[derive(Debug, Default)]
struct AccountSpend {
    last: Option<f64>,
    days: HashMap<String, f64>,
}

type History = HashMap<String, AccountSpend>;

/// Reads the history file. The earlier shape (`account -> date -> cumulative percent`) is converted
/// on the fly; anything unreadable is dropped.
fn read_seven_day_history_file() -> History {
    let Some(Value::Object(file)) = fs::read_to_string(seven_day_history_path())
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
    else {
        return History::new();
    };
    file.into_iter()
        .filter_map(|(account, entry)| {
            let Value::Object(entry) = entry else {
                return None;
            };
            let numbers = |m: &Map<String, Value>| -> HashMap<String, f64> {
                m.iter()
                    .filter_map(|(k, v)| Some((k.clone(), num(Some(v))?)))
                    .collect()
            };
            if let Some(Value::Object(days)) = entry.get("days") {
                return Some((
                    account,
                    AccountSpend {
                        last: num(entry.get("last")),
                        days: numbers(days),
                    },
                ));
            }
            let (days, last) = spend_from_cumulative(&numbers(&entry));
            Some((account, AccountSpend { last, days }))
        })
        .collect()
}

/// Whole numbers stay integers in the file.
fn json_number(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        Value::from(n as i64)
    } else {
        Value::from(n)
    }
}

/// Adds what the latest 7d reading spent to its day under `account_email`, so the day-by-day spend
/// survives restarts. A no-op when the reading matches the last one seen, so frequent polling
/// doesn't turn into frequent disk writes.
pub fn record_seven_day_usage_sample(account_email: Option<&str>, percent: f64, at: DateTime<Local>) {
    let mut file = read_seven_day_history_file();
    let account = file
        .entry(account_email.unwrap_or(UNKNOWN_ACCOUNT_KEY).to_string())
        .or_default();
    if account.last == Some(percent) {
        return;
    }
    let today = at.date_naive();
    let spent = spend_since(account.last, percent);
    *account.days.entry(local_date_key(today)).or_insert(0.0) += spent;
    account.last = Some(percent);
    let cutoff = local_date_key(today - Days::new(SEVEN_DAY_HISTORY_RETENTION_DAYS));
    account.days.retain(|k, _| *k >= cutoff);
    let out: Map<String, Value> = file
        .into_iter()
        .map(|(account, a)| {
            let days: Map<String, Value> = a.days.into_iter().map(|(k, v)| (k, json_number(v))).collect();
            let mut entry = Map::new();
            if let Some(last) = a.last {
                entry.insert("last".into(), json_number(last));
            }
            entry.insert("days".into(), Value::Object(days));
            (account, Value::Object(entry))
        })
        .collect();
    // Best-effort: a lost sample just means that day's spend is a little low.
    if ensure_session_status_dir().is_ok() {
        let _ = fs::write(seven_day_history_path(), Value::Object(out).to_string());
    }
}

/// The last `days` days' (most recent first) spend against `account_email`'s 7d quota.
pub fn read_recent_daily_spend(account_email: Option<&str>, today: NaiveDate, days: u64) -> Vec<DailySpend> {
    let mut file = read_seven_day_history_file();
    let spent = file
        .remove(account_email.unwrap_or(UNKNOWN_ACCOUNT_KEY))
        .map(|a| a.days)
        .unwrap_or_default();
    recent_daily_spend(&spent, today, days)
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

    fn recorded(account: Option<&str>, today: NaiveDate) -> Vec<(&'static str, f64)> {
        read_recent_daily_spend(account, today, 5)
            .into_iter()
            .filter_map(|d| Some((d.label, d.percent?)))
            .collect()
    }

    fn write_usage(session_id: &str, record: Value) {
        ensure_session_status_dir().unwrap();
        fs::write(usage_file_path(session_id).unwrap(), record.to_string()).unwrap();
    }

    #[test]
    fn spend_accumulates_per_sample_and_survives_a_restart() {
        let (_g, _tmp) = fixture();
        record_seven_day_usage_sample(None, 12.0, mon());
        record_seven_day_usage_sample(None, 20.0, mon() + chrono::Duration::hours(2));
        record_seven_day_usage_sample(None, 23.0, tue());
        assert_eq!(recorded(None, tue().date_naive()), [("Tu", 3.0), ("Mo", 8.0)]);
    }

    #[test]
    fn a_reset_counts_the_new_reading_as_that_days_spend() {
        let (_g, _tmp) = fixture();
        record_seven_day_usage_sample(None, 68.0, mon());
        record_seven_day_usage_sample(None, 70.0, mon() + chrono::Duration::hours(1));
        record_seven_day_usage_sample(None, 5.0, tue());
        record_seven_day_usage_sample(None, 9.0, tue() + chrono::Duration::hours(1));
        assert_eq!(recorded(None, tue().date_naive()), [("Tu", 9.0), ("Mo", 2.0)]);
    }

    #[test]
    fn the_earlier_cumulative_history_shape_is_converted() {
        let (_g, _tmp) = fixture();
        ensure_session_status_dir().unwrap();
        fs::write(
            seven_day_history_path(),
            json!({"unknown": {"2026-01-05": 12, "2026-01-06": 20}, "legacy": 1}).to_string(),
        )
        .unwrap();
        assert_eq!(recorded(None, tue().date_naive()), [("Tu", 8.0), ("Mo", 0.0)]);
        record_seven_day_usage_sample(None, 25.0, tue());
        assert_eq!(recorded(None, tue().date_naive()), [("Tu", 13.0), ("Mo", 0.0)]);
    }

    #[test]
    fn unchanged_value_does_not_rewrite() {
        let (_g, _tmp) = fixture();
        record_seven_day_usage_sample(None, 30.0, mon());
        let path = seven_day_history_path();
        fs::write(
            &path,
            "{\"unknown\":{\"last\":30,\"days\":{}},\"marker\":{\"days\":{}}}",
        )
        .unwrap();
        record_seven_day_usage_sample(None, 30.0, mon() + chrono::Duration::minutes(1));
        assert!(fs::read_to_string(&path).unwrap().contains("marker"));
    }

    #[test]
    fn prunes_days_outside_the_retention_window() {
        let (_g, _tmp) = fixture();
        record_seven_day_usage_sample(None, 5.0, at(2025, 1, 1, 0));
        record_seven_day_usage_sample(None, 40.0, at(2025, 1, 1, 1));
        record_seven_day_usage_sample(None, 45.0, mon());
        let history: Value =
            serde_json::from_str(&fs::read_to_string(seven_day_history_path()).unwrap()).unwrap();
        assert!(history["unknown"]["days"].get("2025-01-01").is_none());
        assert_eq!(history["unknown"]["days"]["2026-01-05"], json!(5));
    }

    #[test]
    fn history_is_scoped_per_account() {
        let (_g, _tmp) = fixture();
        let day = mon().date_naive();
        let later = mon() + chrono::Duration::hours(1);
        record_seven_day_usage_sample(Some("personal@example.com"), 10.0, mon());
        record_seven_day_usage_sample(Some("personal@example.com"), 15.0, later);
        record_seven_day_usage_sample(Some("work@example.com"), 50.0, mon());
        record_seven_day_usage_sample(Some("work@example.com"), 60.0, later);
        assert_eq!(recorded(Some("personal@example.com"), day), [("Mo", 5.0)]);
        assert_eq!(recorded(Some("work@example.com"), day), [("Mo", 10.0)]);
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
                startup_context_percent: None,
                model: None,
                effort: None,
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
        let record = |percent: u32, at: DateTime<Local>| {
            write_usage(
                "s",
                json!({"sevenDayPercent": percent, "updatedAt": at.timestamp_millis(), "accountEmail": "me@x.com"}),
            );
            read_latest_rate_limit_usage(Some("me@x.com"));
        };
        record(20, updated);
        record(25, updated + chrono::Duration::hours(1));
        assert_eq!(recorded(Some("me@x.com"), updated.date_naive()), [("Tu", 5.0)]);
        assert!(recorded(Some("work@x.com"), updated.date_naive()).is_empty());
    }

    #[test]
    fn scanner_picks_up_changed_and_removed_files() {
        let (_g, _tmp) = fixture();
        let mut scanner = RateLimitScanner::default();
        write_usage("a", json!({"fiveHourPercent": 10, "updatedAt": 1000}));
        assert_eq!(scanner.latest(None).unwrap().five_hour_percent, Some(10.0));
        write_usage("a", json!({"fiveHourPercent": 55, "updatedAt": 2000}));
        assert_eq!(scanner.latest(None).unwrap().five_hour_percent, Some(55.0));
        fs::remove_file(usage_file_path("a").unwrap()).unwrap();
        assert!(scanner.latest(None).is_none());
    }

    #[test]
    fn prune_removes_only_stale_usage_files() {
        let (_g, _tmp) = fixture();
        write_usage("old", json!({"updatedAt": 1}));
        write_usage("new", json!({"updatedAt": 2}));
        let old = usage_file_path("old").unwrap();
        let stale = SystemTime::now() - USAGE_FILE_MAX_AGE - Duration::from_secs(60);
        fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(stale)
            .unwrap();
        fs::write(session_status_dir().join("keep.json"), "{}").unwrap();
        prune_stale_usage_files();
        assert!(!old.exists());
        assert!(usage_file_path("new").unwrap().exists());
        assert!(session_status_dir().join("keep.json").exists());
    }

    #[test]
    fn expired_windows_are_dropped_and_live_ones_kept() {
        let usage = RateLimitUsage {
            five_hour_percent: Some(80.0),
            five_hour_resets_at: Some(1000),
            seven_day_percent: Some(30.0),
            seven_day_resets_at: Some(5000),
            updated_at: 10,
        };
        let shown = usage.without_expired(1000);
        assert_eq!((shown.five_hour_percent, shown.five_hour_resets_at), (None, None));
        assert_eq!(shown.seven_day_percent, Some(30.0));
    }

    #[test]
    fn staleness_starts_after_the_threshold() {
        assert!(!is_rate_limit_stale(0, RATE_LIMIT_STALE_AFTER_MS));
        assert!(is_rate_limit_stale(0, RATE_LIMIT_STALE_AFTER_MS + 1));
    }
}
