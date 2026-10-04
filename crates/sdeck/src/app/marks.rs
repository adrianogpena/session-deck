//! Seen marks (`u`, `U`) and pins (`,`). Port of the matching cases in `App.onKey` and `cyclePin`.

use std::time::Instant;

use sdeck_core::status::session_status::{acknowledge_session_status, mark_session_unseen};
use sdeck_core::store::deck_store::{Patch, SessionPatch};
use sdeck_core::store::tree_prefs::SessionPin;

use super::App;

/// none → top → bottom → none.
fn next_pin(current: Option<SessionPin>) -> Option<SessionPin> {
    match current {
        None => Some(SessionPin::Top),
        Some(SessionPin::Top) => Some(SessionPin::Bottom),
        Some(SessionPin::Bottom) => None,
    }
}

impl App {
    /// `u`: a seen "done"/"error" asks for attention again.
    pub(super) fn mark_unread(&mut self, now: Instant) {
        match self.selected_session().and_then(|s| s.id.clone()) {
            Some(id) => {
                let _ = mark_session_unseen(&self.store, &id);
                self.poll_procs(now);
                self.flash("Marked as unread".into(), now);
            }
            None => self.flash("Select a session to mark as unread.".into(), now),
        }
    }

    /// `U`: acknowledges the session's current "done"/"error".
    pub(super) fn mark_read(&mut self, now: Instant) {
        match self.selected_session().and_then(|s| s.id.clone()) {
            Some(id) => {
                let _ = acknowledge_session_status(&self.store, &id);
                self.poll_procs(now);
                self.flash("Marked as read".into(), now);
            }
            None => self.flash("Select a session to mark as read.".into(), now),
        }
    }

    pub(super) fn cycle_pin(&mut self, now: Instant) {
        let Some(s) = self.selected_session() else {
            self.flash("Select a session to pin.".into(), now);
            return;
        };
        let Some(id) = s.id.clone() else {
            self.flash(
                "Send something first: a new session has nothing to pin yet.".into(),
                now,
            );
            return;
        };
        let pin = next_pin(self.store.get_session(&id).and_then(|p| p.pin));
        let patch = SessionPatch {
            pin: pin.map_or(Patch::Clear, Patch::Set),
            ..Default::default()
        };
        let _ = self.store.update_session(&id, &patch);
        self.rebuild_rows();
        let text = match pin {
            Some(pin) => format!("Pinned to the {} of its project", pin.as_str()),
            None => "Unpinned".into(),
        };
        self.flash(text, now);
    }
}

#[cfg(test)]
mod tests {
    use sdeck_core::status::session_status::{
        read_session_status, write_session_status, SessionStatus as CoreStatus,
    };
    use sdeck_core::store::tree_prefs::SessionPin;

    use crate::app::test_fixture::fixture;
    use crate::sessions::SessionStatus;

    #[test]
    fn comma_cycles_top_bottom_none() {
        let mut f = fixture();
        let uid = f.add(Some("abc"), true);
        let pin = |f: &crate::app::test_fixture::Fixture| f.app.store.get_session("abc").and_then(|p| p.pin);
        f.key(",");
        assert_eq!(pin(&f), Some(SessionPin::Top));
        assert_eq!(f.app.message, "Pinned to the top of its project");
        f.key(",");
        assert_eq!(pin(&f), Some(SessionPin::Bottom));
        f.key(",");
        assert_eq!(pin(&f), None);
        assert_eq!(f.app.message, "Unpinned");

        f.app.session_mut(uid).unwrap().id = None;
        f.key(",");
        assert_eq!(
            f.app.message,
            "Send something first: a new session has nothing to pin yet."
        );
    }

    #[test]
    fn a_done_session_reads_as_seen_with_capital_u_and_unseen_again_with_u() {
        let mut f = fixture();
        let uid = f.add(Some("abc"), true);
        write_session_status("abc", CoreStatus::Done).unwrap();
        assert!(read_session_status("abc").is_some());
        f.app.procs.poll(&f.app.store, &f.app.accounts);
        assert_eq!(
            f.app.procs.status_of(f.session(uid).unwrap()),
            SessionStatus::Done
        );

        f.key("U");
        assert_eq!(f.app.message, "Marked as read");
        assert_eq!(
            f.app.procs.status_of(f.session(uid).unwrap()),
            SessionStatus::Stopped
        );

        f.key("u");
        assert_eq!(f.app.message, "Marked as unread");
        assert_eq!(
            f.app.procs.status_of(f.session(uid).unwrap()),
            SessionStatus::Done
        );
    }

    #[test]
    fn marks_need_a_selected_session() {
        let mut f = fixture();
        f.key("u");
        assert_eq!(f.app.message, "Select a session to mark as unread.");
        f.key("U");
        assert_eq!(f.app.message, "Select a session to mark as read.");
        f.key(",");
        assert_eq!(f.app.message, "Select a session to pin.");
    }
}
