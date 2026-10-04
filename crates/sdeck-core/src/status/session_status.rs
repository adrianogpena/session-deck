use std::fs::{self, OpenOptions};
use std::io;
use std::path::PathBuf;

use crate::commands::session_id::is_safe_session_id;
use crate::format::now_ms;
use crate::paths;
use crate::store::deck_store::{DeckStore, Patch, SessionPatch};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatus {
    Running,
    Waiting,
    Done,
    Error,
}

impl SessionStatus {
    pub const ALL: [SessionStatus; 4] = [Self::Running, Self::Waiting, Self::Done, Self::Error];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Done => "done",
            Self::Error => "error",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_str() == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionStatusRecord {
    pub status: SessionStatus,
    pub updated_at: i64,
}

/// One small JSON status file per session id, regardless of agent: sdeck's own scratch cache.
pub fn session_status_dir() -> PathBuf {
    paths::status_dir()
}

pub fn ensure_session_status_dir() -> io::Result<()> {
    fs::create_dir_all(session_status_dir())
}

fn status_file(session_id: &str) -> PathBuf {
    session_status_dir().join(format!("{session_id}.json"))
}

fn is_notified_marker(name: &str, session_id: &str) -> bool {
    name.starts_with(&format!("{session_id}.")) && name.ends_with(".notified")
}

/// Removes a session's status file and notification markers. Idempotent and best-effort.
pub fn clear_session_status(session_id: &str) {
    if !is_safe_session_id(session_id) {
        return; // not a real session id: nothing of ours to clear
    }
    let _ = fs::remove_file(status_file(session_id));
    if let Ok(entries) = fs::read_dir(session_status_dir()) {
        for entry in entries.flatten() {
            if is_notified_marker(&entry.file_name().to_string_lossy(), session_id) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

/// Writes a session's status directly: the general case `mark_session_error` and the Copilot status
/// watcher build on. Silently refuses an unsafe id rather than write outside the status dir.
pub fn write_session_status(session_id: &str, status: SessionStatus) -> io::Result<()> {
    if !is_safe_session_id(session_id) {
        return Ok(());
    }
    ensure_session_status_dir()?;
    let body = format!(r#"{{"status":"{}","updatedAt":{}}}"#, status.as_str(), now_ms());
    fs::write(status_file(session_id), body)
}

/// Inferred from the terminal command's exit code: neither agent's live status has a distinct
/// "error" value.
pub fn mark_session_error(session_id: &str) -> io::Result<()> {
    write_session_status(session_id, SessionStatus::Error)
}

/// `None` means "no status": nothing has happened yet, or the process already exited.
pub fn read_session_status(session_id: &str) -> Option<SessionStatusRecord> {
    if !is_safe_session_id(session_id) {
        return None;
    }
    // A missing file or a transient partial write racing a read both read as "no status".
    let text = fs::read_to_string(status_file(session_id)).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&text).ok()?;
    let status = SessionStatus::parse(parsed.get("status")?.as_str()?)?;
    let updated_at = parsed
        .get("updatedAt")
        .and_then(serde_json::Value::as_f64)
        .map_or(0, |f| f as i64);
    Some(SessionStatusRecord { status, updated_at })
}

/// "done" and "error" wait to be seen; "running" and "waiting" are current states, never "seen".
fn needs_seeing(status: Option<SessionStatusRecord>) -> Option<SessionStatusRecord> {
    status.filter(|s| matches!(s.status, SessionStatus::Done | SessionStatus::Error))
}

fn seen_at(store: &DeckStore, session_id: &str) -> Option<i64> {
    store.get_session(session_id).and_then(|s| s.seen_at)
}

/// Marks the session's current "done"/"error" as seen, by its exact `updated_at`. Kept in the shared
/// store, so it survives restarts. "waiting" isn't tracked: it only clears via a real new status.
pub fn acknowledge_session_status(store: &DeckStore, session_id: &str) -> io::Result<()> {
    if let Some(status) = needs_seeing(read_session_status(session_id)) {
        if seen_at(store, session_id) != Some(status.updated_at) {
            let patch = SessionPatch {
                seen_at: Patch::Set(status.updated_at),
                ..Default::default()
            };
            store.update_session(session_id, &patch)?;
        }
    }
    Ok(())
}

/// Whether the session has a "done"/"error" the user hasn't seen yet.
pub fn is_session_status_unseen(store: &DeckStore, session_id: &str) -> bool {
    needs_seeing(read_session_status(session_id))
        .is_some_and(|s| seen_at(store, session_id) != Some(s.updated_at))
}

/// "Mark as unread": a seen "done"/"error" becomes unseen again. A session with no status at all
/// gets a fresh "done"; one that's running or waiting is already asking for attention.
pub fn mark_session_unseen(store: &DeckStore, session_id: &str) -> io::Result<()> {
    let status = read_session_status(session_id);
    if needs_seeing(status).is_some() {
        let patch = SessionPatch {
            seen_at: Patch::Clear,
            ..Default::default()
        };
        store.update_session(session_id, &patch)
    } else if status.is_none() {
        write_session_status(session_id, SessionStatus::Done)
    } else {
        Ok(())
    }
}

/// What the UI should actually display: a seen "done"/"error" reads as no status.
pub fn read_effective_session_status(store: &DeckStore, session_id: &str) -> Option<SessionStatusRecord> {
    let status = read_session_status(session_id);
    if needs_seeing(status).is_some_and(|s| seen_at(store, session_id) == Some(s.updated_at)) {
        return None;
    }
    status
}

/// Claims the right to notify about this exact status (`updated_at`), once across every process.
/// Creating the marker file is atomic (`create_new`), so exactly one caller gets `true`. Older
/// markers for the session are removed.
pub fn claim_notification(session_id: &str, updated_at: i64) -> bool {
    if !is_safe_session_id(session_id) || ensure_session_status_dir().is_err() {
        return false;
    }
    let dir = session_status_dir();
    let marker = format!("{session_id}.{updated_at}.notified");
    // Already claimed (or the directory is unwritable): better silent than duplicated.
    if OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dir.join(&marker))
        .is_err()
    {
        return false;
    }
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name != marker && is_notified_marker(&name, session_id) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_env::EnvGuard;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::Duration;

    fn fixture() -> (EnvGuard, tempfile::TempDir, DeckStore) {
        let g = EnvGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("SESSION_DECK_STATUS_DIR", tmp.path().join("status"));
        let store = DeckStore::new(tmp.path().join("state.json"));
        (g, tmp, store)
    }

    fn effective(store: &DeckStore, id: &str) -> Option<SessionStatus> {
        read_effective_session_status(store, id).map(|r| r.status)
    }

    #[test]
    fn done_stays_unseen_until_acknowledged_and_the_mark_is_in_the_store() {
        let (_g, tmp, store) = fixture();
        write_session_status("s1", SessionStatus::Done).unwrap();
        assert!(is_session_status_unseen(&store, "s1"));
        assert_eq!(effective(&store, "s1"), Some(SessionStatus::Done));
        acknowledge_session_status(&store, "s1").unwrap();
        assert!(!is_session_status_unseen(&store, "s1"));
        assert_eq!(effective(&store, "s1"), None);
        let other = DeckStore::new(tmp.path().join("state.json"));
        assert_eq!(
            seen_at(&other, "s1"),
            read_session_status("s1").map(|r| r.updated_at)
        );
    }

    #[test]
    fn a_newer_done_after_the_seen_one_is_unseen_again() {
        let (_g, _tmp, store) = fixture();
        write_session_status("s2", SessionStatus::Done).unwrap();
        acknowledge_session_status(&store, "s2").unwrap();
        thread::sleep(Duration::from_millis(5));
        write_session_status("s2", SessionStatus::Done).unwrap();
        assert!(is_session_status_unseen(&store, "s2"));
    }

    #[test]
    fn waiting_and_running_are_never_acknowledged_away() {
        let (_g, _tmp, store) = fixture();
        write_session_status("s3", SessionStatus::Waiting).unwrap();
        acknowledge_session_status(&store, "s3").unwrap();
        assert_eq!(effective(&store, "s3"), Some(SessionStatus::Waiting));
        assert!(!is_session_status_unseen(&store, "s3"));
    }

    #[test]
    fn mark_unseen_unsees_creates_done_and_leaves_waiting_alone() {
        let (_g, _tmp, store) = fixture();
        write_session_status("s4", SessionStatus::Done).unwrap();
        acknowledge_session_status(&store, "s4").unwrap();
        mark_session_unseen(&store, "s4").unwrap();
        assert!(is_session_status_unseen(&store, "s4"));

        mark_session_unseen(&store, "s5").unwrap();
        assert_eq!(
            read_session_status("s5").map(|r| r.status),
            Some(SessionStatus::Done)
        );
        assert!(is_session_status_unseen(&store, "s5"));

        write_session_status("s6", SessionStatus::Waiting).unwrap();
        mark_session_unseen(&store, "s6").unwrap();
        assert_eq!(
            read_session_status("s6").map(|r| r.status),
            Some(SessionStatus::Waiting)
        );
    }

    fn markers(prefix: &str) -> usize {
        fs::read_dir(session_status_dir())
            .unwrap()
            .flatten()
            .filter(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                n.starts_with(prefix) && n.ends_with(".notified")
            })
            .count()
    }

    #[test]
    fn claim_notification_succeeds_once_per_status_and_clearing_removes_markers() {
        let (_g, _tmp, _store) = fixture();
        write_session_status("s7", SessionStatus::Waiting).unwrap();
        let updated_at = read_session_status("s7").unwrap().updated_at;
        assert!(claim_notification("s7", updated_at));
        assert!(!claim_notification("s7", updated_at));
        assert!(claim_notification("s7", updated_at + 1));
        assert_eq!(markers("s7."), 1);
        clear_session_status("s7");
        assert_eq!(markers("s7."), 0);
        assert_eq!(read_session_status("s7"), None);
    }

    #[test]
    fn threads_claiming_the_same_status_get_exactly_one_true() {
        let (_g, _tmp, _store) = fixture();
        let wins = AtomicUsize::new(0);
        thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    if claim_notification("s8", 1234) {
                        wins.fetch_add(1, Ordering::SeqCst);
                    }
                });
            }
        });
        assert_eq!(wins.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn unsafe_ids_are_refused_everywhere() {
        let (_g, _tmp, store) = fixture();
        write_session_status("../evil", SessionStatus::Done).unwrap();
        assert_eq!(read_session_status("../evil"), None);
        assert!(!claim_notification("../evil", 1));
        assert!(!is_session_status_unseen(&store, "../evil"));
        assert!(!session_status_dir().join("../evil.json").exists());
    }

    #[test]
    fn malformed_status_files_read_as_no_status() {
        let (_g, _tmp, _store) = fixture();
        ensure_session_status_dir().unwrap();
        fs::write(status_file("bad"), r#"{"status":"bogus"}"#).unwrap();
        assert_eq!(read_session_status("bad"), None);
        fs::write(status_file("half"), r#"{"status":"do"#).unwrap();
        assert_eq!(read_session_status("half"), None);
        fs::write(status_file("nots"), r#"{"status":"done"}"#).unwrap();
        assert_eq!(
            read_session_status("nots"),
            Some(SessionStatusRecord {
                status: SessionStatus::Done,
                updated_at: 0
            })
        );
    }
}
