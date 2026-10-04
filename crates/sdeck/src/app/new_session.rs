//! New sessions. Port of `App.newSession`, `startNewSession` and `applyPrependSession`.

use std::time::Instant;

use sdeck_core::agent_catalog::agent_display_name;
use sdeck_core::format::now_ms;
use sdeck_core::store::tree_prefs::prepend_session;

use super::App;
use crate::sessions::DeckSession;
use crate::tree::TreeRow;

impl App {
    /// The project the selection belongs to, as (key, root): the project row itself, or a
    /// session's project.
    fn selected_project(&self) -> Option<(String, String)> {
        match self.selected_row()? {
            TreeRow::Project {
                project_key, root, ..
            } => Some((project_key.clone(), root.clone())),
            TreeRow::Session { uid, .. } => self
                .session_by_uid(*uid)
                .map(|s| (s.project_key.clone(), s.project_root.clone())),
            _ => None,
        }
    }

    /// A new `agent` session in the selected project: in the selected session's folder (a
    /// subfolder stays a subfolder), else the project root.
    pub(super) fn new_session(&mut self, agent: &str, now: Instant) {
        if self.config.tools.get(agent).and_then(|t| t.enabled) == Some(false) {
            let text = format!(
                "{} is disabled (tools.{agent}.enabled: false in the config).",
                agent_display_name(agent)
            );
            self.flash(text, now);
            return;
        }
        let Some((key, root)) = self.selected_project() else {
            self.flash(
                "Select a project or session to start a new session in.".into(),
                now,
            );
            return;
        };
        let cwd = self
            .selected_session()
            .map_or_else(|| root.clone(), |s| s.cwd.clone());
        self.start_new_session(agent, &cwd, key, root, now);
    }

    /// Lists the new session at the top of its project, selects it and attaches (or interacts, per
    /// `ui.newSessionFullScreen`). Claude reports its own id once started; every other agent takes
    /// a pre-assigned one, so its status file is keyed by id from the start.
    pub(super) fn start_new_session(
        &mut self,
        agent: &str,
        cwd: &str,
        key: String,
        root: String,
        now: Instant,
    ) {
        let claude = agent == "claude";
        let id = (!claude).then(|| uuid::Uuid::new_v4().to_string());
        let title = if claude {
            "(new session)".to_string()
        } else {
            format!("(new {} session)", agent_display_name(agent))
        };
        let mut fresh = DeckSession::new(agent, id.as_deref(), cwd, &title, now_ms());
        fresh.is_new = !claude;
        fresh.project_root = root;
        fresh.project_key = key.clone();
        if !self.config.ui.recent_sessions_first {
            // Claude's position is recorded once it reports its id (`adopt_live_ids`).
            match &id {
                Some(id) => self.apply_prepend_session(&key, id),
                None => fresh.pending_top_order = true,
            }
        }
        let uid = fresh.uid;
        self.sessions.insert(0, fresh);
        self.rebuild_rows();
        self.select_where(|r| matches!(r, TreeRow::Session { uid: u, .. } if *u == uid));
        if self.config.ui.new_session_full_screen {
            self.attach(uid, now);
        } else {
            self.start_interacting(uid, now);
        }
    }

    pub(super) fn apply_prepend_session(&mut self, project_key: &str, session_id: &str) {
        self.change_tree(|t| prepend_session(&t, project_key, session_id));
    }
}

#[cfg(test)]
mod tests {
    use crate::app::test_fixture::fixture;

    #[test]
    fn ctrl_k_n_starts_a_new_claude_session_in_the_same_folder() {
        let mut f = fixture();
        let old = f.add(Some("abc"), true);
        f.key("\r");
        assert_eq!(f.app.attached, Some(old));
        f.key("\x0bn");
        let fresh = f.app.attached.expect("attached to the new session");
        assert_ne!(fresh, old);
        let s = f.session(fresh).unwrap();
        assert_eq!(s.title, "(new session)");
        assert_eq!(s.cwd, f.session(old).unwrap().cwd);
        assert!(s.pending_top_order || f.app.config.ui.recent_sessions_first);
        f.wait_for_screen(fresh, "ARGS=");
    }

    #[test]
    fn new_session_interacts_when_full_screen_is_off_and_flashes_when_disabled() {
        let mut f = fixture();
        f.app.config.ui.new_session_full_screen = false;
        f.add(Some("abc"), true);
        f.app.new_session("claude", std::time::Instant::now());
        assert!(f.app.interacting.is_some());
        f.key("\x11");

        f.app.config.tools.get_mut("claude").unwrap().enabled = Some(false);
        f.app.new_session("claude", std::time::Instant::now());
        assert_eq!(
            f.app.message,
            "Claude is disabled (tools.claude.enabled: false in the config)."
        );
    }
}
