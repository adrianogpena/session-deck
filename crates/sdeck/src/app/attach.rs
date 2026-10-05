//! Full-screen attach: sdeck stops drawing and the agent owns the real terminal until the detach key (Ctrl+Q by default). Port
//! of `App.attach`, `teardownAttached`, `detach`, `switchToInteracting` and `switchToAttach`.

use std::time::Instant;

use super::App;
use crate::keys::{ENABLE_FOCUS_REPORTING, RESET_AGENT_MODES};

impl App {
    /// Enter on a session: starts a stopped one first; refused for one open elsewhere. Repaints the
    /// mirrored screen at once, then live output streams straight through (see `on_pty_output`).
    pub(super) fn attach(&mut self, uid: u64, now: Instant) {
        if !self.ensure_live(uid, now) {
            return;
        }
        self.attached = Some(uid);
        self.mark_seen(uid);
        // Off while input goes straight to the agent: a wheel notch would reach it as raw SGR bytes.
        self.mouse_capture_change = Some(false);
        let (cols, rows) = self.term_size;
        let Some(live) = self.session_mut(uid).and_then(|s| s.live.as_mut()) else {
            return;
        };
        let snapshot = live.screen().contents_formatted();
        // The agent redraws at full size; the snapshot shows meanwhile.
        live.resize(cols, rows);
        self.emit("\x1b[?2026h\x1b[0m\x1b[2J\x1b[H\x1b[?25h");
        self.pending_output.extend_from_slice(&snapshot);
        self.emit("\x1b[?2026l");
    }

    /// Leaves attached mode: resets what the agent may have switched on in the real terminal,
    /// restores sdeck's mouse and focus reporting, and forgets the title (the agent set its own).
    fn teardown_attached(&mut self) {
        self.attached = None;
        self.emit(RESET_AGENT_MODES);
        self.emit("\x1b[?25l");
        self.emit(ENABLE_FOCUS_REPORTING);
        if self.mouse_tracking {
            self.mouse_capture_change = Some(true);
        }
        self.last_title.clear();
        self.clear_screen = true;
        self.dirty = true;
    }

    /// The detach key while attached (or the agent exited, with `note`). Whatever finished while attached
    /// counts as seen.
    pub(super) fn detach(&mut self, note: Option<String>) {
        let seen = self.attached;
        self.teardown_attached();
        if let Some(uid) = seen {
            self.mark_seen(uid);
        }
        self.resize_all_to_pane();
        if let Some(note) = note {
            self.flash(note, Instant::now());
        }
    }

    /// Ctrl+K T while attached: back to typing into it from the list, without detaching first.
    /// Checked live first, so attach stays put if the session can't be reached.
    pub(super) fn switch_to_interacting(&mut self, uid: u64, now: Instant) {
        if !self.ensure_live(uid, now) {
            return;
        }
        self.teardown_attached();
        self.resize_all_to_pane();
        self.start_interacting(uid, now);
    }

    /// Ctrl+K T while interacting: attach full-screen to the same session.
    pub(super) fn switch_to_attach(&mut self, uid: u64, now: Instant) {
        if !self.ensure_live(uid, now) {
            return;
        }
        self.interacting = None;
        self.attach(uid, now);
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use crate::app::test_fixture::{fixture, Fixture};

    impl Fixture {
        /// The real-terminal output since the last call, lossily as text.
        fn take_output(&mut self) -> String {
            String::from_utf8_lossy(&std::mem::take(&mut self.app.pending_output)).into_owned()
        }
    }

    #[test]
    fn attach_repaints_forwards_typing_and_ctrl_q_returns_to_the_list() {
        let mut f = fixture();
        let uid = f.add(Some("abc"), true);
        f.app.set_size(100, 30);
        f.key("s");
        f.wait_for_screen(uid, "> ");
        f.app.pending_output.clear();

        f.key("\r");
        assert_eq!(f.app.attached, Some(uid));
        let repaint = f.take_output();
        assert!(repaint.starts_with("\x1b[?2026h"), "{repaint:?}");
        assert!(repaint.contains("ARGS=--resume abc"), "{repaint:?}");
        assert_eq!(f.app.mouse_capture_change, Some(false));
        let live = f.session(uid).unwrap().live.as_ref().unwrap();
        assert_eq!(live.size(), (100, 30), "full terminal size while attached");

        f.key("hello");
        f.key("\r");
        f.wait_for_screen(uid, "echo: hello");
        // The agent's output went to the real terminal too.
        assert!(f.take_output().contains("echo: hello"));

        f.key("\x11");
        assert_eq!(f.app.attached, None);
        assert!(f.app.clear_screen);
        assert!(f.take_output().contains(crate::keys::RESET_AGENT_MODES));
        let live = f.session(uid).unwrap().live.as_ref().unwrap();
        assert_ne!(live.size(), (100, 30), "back to the preview's size");
        f.key("j");
        assert_eq!(f.app.attached, None, "list keys work again");
    }

    #[test]
    fn attach_starts_a_stopped_session_and_its_exit_detaches() {
        let mut f = fixture();
        let uid = f.add(Some("abc"), true);
        f.key("\r");
        assert_eq!(f.app.attached, Some(uid));
        f.wait_for_screen(uid, "> ");
        f.key("/exit 2\r");
        let deadline = Instant::now() + Duration::from_secs(15);
        while f.app.attached.is_some() {
            let ev = f.rx.recv_timeout(deadline - Instant::now()).unwrap();
            f.app.handle(ev, Instant::now());
        }
        assert_eq!(f.app.message, "Session exited (code 2)");
    }

    #[test]
    fn keys_after_the_attaching_enter_go_to_the_agent() {
        let mut f = fixture();
        let uid = f.add(Some("abc"), true);
        f.key("s");
        f.wait_for_screen(uid, "> ");
        f.key("\rtyped\r");
        f.wait_for_screen(uid, "echo: typed");
    }

    #[test]
    fn ctrl_k_t_swaps_attached_and_interacting() {
        let mut f = fixture();
        let uid = f.add(Some("abc"), true);
        f.key("\r");
        f.key("\x0bt");
        assert_eq!((f.app.attached, f.app.interacting), (None, Some(uid)));
        f.key("\x0b");
        f.key("T");
        assert_eq!((f.app.attached, f.app.interacting), (Some(uid), None));
        f.key("\x0bq");
        assert_eq!(f.app.attached, None);
        assert_eq!(f.app.message, "Session stopped");
        assert!(!f.session(uid).unwrap().is_live());
    }
}
