//! Archive (`A`), the archived view (`^`) and removing a project (`d`). Port of `toggleArchived`
//! and `removeProject`.

use std::time::Instant;

use sdeck_core::store::deck_store::{Patch, SessionPatch};

use super::input::{Confirm, ConfirmAction};
use super::App;
use crate::sessions::{DeckSession, SessionStatus};

impl App {
    /// Whether archiving would interrupt a turn: a live session that's running or waiting.
    pub(super) fn is_working(&self, s: &DeckSession) -> bool {
        s.is_live()
            && matches!(
                self.procs.status_of(s),
                SessionStatus::Running | SessionStatus::Waiting
            )
    }

    pub(super) fn set_archived(&mut self, ids: &[String], archived: bool) {
        let patch = SessionPatch {
            archived: Patch::Set(archived),
            ..Default::default()
        };
        let patches: Vec<(String, SessionPatch)> = ids.iter().map(|id| (id.clone(), patch.clone())).collect();
        let _ = self.store.update_sessions(&patches);
        self.rebuild_rows();
    }

    /// `A`: archives (hides) the session, or unarchives it in the archived view. An idle live
    /// session is stopped first; a working one is refused.
    pub(super) fn toggle_archived(&mut self, now: Instant) {
        if !self.multi_selected.is_empty() {
            self.bulk_archive(now);
            return;
        }
        let Some((uid, id)) = self
            .selected_session()
            .and_then(|s| s.id.clone().map(|id| (s.uid, id)))
        else {
            self.flash("Select a session to archive.".into(), now);
            return;
        };
        let archive = !self.store.get_session(&id).is_some_and(|p| p.archived);
        if archive {
            if self.session_by_uid(uid).is_some_and(|s| self.is_working(s)) {
                self.flash(
                    "It is still working. Stop it (x) or wait until it is idle to archive it.".into(),
                    now,
                );
                return;
            }
            if self.session_by_uid(uid).is_some_and(DeckSession::is_live) {
                self.kill(uid);
            }
        }
        self.set_archived(&[id], archive);
        self.flash(
            if archive {
                "Archived (^ shows archived sessions)"
            } else {
                "Unarchived"
            }
            .into(),
            now,
        );
    }

    /// `^`: archived sessions only, or back to the active ones.
    pub(super) fn toggle_archived_view(&mut self, now: Instant) {
        self.archived_view = !self.archived_view;
        self.rebuild_rows();
        let text = if self.archived_view {
            "Archived sessions (A to unarchive, ^ to go back)"
        } else {
            "Active sessions"
        };
        self.flash(text.into(), now);
    }

    /// `d` on a project: removes it from the list. Nothing on disk is touched and a running session
    /// keeps running; `p` adds the project back.
    pub(super) fn remove_project(&mut self, key: String, label: &str) {
        self.confirm = Some(Confirm {
            question: format!("Remove project \"{label}\" from the list? (p to add it back)"),
            action: ConfirmAction::RemoveProject(key),
        });
    }

    pub(super) fn remove_project_now(&mut self, key: &str, _now: Instant) {
        let _ = self.store.set_project_hidden(key, true);
        self.rebuild_rows();
    }
}

#[cfg(test)]
mod tests {
    use crate::app::test_fixture::fixture;
    use crate::tree::TreeRow;

    fn archived(f: &crate::app::test_fixture::Fixture, id: &str) -> bool {
        f.app.store.get_session(id).is_some_and(|p| p.archived)
    }

    #[test]
    fn an_idle_live_session_is_stopped_then_archived() {
        let mut f = fixture();
        let uid = f.add(Some("abc"), true);
        f.key("s");
        f.wait_for_screen(uid, "> ");
        f.set_process_status(uid, "idle");
        f.key("A");
        assert!(archived(&f, "abc"));
        assert!(!f.session(uid).unwrap().is_live());
        assert_eq!(f.app.message, "Archived (^ shows archived sessions)");
        assert!(!f.app.rows.iter().any(|r| matches!(r, TreeRow::Session { .. })));
    }

    #[test]
    fn a_working_session_is_refused() {
        let mut f = fixture();
        let uid = f.add(Some("abc"), true);
        f.key("s");
        f.wait_for_screen(uid, "> ");
        f.set_process_status(uid, "busy");
        f.key("A");
        assert!(!archived(&f, "abc"));
        assert!(f.session(uid).unwrap().is_live());
        assert!(f.app.message.starts_with("It is still working."));
        f.app.kill(uid);
    }

    #[test]
    fn caret_shows_only_archived_sessions_and_a_unarchives() {
        let mut f = fixture();
        f.add(Some("one"), true);
        let two = f.add(Some("two"), true);
        f.key("A");
        assert!(archived(&f, "two"));

        f.key("^");
        assert!(f.app.archived_view);
        assert_eq!(f.app.message, "Archived sessions (A to unarchive, ^ to go back)");
        let shown: Vec<u64> = f
            .app
            .rows
            .iter()
            .filter_map(|r| match r {
                TreeRow::Session { uid, .. } => Some(*uid),
                _ => None,
            })
            .collect();
        assert_eq!(shown, [two]);

        f.select(two);
        f.key("A");
        assert!(!archived(&f, "two"));
        assert_eq!(f.app.message, "Unarchived");
        f.key("^");
        assert_eq!(f.app.message, "Active sessions");
        assert_eq!(
            f.app
                .rows
                .iter()
                .filter(|r| matches!(r, TreeRow::Session { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn d_on_a_project_asks_and_y_hides_it() {
        let mut f = fixture();
        f.add(Some("abc"), true);
        assert!(f.app.select_where(|r| matches!(r, TreeRow::Project { .. })));
        f.key("d");
        assert!(f.app.confirm.is_some());
        f.key("n");
        assert!(f.app.confirm.is_none());
        assert!(f.app.store.get_hidden_projects().is_empty());

        f.key("d");
        f.key("y");
        assert_eq!(f.app.store.get_hidden_projects().len(), 1);
        assert!(!f.app.rows.iter().any(|r| matches!(r, TreeRow::Session { .. })));
    }
}
