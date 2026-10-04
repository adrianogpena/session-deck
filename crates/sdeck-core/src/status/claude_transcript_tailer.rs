use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::Value;

use super::copilot_status_watcher::split_lines;
use super::watch::{watch_path, PathWatch};
use crate::discovery::claude_storage::{
    extract_assistant_display_text, extract_text, is_displayable_user_prompt,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnRole {
    User,
    Assistant,
}

/// One displayable turn pulled from a Claude transcript line, for the live preview pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TailedTurn {
    pub role: TurnRole,
    pub text: String,
}

/// Parses one `.jsonl` transcript line into a displayable turn, or `None` for anything else (tool
/// calls, hidden slash-command echoes, unparseable/partial lines).
pub fn parse_transcript_line(line: &str) -> Option<TailedTurn> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let record: Value = serde_json::from_str(trimmed).ok()?;
    let message = record.get("message")?;
    let role = message.get("role").and_then(Value::as_str);
    let content = message.get("content").unwrap_or(&Value::Null);
    match (record.get("type").and_then(Value::as_str), role) {
        (Some("user"), Some("user")) => {
            let text = extract_text(content).trim().to_string();
            (!text.is_empty() && is_displayable_user_prompt(&text)).then_some(TailedTurn {
                role: TurnRole::User,
                text,
            })
        }
        (Some("assistant"), Some("assistant")) => {
            let text = extract_assistant_display_text(content).trim().to_string();
            (!text.is_empty()).then_some(TailedTurn {
                role: TurnRole::Assistant,
                text,
            })
        }
        _ => None,
    }
}

/// How far back to backfill from the end of the file on attach: enough recent turns without parsing
/// a multi-MB transcript from the start.
const BACKFILL_WINDOW_BYTES: u64 = 64 * 1024;
/// Bounds the in-memory preview buffer so a long-running elsewhere session can't grow it without limit.
const MAX_TAILED_TURNS: usize = 50;

type OnUpdate = Arc<dyn Fn(Vec<TailedTurn>) + Send + Sync>;
type Log = Arc<dyn Fn(&str) + Send + Sync>;

#[derive(Default)]
struct TailBuffer {
    offset: u64,
    carry: Vec<u8>,
    turns: VecDeque<TailedTurn>,
    /// Set on detach, so a debounced callback already in flight doesn't emit for a stale file.
    detached: bool,
}

impl TailBuffer {
    fn ingest(&mut self, line: &str) -> bool {
        let Some(turn) = parse_transcript_line(line) else {
            return false;
        };
        self.turns.push_back(turn);
        if self.turns.len() > MAX_TAILED_TURNS {
            self.turns.pop_front();
        }
        true
    }

    fn snapshot(&self) -> Vec<TailedTurn> {
        self.turns.iter().cloned().collect()
    }
}

struct Attached {
    path: PathBuf,
    buffer: Arc<Mutex<TailBuffer>>,
    _watch: Option<PathWatch>,
}

/// Live, read-only preview of one Claude transcript file being written by a process Session Deck
/// didn't start (another terminal, VS Code's own terminal). Backfills the last
/// [`BACKFILL_WINDOW_BYTES`] immediately on attach, then tails bytes appended after that.
///
/// Single-slot: only the currently-previewed session ever needs this, so attaching to a new file
/// detaches the previous one. `on_update` runs on the watcher thread for appended turns.
pub struct ClaudeTranscriptTailer {
    current: Option<Attached>,
    log: Log,
}

impl ClaudeTranscriptTailer {
    pub fn new(log: impl Fn(&str) + Send + Sync + 'static) -> Self {
        ClaudeTranscriptTailer {
            current: None,
            log: Arc::new(log),
        }
    }

    pub fn attach(&mut self, path: &Path, on_update: impl Fn(Vec<TailedTurn>) + Send + Sync + 'static) {
        if self.current.as_ref().is_some_and(|c| c.path == path) {
            return;
        }
        self.detach();
        let on_update: OnUpdate = Arc::new(on_update);
        let buffer = Arc::new(Mutex::new(TailBuffer::default()));
        backfill(path, &buffer, &on_update, self.log.as_ref());
        let (poll_path, poll_buffer, poll_log) =
            (path.to_path_buf(), Arc::clone(&buffer), Arc::clone(&self.log));
        let watch = match watch_path(path, move || {
            poll(&poll_path, &poll_buffer, &on_update, poll_log.as_ref())
        }) {
            Ok(w) => Some(w),
            Err(e) => {
                (self.log)(&format!("[live-preview] Failed to watch {}: {e}", path.display()));
                None
            }
        };
        self.current = Some(Attached {
            path: path.to_path_buf(),
            buffer,
            _watch: watch,
        });
    }

    pub fn detach(&mut self) {
        if let Some(attached) = self.current.take() {
            attached.buffer.lock().unwrap_or_else(|e| e.into_inner()).detached = true;
        }
    }
}

impl Drop for ClaudeTranscriptTailer {
    fn drop(&mut self) {
        self.detach();
    }
}

fn read_range(path: &Path, start: u64, length: u64) -> std::io::Result<Vec<u8>> {
    let mut buf = vec![0u8; length as usize];
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(start))?;
    file.read_exact(&mut buf)?;
    Ok(buf)
}

fn backfill(
    path: &Path,
    buffer: &Mutex<TailBuffer>,
    on_update: &OnUpdate,
    log: &(dyn Fn(&str) + Send + Sync),
) {
    let Ok(size) = fs::metadata(path).map(|m| m.len()) else {
        return;
    };
    let mut buf = buffer.lock().unwrap_or_else(|e| e.into_inner());
    let length = BACKFILL_WINDOW_BYTES.min(size);
    if length > 0 {
        match read_range(path, size - length, length) {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                // The window's first line is usually cut mid-record when it doesn't cover the whole
                // file: skip it rather than fail to parse it.
                let skip = usize::from(length < size);
                for line in text.split('\n').skip(skip) {
                    buf.ingest(line);
                }
            }
            Err(e) => log(&format!("[live-preview] Failed to read {}: {e}", path.display())),
        }
    }
    buf.offset = size;
    on_update(buf.snapshot());
}

fn poll(path: &Path, buffer: &Mutex<TailBuffer>, on_update: &OnUpdate, log: &(dyn Fn(&str) + Send + Sync)) {
    let mut buf = buffer.lock().unwrap_or_else(|e| e.into_inner());
    if buf.detached {
        return;
    }
    let Ok(size) = fs::metadata(path).map(|m| m.len()) else {
        return;
    };
    if size < buf.offset {
        buf.offset = size; // truncated/replaced from under us
        return;
    }
    if size == buf.offset {
        return;
    }
    let chunk = match read_range(path, buf.offset, size - buf.offset) {
        Ok(c) => c,
        Err(e) => {
            log(&format!("[live-preview] Failed to read {}: {e}", path.display()));
            return;
        }
    };
    buf.offset = size;
    let (lines, carry) = split_lines(&buf.carry, &chunk);
    buf.carry = carry;
    let mut changed = false;
    for line in lines {
        changed = buf.ingest(&String::from_utf8_lossy(&line)) || changed;
    }
    if changed {
        on_update(buf.snapshot());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;
    use std::sync::mpsc;
    use std::time::Duration;

    fn user_line(text: &str) -> String {
        json!({"type": "user", "message": {"role": "user", "content": text}}).to_string()
    }

    fn assistant_line(blocks: Value) -> String {
        json!({"type": "assistant", "message": {"role": "assistant", "content": blocks}}).to_string()
    }

    fn turn(role: TurnRole, text: &str) -> TailedTurn {
        TailedTurn {
            role,
            text: text.to_string(),
        }
    }

    #[test]
    fn reads_a_plain_user_prompt() {
        assert_eq!(
            parse_transcript_line(&user_line("fix the bug")),
            Some(turn(TurnRole::User, "fix the bug"))
        );
    }

    #[test]
    fn filters_out_a_hidden_slash_command_echo() {
        assert_eq!(
            parse_transcript_line(&user_line("<command-name>/clear</command-name>")),
            None
        );
    }

    #[test]
    fn reads_assistant_display_text_excluding_thinking_blocks() {
        let line = assistant_line(json!([
            {"type": "thinking", "text": "internal reasoning"},
            {"type": "text", "text": "Here is the fix."}
        ]));
        assert_eq!(
            parse_transcript_line(&line),
            Some(turn(TurnRole::Assistant, "Here is the fix."))
        );
    }

    #[test]
    fn ignores_an_assistant_turn_with_only_tool_use_or_thinking_blocks() {
        let line = assistant_line(json!([
            {"type": "tool_use"},
            {"type": "thinking", "text": "internal reasoning"}
        ]));
        assert_eq!(parse_transcript_line(&line), None);
    }

    #[test]
    fn ignores_non_user_or_assistant_record_types() {
        let line = json!({"type": "custom-title", "customTitle": "Renamed"}).to_string();
        assert_eq!(parse_transcript_line(&line), None);
    }

    #[test]
    fn tolerates_a_blank_or_malformed_line() {
        assert_eq!(parse_transcript_line(""), None);
        assert_eq!(parse_transcript_line("   "), None);
        assert_eq!(parse_transcript_line("{not json"), None);
    }

    fn collector() -> (OnUpdate, mpsc::Receiver<Vec<TailedTurn>>) {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let on_update: OnUpdate = Arc::new(move |turns| {
            let _ = tx.lock().unwrap().send(turns);
        });
        (on_update, rx)
    }

    #[test]
    fn backfill_then_poll_appends_new_turns() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("s.jsonl");
        fs::write(&path, format!("{}\n", user_line("first"))).unwrap();
        let buffer = Mutex::new(TailBuffer::default());
        let (on_update, rx) = collector();
        let log = |_: &str| {};

        backfill(&path, &buffer, &on_update, &log);
        assert_eq!(rx.try_recv().unwrap(), [turn(TurnRole::User, "first")]);

        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        let reply = assistant_line(json!([{"type": "text", "text": "done"}]));
        let (head, tail) = reply.split_at(10);
        write!(file, "{head}").unwrap();
        poll(&path, &buffer, &on_update, &log);
        assert!(rx.try_recv().is_err(), "a partial line emits nothing");

        writeln!(file, "{tail}").unwrap();
        poll(&path, &buffer, &on_update, &log);
        assert_eq!(
            rx.try_recv().unwrap(),
            [turn(TurnRole::User, "first"), turn(TurnRole::Assistant, "done")]
        );

        writeln!(file, "{{\"type\":\"progress\"}}").unwrap();
        poll(&path, &buffer, &on_update, &log);
        assert!(rx.try_recv().is_err(), "non-displayable lines emit nothing");
    }

    #[test]
    fn backfill_skips_the_cut_first_line_of_a_partial_window() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("s.jsonl");
        let filler = user_line(&"x".repeat(BACKFILL_WINDOW_BYTES as usize));
        fs::write(&path, format!("{filler}\n{}\n", user_line("recent"))).unwrap();
        let buffer = Mutex::new(TailBuffer::default());
        let (on_update, rx) = collector();
        backfill(&path, &buffer, &on_update, &|_: &str| {});
        assert_eq!(rx.try_recv().unwrap(), [turn(TurnRole::User, "recent")]);
    }

    #[test]
    fn keeps_only_the_most_recent_turns() {
        let mut buf = TailBuffer::default();
        for i in 0..MAX_TAILED_TURNS + 5 {
            buf.ingest(&user_line(&format!("p{i}")));
        }
        let turns = buf.snapshot();
        assert_eq!(turns.len(), MAX_TAILED_TURNS);
        assert_eq!(turns[0].text, "p5");
    }

    #[test]
    fn poll_resyncs_after_truncation_and_ignores_after_detach() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("s.jsonl");
        fs::write(&path, "x\n").unwrap();
        let buffer = Mutex::new(TailBuffer {
            offset: 100,
            ..TailBuffer::default()
        });
        let (on_update, rx) = collector();
        poll(&path, &buffer, &on_update, &|_: &str| {});
        assert_eq!(buffer.lock().unwrap().offset, 2);

        buffer.lock().unwrap().detached = true;
        fs::write(&path, format!("x\n{}\n", user_line("late"))).unwrap();
        poll(&path, &buffer, &on_update, &|_: &str| {});
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn attached_tailer_emits_appended_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("s.jsonl");
        fs::write(&path, "").unwrap();
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let mut tailer = ClaudeTranscriptTailer::new(|_| {});
        tailer.attach(&path, move |turns| {
            let _ = tx.lock().unwrap().send(turns);
        });
        assert!(rx.recv_timeout(Duration::from_secs(1)).unwrap().is_empty());

        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "{}", user_line("hello")).unwrap();
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            [turn(TurnRole::User, "hello")]
        );
        tailer.detach();
        assert!(tailer.current.is_none());
    }
}
