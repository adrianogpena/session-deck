use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags, Row};

use crate::commands::session_id::assert_safe_session_id;
use crate::paths;

fn session_store_path() -> PathBuf {
    paths::copilot_dir().join("session-store.db")
}

/// Copilot CLI's own live per-session activity log. Internal, undocumented format: best-effort.
/// `Err` for an id that isn't safe to build into a path.
pub fn copilot_session_events_log_path(session_id: &str) -> Result<PathBuf, String> {
    let id = assert_safe_session_id(session_id)?;
    Ok(paths::copilot_dir()
        .join("session-state")
        .join(id)
        .join("events.jsonl"))
}

pub fn copilot_store_exists() -> bool {
    session_store_path().exists()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopilotSessionRow {
    pub id: String,
    pub cwd: String,
    pub repository: Option<String>,
    pub branch: Option<String>,
    pub summary: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopilotTurn {
    pub turn_index: i64,
    pub user_message: Option<String>,
    pub assistant_response: Option<String>,
    pub timestamp: String,
}

/// Fresh read-only connection per call, not cached: `copilot` writes this DB in WAL mode, so a stale
/// connection could miss newer sessions. Opens the real path in place, never a copy: a copy misses
/// rows still sitting in the `-wal` file. Session Deck never writes Copilot's DB. Any DB error reads
/// as "nothing there".
fn with_db<T>(f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Option<T> {
    let conn = Connection::open_with_flags(
        session_store_path(),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    f(&conn).ok()
}

/// A column as text whatever its stored type; `None` for NULL.
fn text(row: &Row, index: usize) -> rusqlite::Result<Option<String>> {
    Ok(match row.get_ref(index)? {
        ValueRef::Null => None,
        ValueRef::Integer(i) => Some(i.to_string()),
        ValueRef::Real(f) => Some(f.to_string()),
        ValueRef::Text(t) => Some(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => Some(String::from_utf8_lossy(b).into_owned()),
    })
}

/// Every real Copilot CLI session, newest first. "Real" excludes a session with no prompt yet:
/// unlike Claude Code, Copilot inserts a `sessions` row immediately at startup, before anything is
/// typed. Filtered via events.jsonl, not a SQL condition on `turns`: `turns` only gets written once
/// the `copilot` process exits, so it's not a live signal.
pub fn list_copilot_sessions() -> Vec<CopilotSessionRow> {
    if !copilot_store_exists() {
        return Vec::new();
    }
    let rows = with_db(|db| {
        let mut stmt = db.prepare(
            "SELECT id, cwd, repository, branch, summary, created_at, updated_at \
             FROM sessions ORDER BY updated_at DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(CopilotSessionRow {
                id: text(r, 0)?.unwrap_or_default(),
                cwd: text(r, 1)?.unwrap_or_default(),
                repository: text(r, 2)?,
                branch: text(r, 3)?,
                summary: text(r, 4)?,
                created_at: text(r, 5)?.unwrap_or_default(),
                updated_at: text(r, 6)?.unwrap_or_default(),
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
    })
    .unwrap_or_default();
    rows.into_iter()
        .filter(|r| copilot_session_has_started(&r.id))
        .collect()
}

/// Sessions confirmed to have a real `user.message` event: cached permanently (a monotonic fact,
/// never re-checked or cleared) to avoid re-reading a long session's event log on every refresh.
static CONFIRMED_STARTED: Mutex<Option<HashSet<String>>> = Mutex::new(None);

fn copilot_session_has_started(session_id: &str) -> bool {
    let is_confirmed = |set: &Option<HashSet<String>>| set.as_ref().is_some_and(|s| s.contains(session_id));
    if is_confirmed(&CONFIRMED_STARTED.lock().unwrap_or_else(|e| e.into_inner())) {
        return true;
    }
    // No events file yet (or unreadable, or an unsafe id): nothing has happened in this session yet.
    let Ok(path) = copilot_session_events_log_path(session_id) else {
        return false;
    };
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    let has_user_message = String::from_utf8_lossy(&bytes).split('\n').any(|line| {
        // Tolerates a partial trailing line mid-write.
        serde_json::from_str::<serde_json::Value>(line.trim())
            .is_ok_and(|v| v.get("type").and_then(|t| t.as_str()) == Some("user.message"))
    });
    if has_user_message {
        CONFIRMED_STARTED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_or_insert_with(HashSet::new)
            .insert(session_id.to_string());
    }
    has_user_message
}

/// A session's turns in order: for a read-only "transcript", like `claude_storage` does for Claude.
pub fn list_copilot_turns(session_id: &str) -> Vec<CopilotTurn> {
    if !copilot_store_exists() {
        return Vec::new();
    }
    with_db(|db| {
        let mut stmt = db.prepare(
            "SELECT turn_index, user_message, assistant_response, timestamp \
             FROM turns WHERE session_id = ?1 ORDER BY turn_index",
        )?;
        let rows = stmt.query_map([session_id], |r| {
            Ok(CopilotTurn {
                turn_index: r.get(0)?,
                user_message: text(r, 1)?,
                assistant_response: text(r, 2)?,
                timestamp: text(r, 3)?.unwrap_or_default(),
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
    })
    .unwrap_or_default()
}

/// Every turn's user/assistant text concatenated, for search: the Copilot analogue of
/// `claude_storage::read_session_search_text`.
pub fn copilot_session_search_text(session_id: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    for turn in list_copilot_turns(session_id) {
        parts.extend(turn.user_message.filter(|m| !m.is_empty()));
        parts.extend(turn.assistant_response.filter(|m| !m.is_empty()));
    }
    parts.join("\n")
}

/// For "Copy Last Response": the most recent turn with a non-empty assistant reply, last-to-first
/// so a trailing empty/pending turn doesn't shadow the real last answer.
pub fn last_copilot_assistant_response(session_id: &str) -> Option<String> {
    list_copilot_turns(session_id)
        .into_iter()
        .rev()
        .find_map(|t| t.assistant_response.filter(|r| !r.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_env::EnvGuard;

    fn fixture() -> (EnvGuard, tempfile::TempDir) {
        let g = EnvGuard::new();
        *CONFIRMED_STARTED.lock().unwrap_or_else(|e| e.into_inner()) = None;
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("SDECK_USER_HOME", home.path());
        fs::create_dir_all(paths::copilot_dir()).unwrap();
        let db = Connection::open(session_store_path()).unwrap();
        db.execute_batch(
            "CREATE TABLE sessions (id TEXT PRIMARY KEY, cwd TEXT, repository TEXT, branch TEXT, \
                summary TEXT, created_at TEXT, updated_at TEXT);
             CREATE TABLE turns (session_id TEXT, turn_index INTEGER, user_message TEXT, \
                assistant_response TEXT, timestamp TEXT);
             INSERT INTO sessions VALUES ('old', 'C:\\a', 'org/a', 'main', 'Old one', '2026-01-01', '2026-01-02');
             INSERT INTO sessions VALUES ('new', 'C:\\b', NULL, NULL, NULL, '2026-02-01', '2026-02-02');
             INSERT INTO sessions VALUES ('blank', 'C:\\c', NULL, NULL, NULL, '2026-03-01', '2026-03-02');
             INSERT INTO turns VALUES ('new', 2, 'second q', '', 't2');
             INSERT INTO turns VALUES ('new', 1, 'first q', 'first a', 't1');",
        )
        .unwrap();
        (g, home)
    }

    fn write_events(id: &str, body: &str) {
        let path = copilot_session_events_log_path(id).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    #[test]
    fn no_store_means_no_sessions() {
        let _g = EnvGuard::new();
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("SDECK_USER_HOME", home.path());
        assert!(!copilot_store_exists());
        assert!(list_copilot_sessions().is_empty());
        assert!(list_copilot_turns("x").is_empty());
    }

    #[test]
    fn lists_started_sessions_newest_first() {
        let (_g, _home) = fixture();
        write_events(
            "old",
            "{\"type\":\"session.start\"}\n{\"type\":\"user.message\"}\n",
        );
        write_events("new", "{\"type\":\"user.message\"}\n{\"type\":\"assistant.turn_s");
        write_events("blank", "{\"type\":\"session.start\"}\n");
        let rows = list_copilot_sessions();
        let ids: Vec<_> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["new", "old"]);
        assert_eq!(rows[1].repository.as_deref(), Some("org/a"));
        assert_eq!(rows[1].summary.as_deref(), Some("Old one"));
        assert_eq!(rows[0].repository, None);
        assert_eq!(rows[1].created_at, "2026-01-01");
    }

    #[test]
    fn session_without_an_events_file_is_not_started() {
        let (_g, _home) = fixture();
        assert!(list_copilot_sessions().is_empty());
    }

    #[test]
    fn turns_are_ordered_and_feed_search_text_and_last_response() {
        let (_g, _home) = fixture();
        let turns = list_copilot_turns("new");
        assert_eq!(turns.iter().map(|t| t.turn_index).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(copilot_session_search_text("new"), "first q\nfirst a\nsecond q");
        assert_eq!(last_copilot_assistant_response("new").as_deref(), Some("first a"));
        assert_eq!(last_copilot_assistant_response("old"), None);
    }

    #[test]
    fn unsafe_ids_have_no_events_path() {
        assert!(copilot_session_events_log_path("../x").is_err());
        assert!(!copilot_session_has_started("../x"));
    }

    #[test]
    fn the_database_is_never_written() {
        let (_g, _home) = fixture();
        let before = fs::read(session_store_path()).unwrap();
        list_copilot_sessions();
        list_copilot_turns("new");
        assert_eq!(fs::read(session_store_path()).unwrap(), before);
    }
}
