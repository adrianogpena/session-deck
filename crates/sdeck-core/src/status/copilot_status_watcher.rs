use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::session_status::{clear_session_status, write_session_status, SessionStatus};
use super::watch::{watch_path, PathWatch};
use crate::commands::session_id::is_safe_session_id;
use crate::discovery::copilot_storage::copilot_session_events_log_path;

/// Copilot CLI's `events.jsonl` event types mapped to Session Deck's coarse status. Unlisted types
/// are ignored (status stays as-is). Note: the built-in `ask_user` clarifying-question tool does NOT
/// fire `*.requested`/`*.completed` despite the schema: it goes through the generic
/// `tool.execution_start`/`_complete` pair like any other tool, distinguished by `data.toolName`.
const ASK_USER_TOOL_NAME: &str = "ask_user";
const RUNNING_EVENTS: [&str; 4] = [
    "assistant.turn_start",
    "permission.completed",
    "elicitation.completed",
    "user_input.completed",
];
const WAITING_EVENTS: [&str; 3] = [
    "permission.requested",
    "elicitation.requested",
    "user_input.requested",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopilotStatusAction {
    Status(SessionStatus),
    Clear,
}

/// Pure decision logic: see [`CopilotStatusWatcher`] for how it's applied. `tool_name` is the
/// event's `data.toolName`, when it has one.
pub fn classify_copilot_event(event_type: &str, tool_name: Option<&str>) -> Option<CopilotStatusAction> {
    use CopilotStatusAction::{Clear, Status};
    let ask_user = tool_name == Some(ASK_USER_TOOL_NAME);
    match event_type {
        "session.shutdown" => Some(Clear),
        "session.error" => Some(Status(SessionStatus::Error)),
        "tool.execution_start" if ask_user => Some(Status(SessionStatus::Waiting)),
        "tool.execution_complete" if ask_user => Some(Status(SessionStatus::Running)),
        t if WAITING_EVENTS.contains(&t) => Some(Status(SessionStatus::Waiting)),
        "assistant.turn_end" => Some(Status(SessionStatus::Done)),
        t if RUNNING_EVENTS.contains(&t) => Some(Status(SessionStatus::Running)),
        _ => None,
    }
}

/// Splits a growing NDJSON chunk into complete lines plus a trailing partial line to carry into the
/// next read. Works on bytes so a multi-byte character cut by a read boundary survives.
pub fn split_lines(carry: &[u8], chunk: &[u8]) -> (Vec<Vec<u8>>, Vec<u8>) {
    let mut combined = carry.to_vec();
    combined.extend_from_slice(chunk);
    let mut parts: Vec<Vec<u8>> = combined.split(|b| *b == b'\n').map(<[u8]>::to_vec).collect();
    let new_carry = parts.pop().unwrap_or_default();
    (parts, new_carry)
}

/// How long to wait for a resumed session's `events.jsonl` to appear before giving up.
const FILE_WAIT_TIMEOUT: Duration = Duration::from_secs(15);
const FILE_WAIT_POLL_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Default)]
struct TailState {
    offset: u64,
    carry: Vec<u8>,
}

struct Tail {
    stop: Arc<AtomicBool>,
    watch: Arc<Mutex<Option<PathWatch>>>,
}

type Tails = Arc<Mutex<HashMap<String, Tail>>>;

/// Live status for Copilot CLI sessions, tailing `events.jsonl` directly (zero setup, no hooks
/// config). `start()` tails only bytes appended after tracking begins: it never backfills history.
/// Writes through the same status file `claude_process_watcher` uses for Claude, so every
/// downstream consumer works unchanged.
pub struct CopilotStatusWatcher {
    tails: Tails,
    log: Arc<dyn Fn(&str) + Send + Sync>,
}

impl CopilotStatusWatcher {
    pub fn new(log: impl Fn(&str) + Send + Sync + 'static) -> Self {
        CopilotStatusWatcher {
            tails: Arc::default(),
            log: Arc::new(log),
        }
    }

    pub fn start(&self, session_id: &str) {
        if !is_safe_session_id(session_id) {
            return;
        }
        let Ok(path) = copilot_session_events_log_path(session_id) else {
            return;
        };
        let stop = Arc::new(AtomicBool::new(false));
        let watch: Arc<Mutex<Option<PathWatch>>> = Arc::default();
        {
            let mut tails = self.tails.lock().unwrap_or_else(|e| e.into_inner());
            if tails.contains_key(session_id) {
                return;
            }
            tails.insert(
                session_id.to_string(),
                Tail {
                    stop: Arc::clone(&stop),
                    watch: Arc::clone(&watch),
                },
            );
        }
        let ctx = (
            session_id.to_string(),
            Arc::clone(&self.tails),
            Arc::clone(&self.log),
        );
        thread::spawn(move || wait_then_attach(ctx, path, stop, watch));
    }

    pub fn stop(&self, session_id: &str) {
        let tail = self
            .tails
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(session_id);
        if let Some(tail) = tail {
            tail.stop.store(true, Ordering::SeqCst);
            // Dropping the watch ends the notify thread.
            tail.watch.lock().unwrap_or_else(|e| e.into_inner()).take();
        }
    }

    pub fn dispose(&self) {
        let ids: Vec<String> = self
            .tails
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect();
        for id in ids {
            self.stop(&id);
        }
    }
}

impl Drop for CopilotStatusWatcher {
    fn drop(&mut self) {
        self.dispose();
    }
}

type WaitContext = (String, Tails, Arc<dyn Fn(&str) + Send + Sync>);

fn wait_then_attach(
    (session_id, tails, log): WaitContext,
    path: PathBuf,
    stop: Arc<AtomicBool>,
    watch_slot: Arc<Mutex<Option<PathWatch>>>,
) {
    let started = Instant::now();
    while !path.exists() {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        if started.elapsed() >= FILE_WAIT_TIMEOUT {
            log(&format!(
                "[copilot-status] Gave up waiting for {} to appear.",
                path.display()
            ));
            tails
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&session_id);
            return;
        }
        thread::sleep(FILE_WAIT_POLL_INTERVAL);
    }
    // Starts at EOF: only events appended from this point on are "live".
    let state = Arc::new(Mutex::new(TailState {
        offset: fs::metadata(&path).map_or(0, |m| m.len()),
        carry: Vec::new(),
    }));
    let (poll_path, poll_id, poll_log) = (path.clone(), session_id, Arc::clone(&log));
    let watch = watch_path(&path, move || {
        poll(&poll_id, &poll_path, &state, poll_log.as_ref())
    });
    match watch {
        Ok(w) => {
            let mut slot = watch_slot.lock().unwrap_or_else(|e| e.into_inner());
            if !stop.load(Ordering::SeqCst) {
                *slot = Some(w);
            }
        }
        Err(e) => log(&format!(
            "[copilot-status] Failed to watch {}: {e}",
            path.display()
        )),
    }
}

/// Reads whatever was appended since the last poll and applies each complete line.
fn poll(session_id: &str, path: &Path, state: &Mutex<TailState>, log: &(dyn Fn(&str) + Send + Sync)) {
    let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(size) = fs::metadata(path).map(|m| m.len()) else {
        return;
    };
    // File was truncated/replaced from under us: resync to the new end instead of a stale offset.
    if size < state.offset {
        state.offset = size;
        return;
    }
    if size == state.offset {
        return;
    }
    let mut chunk = vec![0u8; (size - state.offset) as usize];
    let read = File::open(path).and_then(|mut f| {
        f.seek(SeekFrom::Start(state.offset))?;
        f.read_exact(&mut chunk)
    });
    if let Err(e) = read {
        log(&format!(
            "[copilot-status] Failed to read {}: {e}",
            path.display()
        ));
        return;
    }
    state.offset = size;
    let (lines, carry) = split_lines(&state.carry, &chunk);
    state.carry = carry;
    for line in lines {
        handle_line(session_id, &String::from_utf8_lossy(&line));
    }
}

fn handle_line(session_id: &str, line: &str) {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return;
    }
    let Ok(parsed) = serde_json::from_str::<Value>(trimmed) else {
        return; // Tolerate a malformed/partial line.
    };
    let Some(event_type) = parsed.get("type").and_then(Value::as_str) else {
        return;
    };
    let tool_name = parsed
        .get("data")
        .and_then(|d| d.get("toolName"))
        .and_then(Value::as_str);
    match classify_copilot_event(event_type, tool_name) {
        Some(CopilotStatusAction::Clear) => clear_session_status(session_id),
        Some(CopilotStatusAction::Status(status)) => {
            let _ = write_session_status(session_id, status);
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_env::EnvGuard;
    use crate::status::session_status::read_session_status;
    use CopilotStatusAction::{Clear, Status};

    fn c(t: &str) -> Option<CopilotStatusAction> {
        classify_copilot_event(t, None)
    }

    #[test]
    fn turn_start_is_running() {
        assert_eq!(c("assistant.turn_start"), Some(Status(SessionStatus::Running)));
    }

    #[test]
    fn resolved_requests_are_running_too() {
        for t in [
            "permission.completed",
            "elicitation.completed",
            "user_input.completed",
        ] {
            assert_eq!(c(t), Some(Status(SessionStatus::Running)), "{t}");
        }
    }

    #[test]
    fn pending_requests_are_waiting() {
        for t in [
            "permission.requested",
            "elicitation.requested",
            "user_input.requested",
        ] {
            assert_eq!(c(t), Some(Status(SessionStatus::Waiting)), "{t}");
        }
    }

    #[test]
    fn turn_end_error_and_shutdown() {
        assert_eq!(c("assistant.turn_end"), Some(Status(SessionStatus::Done)));
        assert_eq!(c("session.error"), Some(Status(SessionStatus::Error)));
        assert_eq!(c("session.shutdown"), Some(Clear));
    }

    #[test]
    fn unknown_events_are_ignored() {
        assert_eq!(c("session.model_change"), None);
        assert_eq!(c("assistant.message_delta"), None);
    }

    #[test]
    fn ask_user_tool_start_waits_and_other_tools_are_left_alone() {
        assert_eq!(
            classify_copilot_event("tool.execution_start", Some("ask_user")),
            Some(Status(SessionStatus::Waiting))
        );
        assert_eq!(classify_copilot_event("tool.execution_start", Some("view")), None);
        assert_eq!(c("tool.execution_start"), None);
    }

    #[test]
    fn ask_user_tool_complete_runs_and_other_tools_are_left_alone() {
        assert_eq!(
            classify_copilot_event("tool.execution_complete", Some("ask_user")),
            Some(Status(SessionStatus::Running))
        );
        assert_eq!(
            classify_copilot_event("tool.execution_complete", Some("grep")),
            None
        );
    }

    #[test]
    fn split_lines_carries_a_trailing_partial_line() {
        let (lines, carry) = split_lines(b"", b"{\"a\":1}\n{\"a\":2}\n{\"a\":3");
        assert_eq!(lines, [b"{\"a\":1}".to_vec(), b"{\"a\":2}".to_vec()]);
        assert_eq!(carry, b"{\"a\":3");
    }

    #[test]
    fn split_lines_completes_a_carried_line_once_the_rest_arrives() {
        let (_, carry) = split_lines(b"", b"{\"a\":1}\n{\"partial\":");
        let (lines, carry) = split_lines(&carry, b"true}\n");
        assert_eq!(lines, [b"{\"partial\":true}".to_vec()]);
        assert!(carry.is_empty());
    }

    #[test]
    fn split_lines_with_no_newline_is_a_pure_carry() {
        let (lines, carry) = split_lines(b"abc", b"def");
        assert!(lines.is_empty());
        assert_eq!(carry, b"abcdef");
    }

    #[test]
    fn poll_applies_only_appended_complete_lines() {
        let _g = EnvGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("SESSION_DECK_STATUS_DIR", tmp.path().join("status"));
        let path = tmp.path().join("events.jsonl");
        fs::write(&path, "{\"type\":\"assistant.turn_end\"}\n").unwrap();
        let state = Mutex::new(TailState {
            offset: fs::metadata(&path).unwrap().len(),
            carry: Vec::new(),
        });
        let log = |_: &str| {};

        poll("sess-a", &path, &state, &log);
        assert_eq!(read_session_status("sess-a"), None, "history is never backfilled");

        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        std::io::Write::write_all(
            &mut file,
            b"{\"type\":\"assistant.turn_start\"}\n{\"type\":\"assistant.turn_e",
        )
        .unwrap();
        poll("sess-a", &path, &state, &log);
        assert_eq!(
            read_session_status("sess-a").map(|r| r.status),
            Some(SessionStatus::Running)
        );

        std::io::Write::write_all(&mut file, b"nd\"}\nnot json\n").unwrap();
        poll("sess-a", &path, &state, &log);
        assert_eq!(
            read_session_status("sess-a").map(|r| r.status),
            Some(SessionStatus::Done)
        );

        std::io::Write::write_all(&mut file, b"{\"type\":\"session.shutdown\"}\n").unwrap();
        poll("sess-a", &path, &state, &log);
        assert_eq!(read_session_status("sess-a"), None);
    }

    #[test]
    fn poll_resyncs_after_truncation() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("events.jsonl");
        fs::write(&path, "x\n").unwrap();
        let state = Mutex::new(TailState {
            offset: 100,
            carry: Vec::new(),
        });
        poll("sess-b", &path, &state, &|_: &str| {});
        assert_eq!(state.lock().unwrap().offset, 2);
    }

    #[test]
    fn watcher_picks_up_a_session_started_before_its_file_exists() {
        let _g = EnvGuard::new();
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("SDECK_USER_HOME", home.path());
        std::env::set_var("SESSION_DECK_STATUS_DIR", home.path().join("status"));
        let path = copilot_session_events_log_path("sess-c").unwrap();
        let watcher = CopilotStatusWatcher::new(|_| {});
        watcher.start("sess-c");
        watcher.start("sess-c");
        watcher.start("../bad");
        assert_eq!(watcher.tails.lock().unwrap().len(), 1);

        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "").unwrap();
        thread::sleep(Duration::from_millis(1500));
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        std::io::Write::write_all(&mut file, b"{\"type\":\"permission.requested\"}\n").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while read_session_status("sess-c").is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(
            read_session_status("sess-c").map(|r| r.status),
            Some(SessionStatus::Waiting)
        );
        watcher.stop("sess-c");
        assert!(watcher.tails.lock().unwrap().is_empty());
    }
}
