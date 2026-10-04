//! `v`: the trajectory popup of the selected Claude session. Port of `openTrace` / `onTraceKey`
//! (the alerts popup, `onAlertsKey`, is a plain scroller and lives with the other overlays).

use std::cell::Cell;
use std::time::Instant;

use sdeck_core::discovery::claude_trace::{read_session_trace, TraceStep};

use super::overlays::Overlay;
use super::App;
use crate::event::AppEvent;

pub(super) struct TraceState {
    /// The session being traced, by `DeckSession::uid`.
    pub uid: u64,
    /// Empty until the transcript has been read.
    pub steps: Vec<TraceStep>,
    pub selected: usize,
    pub detail_scroll: Cell<usize>,
}

impl App {
    /// Opens the popup and reads the trace fresh from the transcript, off the main thread.
    pub(super) fn open_trace(&mut self, now: Instant) {
        let target = self
            .selected_session()
            .filter(|s| s.agent == "claude")
            .and_then(|s| Some((s.uid, s.file.clone()?)));
        let Some((uid, file)) = target else {
            self.flash("Trajectory is only available for Claude sessions.".into(), now);
            return;
        };
        self.overlay = Some(Overlay::Trace(TraceState {
            uid,
            steps: Vec::new(),
            selected: 0,
            detail_scroll: Cell::new(0),
        }));
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let steps = read_session_trace(&file).unwrap_or_default();
            let _ = tx.send(AppEvent::Trace(uid, steps));
        });
    }

    pub(super) fn on_trace_steps(&mut self, uid: u64, steps: Vec<TraceStep>) {
        let Some(Overlay::Trace(trace)) = &mut self.overlay else {
            return;
        };
        if trace.uid != uid {
            return;
        }
        trace.selected = trace.selected.min(steps.len().saturating_sub(1));
        trace.steps = steps;
        self.dirty = true;
    }

    pub(super) fn on_trace_key(&mut self, key: &str) {
        let Some(Overlay::Trace(trace)) = &mut self.overlay else {
            return;
        };
        let last = trace.steps.len().saturating_sub(1);
        match key {
            "\x1b" | "v" | "q" => self.overlay = None,
            "\x1b[A" | "\x1bOA" | "k" => {
                trace.selected = trace.selected.saturating_sub(1);
                trace.detail_scroll.set(0);
            }
            "\x1b[B" | "\x1bOB" | "j" => {
                trace.selected = (trace.selected + 1).min(last);
                trace.detail_scroll.set(0);
            }
            "\x1b[5~" => trace.detail_scroll.set(trace.detail_scroll.get() + 5),
            "\x1b[6~" => trace
                .detail_scroll
                .set(trace.detail_scroll.get().saturating_sub(5)),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{Duration, Instant};

    use serde_json::json;

    use crate::app::test_fixture::fixture;
    use crate::app::Overlay;

    fn pump(f: &mut crate::app::test_fixture::Fixture) {
        loop {
            let ev = f.rx.recv_timeout(Duration::from_secs(10)).expect("trace arrives");
            let is_trace = matches!(ev, crate::event::AppEvent::Trace(..));
            f.app.handle(ev, Instant::now());
            if is_trace {
                return;
            }
        }
    }

    fn trace(f: &crate::app::test_fixture::Fixture) -> (usize, usize, usize) {
        match &f.app.overlay {
            Some(Overlay::Trace(t)) => (t.steps.len(), t.selected, t.detail_scroll.get()),
            _ => panic!("trace is open"),
        }
    }

    #[test]
    fn opens_with_the_transcripts_steps_and_navigates() {
        let mut f = fixture();
        let uid = f.add(Some("aaa"), true);
        let file = f.home.path().join("aaa.jsonl");
        let user = json!({"type": "user", "message": {"role": "user", "content": "fix it"}});
        let reply = json!({
            "type": "assistant",
            "message": {"role": "assistant", "content": [
                {"type": "text", "text": "on it"},
                {"type": "tool_use", "id": "t1", "name": "Read", "input": {"file_path": "a.rs"}}
            ]}
        });
        let result = json!({
            "type": "user",
            "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t1", "content": "ok"}]}
        });
        fs::write(&file, format!("{user}\n{reply}\n{result}\n")).unwrap();
        f.app.session_mut(uid).unwrap().file = Some(file);

        f.key("v");
        assert_eq!(trace(&f).0, 0);
        pump(&mut f);
        assert_eq!(trace(&f).0, 3);

        f.key("jj");
        assert_eq!(trace(&f).1, 2);
        f.key("j");
        assert_eq!(trace(&f).1, 2, "stops at the last step");
        f.key("\x1b[5~");
        assert_eq!(trace(&f).2, 5);
        f.key("\x1b[6~\x1b[6~");
        assert_eq!(trace(&f).2, 0);
        f.key("k");
        assert_eq!(trace(&f).1, 1);
        f.key("v");
        assert!(f.app.overlay.is_none());
    }

    #[test]
    fn only_claude_sessions_with_a_transcript_have_one() {
        let mut f = fixture();
        f.add(Some("aaa"), true);
        f.key("v");
        assert!(f.app.overlay.is_none());
        assert_eq!(f.app.message, "Trajectory is only available for Claude sessions.");
    }
}
