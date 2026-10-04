//! `q` / Ctrl+C: quits at once, or asks first while a session is running or waiting. Port of
//! `App.quit`, `onQuitConfirmKey` and `quitNow`.

use super::App;
use crate::sessions::SessionStatus;

/// The "sessions still running will be stopped" popup; the default button is No.
pub(super) struct QuitConfirm {
    pub active_count: usize,
    pub yes: bool,
}

impl App {
    pub(super) fn quit(&mut self) {
        let active_count = self
            .sessions
            .iter()
            .filter(|s| {
                s.is_live()
                    && matches!(
                        self.procs.status_of(s),
                        SessionStatus::Running | SessionStatus::Waiting
                    )
            })
            .count();
        if active_count == 0 {
            self.quit_now();
        } else {
            self.quit_confirm = Some(QuitConfirm {
                active_count,
                yes: false,
            });
            self.dirty = true;
        }
    }

    pub(super) fn on_quit_confirm_key(&mut self, key: &str) {
        let Some(confirm) = self.quit_confirm.as_mut() else {
            return;
        };
        match key {
            "\x1b[C" | "\x1b[D" | "\t" => confirm.yes = !confirm.yes,
            "\r" if confirm.yes => self.quit_now(),
            "y" | "Y" => self.quit_now(),
            "\r" | "n" | "N" | "\x1b" | "q" | "\x03" => self.quit_confirm = None,
            _ => {}
        }
        self.dirty = true;
    }

    /// Stops every live session and ends the loop.
    fn quit_now(&mut self) {
        self.quit_confirm = None;
        self.copilot_watcher.dispose();
        for s in &mut self.sessions {
            if let Some(live) = s.live.as_mut() {
                live.dispose();
            }
        }
        self.quit = true;
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_fixture::fixture;
    use crate::sessions::SessionStatus;

    #[test]
    fn quits_at_once_when_nothing_is_running() {
        let mut f = fixture();
        f.add(Some("a"), true);
        f.key("q");
        assert!(f.app.should_quit());
    }

    #[test]
    fn running_session_asks_and_the_default_keeps_it_running() {
        let mut f = fixture();
        let uid = f.add(Some("a"), true);
        f.key("s");
        f.wait_for_screen(uid, "> ");
        f.set_process_status(uid, "busy");
        assert_eq!(
            f.app.procs.status_of(f.session(uid).unwrap()),
            SessionStatus::Running
        );
        f.key("q");
        assert!(!f.app.should_quit());
        assert!(f.app.quit_confirm.is_some());
        f.key("\r");
        assert!(f.app.quit_confirm.is_none());
        assert!(!f.app.should_quit());
        assert!(f.session(uid).unwrap().is_live());
    }

    #[test]
    fn yes_stops_every_live_session() {
        let mut f = fixture();
        let uid = f.add(Some("a"), true);
        f.key("s");
        f.wait_for_screen(uid, "> ");
        f.set_process_status(uid, "busy");
        f.key("\x03");
        f.key("\t");
        f.key("\r");
        assert!(f.app.should_quit());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while f.session(uid).is_some_and(|s| s.is_live()) {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            let ev = f.rx.recv_timeout(left).expect("the agent was not stopped");
            f.app.handle(ev, std::time::Instant::now());
        }
    }

    #[test]
    fn y_confirms_and_escape_cancels() {
        let mut f = fixture();
        let uid = f.add(Some("a"), true);
        f.key("s");
        f.wait_for_screen(uid, "> ");
        f.set_process_status(uid, "busy");
        f.key("q");
        f.key("\x1b");
        assert!(f.app.quit_confirm.is_none() && !f.app.should_quit());
        f.key("q");
        f.key("y");
        assert!(f.app.should_quit());
    }
}
