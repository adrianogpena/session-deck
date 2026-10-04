//! `o`: a one-line prompt sent without attaching. Port of `App.openPromptInput` and `sendPrompt`;
//! the queued text is typed by `send_pending_prompt` (lifecycle).

use std::time::Instant;

use super::input::PromptAction;
use super::App;
use crate::ansi::one_line;
use crate::sessions::{display_title, SessionStatus};

impl App {
    pub(super) fn open_prompt_input(&mut self, now: Instant) {
        let Some(s) = self.selected_session() else {
            self.flash("Select a session to send a prompt to.".into(), now);
            return;
        };
        if self.procs.is_elsewhere(s) {
            self.flash(
                "That session is running in another terminal. Send it from there.".into(),
                now,
            );
        } else if self.procs.status_of(s) == SessionStatus::Waiting {
            self.flash(
                "It is waiting for an answer. Attach (Enter) to reply.".into(),
                now,
            );
        } else {
            let label = format!("Prompt for {}", display_title(s, &self.store));
            let action = PromptAction::SendPrompt(s.uid);
            self.open_prompt(label, String::new(), action);
        }
    }

    /// A stopped session is started first and gets the text once it is idle and quiet.
    pub(super) fn send_prompt(&mut self, uid: u64, raw: &str, now: Instant) {
        let text = one_line(raw);
        if text.is_empty() || !self.ensure_live(uid, now) {
            return;
        }
        if let Some(s) = self.session_mut(uid) {
            s.pending_prompt = Some(text);
        }
        self.send_pending_prompt(uid, now);
        let queued = self
            .session_by_uid(uid)
            .is_some_and(|s| s.pending_prompt.is_some());
        let text = if queued {
            "Starting… the prompt is sent once it is ready"
        } else {
            "Prompt sent"
        };
        self.flash(text.into(), now);
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use crate::app::test_fixture::fixture;

    #[test]
    fn o_starts_a_stopped_session_and_types_the_prompt_once_it_is_idle_and_quiet() {
        let mut f = fixture();
        let uid = f.add(Some("p1"), true);
        f.key("o");
        assert_eq!(f.app.prompt.as_ref().unwrap().label, "Prompt for a session");
        f.key("say  hi\r");
        assert!(f.app.prompt.is_none());
        assert!(f.session(uid).unwrap().is_live());
        assert_eq!(f.app.message, "Starting… the prompt is sent once it is ready");
        f.wait_for_screen(uid, "> ");

        f.set_process_status(uid, "idle");
        let now = Instant::now();
        f.app.send_pending_prompt(uid, now);
        assert!(f.session(uid).unwrap().pending_prompt.is_some());
        f.app.send_pending_prompt(uid, now + Duration::from_millis(1000));
        assert!(f.session(uid).unwrap().pending_prompt.is_none());
        f.app
            .handle(crate::event::AppEvent::Tick, now + Duration::from_millis(1300));
        f.wait_for_screen(uid, "echo: say hi");
    }

    #[test]
    fn a_busy_session_gets_the_prompt_only_once_it_is_idle() {
        let mut f = fixture();
        let uid = f.add(Some("p1"), true);
        f.key("s");
        f.wait_for_screen(uid, "> ");
        f.set_process_status(uid, "busy");
        f.key("o");
        f.key("later\r");
        assert_eq!(f.app.message, "Starting… the prompt is sent once it is ready");
        let now = Instant::now() + Duration::from_secs(2);
        f.app.send_pending_prompt(uid, now);
        assert!(f.session(uid).unwrap().pending_prompt.is_some());
        f.set_process_status(uid, "idle");
        f.app.send_pending_prompt(uid, now);
        assert!(f.session(uid).unwrap().pending_prompt.is_none());
    }

    #[test]
    fn a_waiting_session_refuses_a_prompt() {
        let mut f = fixture();
        let uid = f.add(Some("p1"), true);
        f.key("s");
        f.wait_for_screen(uid, "> ");
        f.set_process_status(uid, "waiting");
        f.key("o");
        assert!(f.app.prompt.is_none());
        assert_eq!(
            f.app.message,
            "It is waiting for an answer. Attach (Enter) to reply."
        );
    }
}
