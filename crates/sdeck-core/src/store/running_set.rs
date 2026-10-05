//! `~/.session-deck/running.json`: which sessions were live in the sdeck that wrote it, so the next
//! start can resume them. Kept apart from `state.json` so writes don't trigger `StoreChanged`.

use std::io;
use std::path::PathBuf;

use serde_json::{json, Value};

use super::atomic::write_atomic;
use crate::paths::deck_home;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningEntry {
    pub id: String,
    pub agent: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunningSet {
    /// The sdeck that wrote the file.
    pub pid: u32,
    pub sessions: Vec<RunningEntry>,
}

pub fn running_set_path() -> PathBuf {
    deck_home().join("running.json")
}

/// A missing or unreadable file is an empty set.
pub fn read_running_set() -> RunningSet {
    parse(&std::fs::read_to_string(running_set_path()).unwrap_or_default())
}

pub fn write_running_set(set: &RunningSet) -> io::Result<()> {
    let sessions: Vec<Value> = set
        .sessions
        .iter()
        .map(|e| json!({"id": e.id, "agent": e.agent}))
        .collect();
    let body = json!({"pid": set.pid, "sessions": sessions});
    write_atomic(
        &running_set_path(),
        &serde_json::to_string_pretty(&body).unwrap_or_default(),
    )
}

fn parse(raw: &str) -> RunningSet {
    let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(raw) else {
        return RunningSet::default();
    };
    let pid = obj
        .get("pid")
        .and_then(Value::as_u64)
        .and_then(|p| u32::try_from(p).ok())
        .unwrap_or(0);
    let sessions = obj
        .get("sessions")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|v| {
                    Some(RunningEntry {
                        id: v.get("id")?.as_str()?.to_string(),
                        agent: v.get("agent")?.as_str()?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    RunningSet { pid, sessions }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_env::EnvGuard;

    #[test]
    fn round_trips_and_a_corrupt_file_is_empty() {
        let _guard = EnvGuard::new();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("SESSION_DECK_HOME", dir.path());
        assert_eq!(read_running_set(), RunningSet::default());

        let set = RunningSet {
            pid: 42,
            sessions: vec![
                RunningEntry {
                    id: "a".into(),
                    agent: "claude".into(),
                },
                RunningEntry {
                    id: "b".into(),
                    agent: "copilot".into(),
                },
            ],
        };
        write_running_set(&set).unwrap();
        assert_eq!(read_running_set(), set);

        std::fs::write(running_set_path(), "{ nope").unwrap();
        assert_eq!(read_running_set(), RunningSet::default());
        std::fs::write(
            running_set_path(),
            r#"{"pid":7,"sessions":[{"id":"a"},{"id":"b","agent":"claude"}]}"#,
        )
        .unwrap();
        assert_eq!(read_running_set().sessions.len(), 1);
    }
}
