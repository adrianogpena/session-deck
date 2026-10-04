//! New sessions. Port of `App.newSession`, `startNewSession` and `applyPrependSession`.

use std::time::Instant;

use sdeck_core::agent_catalog::{agent_display_name, all_agent_ids, is_builtin_agent};
use sdeck_core::discovery::git_project::resolve_project_root;
use sdeck_core::discovery::path_utils::{expand_home, normalize_fs_path};
use sdeck_core::format::now_ms;
use sdeck_core::store::deck_store::{Patch, UiPatch};
use sdeck_core::store::tree_prefs::{prepend_session, rename_session_id, TreePrefs};

use super::input::{Picker, PickerAction, PromptAction};
use super::App;
use crate::sessions::DeckSession;
use crate::tree::TreeRow;

impl App {
    /// The project the selection belongs to, as (key, root): the project row itself, or a
    /// session's project.
    pub(super) fn selected_project(&self) -> Option<(String, String)> {
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

    fn agent_enabled(&self, agent: &str) -> bool {
        self.config.tools.get(agent).and_then(|t| t.enabled) != Some(false)
    }

    /// `F3` sets the agent `n` starts; `N` picks one for a single new session. Lists enabled agents
    /// the machine can actually run — offering one that isn't installed could only end in a
    /// "not found" flash.
    pub(super) fn open_agent_picker(&mut self, set_active: bool, now: Instant) {
        let candidates: Vec<String> = all_agent_ids()
            .into_iter()
            .filter(|id| {
                self.agent_enabled(id)
                    && (is_builtin_agent(id) || self.executables.is_available(id, &self.config))
            })
            .map(str::to_string)
            .collect();
        if candidates.is_empty() {
            self.flash("No agent is both enabled and installed.".into(), now);
            return;
        }
        let items = candidates
            .iter()
            .map(|id| {
                let mark = if *id == self.active_agent { '*' } else { ' ' };
                format!("{mark} {}", agent_display_name(id))
            })
            .collect();
        self.picker = Some(Picker {
            title: if set_active {
                "Session Deck Agent".into()
            } else {
                "New session · choose an agent".into()
            },
            index: candidates
                .iter()
                .position(|id| *id == self.active_agent)
                .unwrap_or(0),
            items,
            action: if set_active {
                PickerAction::SetActiveAgent(candidates)
            } else {
                PickerAction::NewSession(candidates)
            },
        });
        self.dirty = true;
    }

    pub(super) fn set_active_agent(&mut self, agent: &str, now: Instant) {
        self.active_agent = agent.to_string();
        let _ = self.store.update_ui(&UiPatch {
            active_agent: Patch::Set(agent.to_string()),
            ..UiPatch::default()
        });
        self.flash(format!("Session Deck Agent: {}", agent_display_name(agent)), now);
    }

    /// `p`: a new session in any folder, which lists its project once a prompt is sent.
    pub(super) fn open_add_project(&mut self) {
        self.open_prompt("Project folder".into(), String::new(), PromptAction::AddProject);
    }

    pub(super) fn add_project(&mut self, input: &str, now: Instant) {
        let raw = input.trim();
        let raw = raw
            .strip_prefix('"')
            .and_then(|r| r.strip_suffix('"'))
            .unwrap_or(raw);
        if raw.is_empty() {
            return;
        }
        let expanded = expand_home(raw);
        let path = std::path::absolute(&expanded).unwrap_or_else(|_| expanded.clone().into());
        let cwd = path.to_string_lossy().into_owned();
        if !path.is_dir() {
            self.flash(format!("Not a folder: {cwd}"), now);
            return;
        }
        let agent = if self.agent_enabled(&self.active_agent) {
            Some(self.active_agent.clone())
        } else {
            all_agent_ids()
                .into_iter()
                .find(|id| self.agent_enabled(id))
                .map(str::to_string)
        };
        let Some(agent) = agent else {
            self.flash("Every agent is disabled in the config.".into(), now);
            return;
        };
        let root = resolve_project_root(&cwd).root;
        let key = normalize_fs_path(&root);
        // Explicitly added here: undoes a previous `d` removal, if any.
        let _ = self.store.set_project_hidden(&key, false);
        self.start_new_session(&agent, &cwd, key, root, now);
    }
}

/// A session that got a new id (`/clear`) keeps its manual position; one that had none is
/// put at the top of its project. See `rename_session_id`.
pub(super) fn id_change(
    project_key: String,
    old_id: String,
    new_id: String,
) -> impl Fn(TreePrefs) -> TreePrefs {
    move |tree| {
        let renamed = rename_session_id(&tree, &project_key, &old_id, &new_id);
        if renamed != tree {
            renamed
        } else {
            prepend_session(&tree, &project_key, &new_id)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::test_fixture::fixture;
    use crate::tree::TreeRow;

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

    #[test]
    fn n_starts_in_the_selected_sessions_folder_and_the_project_root_from_a_project_row() {
        let mut f = fixture();
        let old = f.add(Some("abc"), true);
        let sub = f.home.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        f.app.session_mut(old).unwrap().cwd = sub.to_string_lossy().into_owned();
        f.key("n");
        let fresh = f.app.attached.expect("attached");
        assert_eq!(f.session(fresh).unwrap().cwd, sub.to_string_lossy());
        f.key("\x0bq");

        f.app.select_where(|r| matches!(r, TreeRow::Project { .. }));
        f.key("n");
        let fresh = f.app.attached.expect("attached");
        let root = f.session(old).unwrap().project_root.clone();
        assert_eq!(f.session(fresh).unwrap().cwd, root);
    }

    #[test]
    fn the_pickers_set_the_default_agent_or_start_a_one_off_session() {
        let mut f = fixture();
        f.add(Some("abc"), true);
        f.key("\x1bOR");
        let picker = f.app.picker.as_ref().expect("picker");
        assert_eq!(picker.title, "Session Deck Agent");
        assert!(picker.items[0].starts_with("* Claude"), "{:?}", picker.items);
        f.key("\x1b");
        assert!(f.app.picker.is_none());

        f.key("N");
        assert_eq!(
            f.app.picker.as_ref().unwrap().title,
            "New session · choose an agent"
        );
        f.key("\r");
        assert!(f.app.attached.is_some());
        assert_eq!(f.app.active_agent, "claude");

        f.app.config.tools.get_mut("claude").unwrap().enabled = Some(false);
        f.key("\x0bq");
        f.key("\x1bOR");
        let picker = f.app.picker.as_ref().expect("copilot is still listed");
        assert!(!picker.items.iter().any(|i| i.contains("Claude")));
        f.key("\r");
        assert_eq!(f.app.active_agent, "copilot");
        assert_eq!(f.app.store.get_ui().active_agent.as_deref(), Some("copilot"));
    }

    #[test]
    fn p_adds_a_folder_as_a_project_and_refuses_one_that_does_not_exist() {
        let mut f = fixture();
        f.add(Some("abc"), true);
        let folder = f.home.path().join("proj");
        std::fs::create_dir(&folder).unwrap();
        f.key("p");
        f.key(&format!("\"{}\"", folder.display()));
        f.key("\r");
        let fresh = f.app.attached.expect("attached");
        let s = f.session(fresh).unwrap();
        assert_eq!(s.cwd, folder.to_string_lossy());
        assert_eq!(s.title, "(new session)");
        f.key("\x0bq");

        f.key("p");
        f.key("nowhere");
        f.key("\r");
        assert!(f.app.message.starts_with("Not a folder"), "{}", f.app.message);
    }
}
