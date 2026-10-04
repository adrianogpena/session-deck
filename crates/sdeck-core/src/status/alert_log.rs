use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::session_status::{session_status_dir, SessionStatus};
use crate::commands::session_id::is_safe_session_id;

/// Both pitago's plain notification log and z4-oriel's alert center independently settled on this cap.
pub const ALERT_LOG_CAP: usize = 200;

/// One status change, logged whether or not it also fired a desktop toast (see `waiting_notifier`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertEntry {
    pub session_id: String,
    pub status: SessionStatus,
    pub label: String,
    pub project: Option<String>,
    /// The status record's own `updated_at`: also what dedupes the same transition logged twice.
    pub at: i64,
}

impl AlertEntry {
    fn to_json(&self) -> Value {
        let mut value = json!({
            "sessionId": self.session_id,
            "status": self.status.as_str(),
            "label": self.label,
        });
        if let Some(project) = &self.project {
            value["project"] = json!(project);
        }
        value["at"] = json!(self.at);
        value
    }
}

fn alerts_dir() -> PathBuf {
    session_status_dir().join("alerts")
}

/// `None` for a transient partial write racing a read, or a file removed by a concurrent prune.
fn parse_alert(path: &Path) -> Option<AlertEntry> {
    let parsed: Value = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
    let text = |key: &str| parsed.get(key).and_then(Value::as_str).map(str::to_string);
    Some(AlertEntry {
        session_id: text("sessionId")?,
        status: SessionStatus::parse(parsed.get("status")?.as_str()?)?,
        label: text("label")?,
        project: text("project"),
        at: parsed.get("at")?.as_f64()? as i64,
    })
}

fn read_entries(dir: &Path) -> Vec<(PathBuf, AlertEntry)> {
    let Ok(names) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut entries: Vec<_> = names
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            parse_alert(&path).map(|entry| (path, entry))
        })
        .collect();
    entries.sort_by_key(|e| std::cmp::Reverse(e.1.at));
    entries
}

/// Keeps at most [`ALERT_LOG_CAP`] entries, oldest removed first.
fn prune() {
    let dir = alerts_dir();
    if fs::read_dir(&dir).map_or(true, |names| names.count() <= ALERT_LOG_CAP) {
        return;
    }
    for (path, _) in read_entries(&dir).into_iter().skip(ALERT_LOG_CAP) {
        let _ = fs::remove_file(path);
    }
}

/// Appends one status transition to the shared alert history, deduped by session id + `at` (the
/// atomic `create_new` write lets exactly one caller win). Best-effort: a failure (already logged,
/// or an unwritable directory) is dropped silently.
pub fn append_alert(entry: &AlertEntry) {
    if !is_safe_session_id(&entry.session_id) {
        return;
    }
    let dir = alerts_dir();
    let written = fs::create_dir_all(&dir).and_then(|()| {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.join(format!("{}.{}.json", entry.session_id, entry.at)))?
            .write_all(entry.to_json().to_string().as_bytes())
    });
    if written.is_ok() {
        prune();
    }
}

/// The alert history, newest first, capped at `limit`.
pub fn read_alerts(limit: usize) -> Vec<AlertEntry> {
    read_entries(&alerts_dir())
        .into_iter()
        .take(limit)
        .map(|(_, entry)| entry)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_env::EnvGuard;

    fn fixture() -> (EnvGuard, tempfile::TempDir) {
        let g = EnvGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("SESSION_DECK_STATUS_DIR", tmp.path());
        (g, tmp)
    }

    fn alert(session_id: &str, status: SessionStatus, label: &str, at: i64) -> AlertEntry {
        AlertEntry {
            session_id: session_id.to_string(),
            status,
            label: label.to_string(),
            project: None,
            at,
        }
    }

    #[test]
    fn deduped_by_session_and_at_and_read_newest_first() {
        let (_g, _tmp) = fixture();
        append_alert(&alert("a1", SessionStatus::Waiting, "one", 100));
        append_alert(&alert("a1", SessionStatus::Waiting, "one", 100));
        append_alert(&alert("a1", SessionStatus::Done, "one", 200));
        let entries = read_alerts(ALERT_LOG_CAP);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].status, SessionStatus::Done);
        assert_eq!(entries[1].status, SessionStatus::Waiting);
    }

    #[test]
    fn refuses_an_unsafe_session_id() {
        let (_g, _tmp) = fixture();
        append_alert(&alert("../escape", SessionStatus::Error, "nope", 999));
        assert!(read_alerts(ALERT_LOG_CAP).is_empty());
    }

    #[test]
    fn caps_at_alert_log_cap_dropping_the_oldest_first() {
        let (_g, _tmp) = fixture();
        for i in 0..ALERT_LOG_CAP as i64 + 10 {
            append_alert(&alert("cap", SessionStatus::Waiting, &format!("turn {i}"), i));
        }
        let entries = read_alerts(ALERT_LOG_CAP + 50);
        assert_eq!(entries.len(), ALERT_LOG_CAP);
        assert_eq!(entries[0].at, ALERT_LOG_CAP as i64 + 9);
        assert_eq!(entries[entries.len() - 1].at, 10);
    }

    #[test]
    fn writes_the_ts_json_shape() {
        let (_g, tmp) = fixture();
        let mut entry = alert("s1", SessionStatus::Waiting, "Fix bug", 5);
        entry.project = Some("deck".to_string());
        append_alert(&entry);
        let raw = fs::read_to_string(tmp.path().join("alerts/s1.5.json")).unwrap();
        assert_eq!(
            raw,
            r#"{"sessionId":"s1","status":"waiting","label":"Fix bug","project":"deck","at":5}"#
        );
        assert_eq!(read_alerts(1), [entry]);
    }
}
