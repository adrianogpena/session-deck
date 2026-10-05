//! Moving around the tree: cursor movement, collapse/expand, jump keys, and `[`/`]` over started
//! sessions. Port of `App.move`, `collapseOrParent`, `expandOrChild`, `cycleActiveSession` & co.

use std::time::Instant;

use sdeck_core::store::tree_prefs::{folder_node_key, project_node_key, set_collapsed};

use super::{same_row, App};
use crate::tree::{is_started, TreeRow};

impl App {
    /// Moves the selection by `delta` rows, skipping dividers and the usage block. Stays put when
    /// there is no selectable row that way.
    pub(super) fn move_selection(&mut self, delta: isize) {
        let mut i = self.selected as isize;
        loop {
            i += delta;
            match usize::try_from(i).ok().and_then(|u| self.rows.get(u)) {
                Some(row) if row.is_unselectable() => continue,
                Some(_) => {
                    self.selected = i as usize;
                    return;
                }
                None => return,
            }
        }
    }

    /// Selects the first folder or project row carrying jump key `n`.
    pub(super) fn select_hotkey(&mut self, n: u8) {
        self.select_where(|r| match r {
            TreeRow::Folder { hotkey, .. } | TreeRow::Project { hotkey, .. } => *hotkey == Some(n),
            _ => false,
        });
    }

    pub(super) fn select_where(&mut self, matches: impl Fn(&TreeRow) -> bool) -> bool {
        match self.rows.iter().position(matches) {
            Some(i) => {
                self.selected = i;
                true
            }
            None => false,
        }
    }

    fn node_key(row: &TreeRow) -> Option<(String, bool)> {
        match row {
            TreeRow::Folder {
                folder_id, collapsed, ..
            } => Some((folder_node_key(folder_id), *collapsed)),
            TreeRow::Project {
                project_key,
                collapsed,
                ..
            } => Some((project_node_key(project_key), *collapsed)),
            _ => None,
        }
    }

    fn toggle_collapsed(&mut self, node_key: String, collapsed: bool) {
        self.change_tree(|t| set_collapsed(&t, &node_key, !collapsed));
    }

    /// Tab: collapse or expand the selected folder or project.
    pub(super) fn toggle_selected_group(&mut self) {
        if let Some((key, collapsed)) = self.selected_row().and_then(Self::node_key) {
            self.toggle_collapsed(key, collapsed);
        }
    }

    /// Enter: attaches the selected session (09); on a group it collapses or expands; on a tag row
    /// it filters to that tag (again clears).
    pub(super) fn enter_selected_row(&mut self) {
        match self.selected_row() {
            Some(TreeRow::Tag { name, .. }) => {
                let name = name.clone();
                self.tag_filter = if self.tag_filter.as_ref() == Some(&name) {
                    None
                } else {
                    Some(name)
                };
                self.rebuild_rows();
            }
            Some(TreeRow::Folder { .. } | TreeRow::Project { .. }) => self.toggle_selected_group(),
            _ => {}
        }
    }

    /// ←: collapse an expanded group, otherwise go up to the parent row (session → project → folder).
    pub(super) fn collapse_or_parent(&mut self) {
        if let Some((key, false)) = self.selected_row().and_then(Self::node_key) {
            self.toggle_collapsed(key, false);
            return;
        }
        let depth = match self.selected_row() {
            Some(TreeRow::Session { depth, .. } | TreeRow::Project { depth, .. }) => *depth,
            _ => 0,
        };
        if depth == 0 {
            return;
        }
        let parent = self.rows[..self.selected].iter().rposition(|r| match r {
            TreeRow::Project { depth: d, .. } => *d < depth,
            TreeRow::Folder { .. } => true,
            _ => false,
        });
        if let Some(i) = parent {
            self.selected = i;
        }
    }

    /// →: expand a collapsed group, otherwise step into its first child.
    pub(super) fn expand_or_child(&mut self) {
        let Some((key, collapsed)) = self.selected_row().and_then(Self::node_key) else {
            return;
        };
        if collapsed {
            self.toggle_collapsed(key, true);
        } else if self
            .rows
            .get(self.selected + 1)
            .is_some_and(|r| !r.is_unselectable())
        {
            self.selected += 1;
        }
    }

    /// `` ` ``: back to the session selected before the current one, expanding its groups if collapsed.
    pub(super) fn select_previous_session(&mut self, now: Instant) {
        match self
            .previous_session
            .filter(|&uid| self.session_by_uid(uid).is_some())
        {
            Some(uid) => self.jump_to_session(uid, now),
            None => self.flash("No previous session.".into(), now),
        }
    }

    /// `]` / `[`: selects the next / previous started session (running, waiting or idle), wrapping
    /// around and skipping stopped or erroring ones, folders and projects. With
    /// `ui.expandCollapsedOnActiveJump` on (the default), a target hidden inside a collapsed group is
    /// still reached, expanding just that group; off, only rows already shown are targets.
    pub(super) fn cycle_active_session(&mut self, delta: isize, now: Instant) {
        let rows = if self.config.ui.expand_collapsed_on_active_jump {
            self.build(true).rows
        } else {
            self.rows.clone()
        };
        let started: Vec<(usize, u64)> = rows
            .iter()
            .enumerate()
            .filter_map(|(i, r)| match r {
                TreeRow::Session { uid, .. } => {
                    let s = self.session_by_uid(*uid)?;
                    is_started(self.procs.category_of(s)).then_some((i, *uid))
                }
                _ => None,
            })
            .collect();
        if started.is_empty() {
            self.flash("No started sessions.".into(), now);
            return;
        }
        let at = self
            .selected_row()
            .and_then(|current| rows.iter().position(|r| same_row(r, current)))
            .map(|i| i as isize);
        let target = if delta > 0 {
            started
                .iter()
                .find(|(i, _)| at.is_none_or(|at| *i as isize > at))
                .or(started.first())
        } else {
            started
                .iter()
                .rev()
                .find(|(i, _)| at.is_none_or(|at| (*i as isize) < at))
                .or(started.last())
        };
        if let Some(&(_, uid)) = target {
            self.jump_to_session(uid, now);
        }
    }

    /// Selects a session in the tree, expanding its project (and folder) if collapsed. Flashes if a
    /// filter still hides it.
    pub(super) fn jump_to_session(&mut self, uid: u64, now: Instant) {
        let is_target = |r: &TreeRow| matches!(r, TreeRow::Session { uid: u, .. } if *u == uid);
        if self.select_where(is_target) {
            return;
        }
        let Some(project_key) = self.session_by_uid(uid).map(|s| s.project_key.clone()) else {
            return;
        };
        let folder_id = self
            .tree
            .folders
            .iter()
            .find(|f| f.projects.contains(&project_key))
            .map(|f| f.id.clone());
        self.change_tree(|t| {
            let next = set_collapsed(&t, &project_node_key(&project_key), false);
            match &folder_id {
                Some(id) => set_collapsed(&next, &folder_node_key(id), false),
                None => next,
            }
        });
        if !self.select_where(is_target) {
            self.flash("Hidden by the filter. Press 0 to clear it.".into(), now);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use sdeck_core::status::claude_process_watcher::LiveClaudeProcess;
    use sdeck_core::store::deck_store::DeckStore;
    use sdeck_core::store::tree_prefs::FolderPrefs;

    use super::*;
    use crate::event::AppEvent;
    use crate::filters::{StatusCategory, TimeFilter};
    use crate::sessions::DeckSession;

    struct Fixture {
        app: App,
        now: Instant,
        _dir: tempfile::TempDir,
    }

    fn session(id: &str, project: &str, mtime: i64) -> DeckSession {
        let root = format!("C:\\repos\\{project}");
        let mut s = DeckSession::new("claude", Some(id), &root, id, mtime);
        s.project_key = root.to_lowercase();
        s
    }

    /// api: a1 (newest, started), a2; web: w1 (started); docs: d1 — alphabetical: api, docs, web.
    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let (tx, _rx) = mpsc::channel();
        let mut app = App::new(DeckStore::new(dir.path().join("state.json")), tx, None);
        for (id, status) in [("a1", "idle"), ("w1", "busy")] {
            let process = LiveClaudeProcess {
                pid: 1,
                session_id: id.into(),
                status: status.into(),
                cwd: None,
                account: sdeck_core::status::account::Account {
                    config_dir: "C:\\.claude".into(),
                    email: None,
                    is_default: true,
                },
            };
            app.procs.by_session.insert(id.into(), process);
        }
        app.set_sessions(vec![
            session("a1", "api", 50),
            session("a2", "api", 10),
            session("w1", "web", 40),
            session("d1", "docs", 30),
        ]);
        Fixture {
            app,
            now: Instant::now(),
            _dir: dir,
        }
    }

    impl Fixture {
        fn keys(&mut self, keys: &str) -> &mut Self {
            self.app.handle(AppEvent::Input(keys.into()), self.now);
            self
        }

        fn selected(&self) -> String {
            match self.app.selected_row().unwrap() {
                TreeRow::Folder { name, .. } => format!("F:{name}"),
                TreeRow::Project { label, collapsed, .. } => {
                    format!("P:{label}{}", if *collapsed { "+" } else { "" })
                }
                TreeRow::Session { uid, .. } => {
                    format!("s:{}", self.app.session_by_uid(*uid).unwrap().id.clone().unwrap())
                }
                TreeRow::Tag { name, .. } => format!("T:{name}"),
                _ => "?".into(),
            }
        }
    }

    #[test]
    fn the_first_session_is_selected_and_arrows_move_over_rows_without_wrapping() {
        let mut f = fixture();
        assert_eq!(f.selected(), "s:a1");
        f.keys("j");
        assert_eq!(f.selected(), "s:a2");
        f.keys("\x1b[B");
        assert_eq!(f.selected(), "P:docs");
        f.keys("k\x1bOA");
        assert_eq!(f.selected(), "s:a1");
        f.keys("kkkk");
        assert_eq!(f.selected(), "P:api");
        f.keys("jjjjjjjjjjjj");
        assert_eq!(f.selected(), "s:w1");
    }

    #[test]
    fn number_keys_jump_to_top_level_groups() {
        let mut f = fixture();
        f.keys("3");
        assert_eq!(f.selected(), "P:web");
        f.keys("2");
        assert_eq!(f.selected(), "P:docs");
        f.keys("9");
        assert_eq!(f.selected(), "P:docs");
    }

    #[test]
    fn left_collapses_then_goes_to_the_parent_and_right_expands_then_steps_in() {
        let mut f = fixture();
        f.keys("h");
        assert_eq!(f.selected(), "P:api");
        f.keys("h");
        assert_eq!(f.selected(), "P:api+");
        f.keys("l");
        assert_eq!(f.selected(), "P:api");
        f.keys("l");
        assert_eq!(f.selected(), "s:a1");
        f.keys("jh");
        assert_eq!(f.selected(), "P:api");
        f.keys("\t");
        assert_eq!(f.selected(), "P:api+");
        f.keys("\r");
        assert_eq!(f.selected(), "P:api");
        assert!(f.app.store.get_tree().collapsed.is_empty());
    }

    #[test]
    fn a_folder_collapse_persists_and_left_climbs_from_project_to_folder() {
        let mut f = fixture();
        f.app.tree.folders.push(FolderPrefs {
            id: "f1".into(),
            name: "Work".into(),
            projects: vec!["c:\\repos\\web".into()],
        });
        f.app.rebuild_rows();
        f.keys("1");
        assert_eq!(f.selected(), "F:Work");
        f.keys("j");
        assert_eq!(f.selected(), "P:web");
        f.keys("hh");
        assert_eq!(f.selected(), "F:Work");
        f.keys("h");
        assert!(f
            .app
            .rows
            .iter()
            .all(|r| !matches!(r, TreeRow::Project { label, .. } if label == "web")));
        assert!(f
            .app
            .store
            .get_tree()
            .collapsed
            .contains(&"folder:f1".to_string()));
    }

    #[test]
    fn brackets_cycle_through_started_sessions_expanding_collapsed_groups() {
        let mut f = fixture();
        f.keys("]");
        assert_eq!(f.selected(), "s:w1");
        f.keys("]");
        assert_eq!(f.selected(), "s:a1");
        f.keys("[");
        assert_eq!(f.selected(), "s:w1");
        f.keys("3");
        f.keys("h");
        assert_eq!(f.selected(), "P:web+");
        f.keys("[");
        assert_eq!(f.selected(), "s:a1");
        f.keys("]");
        assert_eq!(f.selected(), "s:w1");
        assert!(f.app.store.get_tree().collapsed.is_empty());
    }

    #[test]
    fn brackets_stay_on_visible_rows_when_expanding_is_off() {
        let mut f = fixture();
        f.app.config.ui.expand_collapsed_on_active_jump = false;
        f.keys("3");
        f.keys("hh");
        assert_eq!(f.selected(), "P:web+");
        f.keys("]");
        assert_eq!(f.selected(), "s:a1");
        f.keys("]");
        assert_eq!(f.selected(), "s:a1");
        assert!(!f.app.rows.iter().any(|r| matches!(r, TreeRow::Session { uid, .. } if f.app.session_by_uid(*uid).unwrap().id.as_deref() == Some("w1"))));
    }

    #[test]
    fn brackets_report_when_nothing_is_started() {
        let mut f = fixture();
        f.app.procs.by_session.clear();
        f.app.rebuild_rows();
        f.keys("]");
        assert_eq!(f.app.message, "No started sessions.");
    }

    #[test]
    fn backtick_returns_to_the_previous_session_and_reports_when_there_is_none() {
        let mut f = fixture();
        f.keys("`");
        assert_eq!(f.app.message, "No previous session.");
        f.keys("j");
        assert_eq!(f.selected(), "s:a2");
        f.keys("jj");
        assert_eq!(f.selected(), "s:d1");
        f.keys("`");
        assert_eq!(f.selected(), "s:a2");
        f.keys("`");
        assert_eq!(f.selected(), "s:d1");
    }

    #[test]
    fn backtick_expands_a_collapsed_group_to_reach_its_session() {
        let mut f = fixture();
        f.keys("jj");
        f.keys("j");
        assert_eq!(f.selected(), "s:d1");
        f.keys("kkk");
        assert_eq!(f.selected(), "s:a1");
        f.keys("2");
        f.keys("h");
        assert_eq!(f.selected(), "P:docs+");
        f.keys("`");
        assert_eq!(f.selected(), "s:d1");
        assert!(f.app.store.get_tree().collapsed.is_empty());
    }

    #[test]
    fn status_filter_keys_toggle_categories_and_zero_clears_every_filter() {
        let mut f = fixture();
        f.keys("!");
        assert_eq!(f.app.status_filter, [StatusCategory::Running]);
        assert_eq!(f.selected(), "P:web");
        f.keys("#");
        f.keys("!");
        assert_eq!(f.app.status_filter, [StatusCategory::Idle]);
        assert_eq!(f.selected(), "s:a1");
        f.keys("0");
        assert!(f.app.status_filter.is_empty());
        f.keys("*");
        assert_eq!(f.app.time_filter, TimeFilter::Today);
        f.keys("*");
        assert_eq!(f.app.time_filter, TimeFilter::ThreeDays);
        f.keys("0");
        assert_eq!(f.app.time_filter, TimeFilter::All);
    }

    #[test]
    fn a_filtered_out_selection_moves_to_the_nearest_surviving_row() {
        let mut f = fixture();
        f.keys("jj");
        assert_eq!(f.selected(), "P:docs");
        f.keys("&");
        assert_eq!(
            f.app
                .rows
                .iter()
                .filter(|r| matches!(r, TreeRow::Session { .. }))
                .count(),
            0
        );
        f.keys("0");
        f.keys("jjj");
        assert_eq!(f.selected(), "s:d1");
        f.keys("!");
        assert_eq!(f.selected(), "P:web");
    }

    #[test]
    fn sort_and_view_keys_flip_and_persist_with_a_flash() {
        let mut f = fixture();
        f.keys("S");
        assert_eq!(
            f.app.message,
            "Sessions: needs attention first (error, waiting, running, idle)"
        );
        assert_eq!(
            f.app.store.get_tree().sort,
            sdeck_core::store::tree_prefs::SessionSort::Actionable
        );
        f.keys("S");
        assert_eq!(f.app.message, "Sessions: most recent first");
        f.keys("t");
        assert_eq!(
            f.app.message,
            "View: groups with running or waiting sessions on top"
        );
        assert_eq!(f.selected(), "s:a1");
        assert!(matches!(f.app.rows[0], TreeRow::Project { ref label, .. } if label == "web"));
        f.keys("t");
        assert_eq!(f.app.message, "View: normal");
    }

    #[test]
    fn enter_on_a_tag_row_filters_to_that_tag_and_again_clears_it() {
        let mut f = fixture();
        f.app
            .store
            .update_session(
                "d1",
                &sdeck_core::store::deck_store::SessionPatch {
                    tags: sdeck_core::store::deck_store::Patch::Set(vec!["urgent".into()]),
                    ..Default::default()
                },
            )
            .unwrap();
        f.app.rebuild_rows();
        let tag_index = f
            .app
            .rows
            .iter()
            .position(|r| matches!(r, TreeRow::Tag { .. }))
            .unwrap();
        f.app.selected = tag_index;
        f.keys("\r");
        assert_eq!(f.app.tag_filter.as_deref(), Some("urgent"));
        assert_eq!(
            f.app
                .rows
                .iter()
                .filter(|r| matches!(r, TreeRow::Session { .. }))
                .count(),
            1
        );
        let tag_index = f
            .app
            .rows
            .iter()
            .position(|r| matches!(r, TreeRow::Tag { .. }))
            .unwrap();
        f.app.selected = tag_index;
        f.keys("\r");
        assert_eq!(f.app.tag_filter, None);
    }

    #[test]
    fn a_project_scope_lists_only_that_project_at_the_top_level_with_its_own_tags_and_0_clears_it() {
        let mut f = fixture();
        for (id, name, projects) in [
            (
                "f1",
                "Work",
                vec!["c:\\repos\\web".to_string(), "c:\\repos\\api".to_string()],
            ),
            ("f2", "Empty", Vec::new()),
        ] {
            f.app
                .tree
                .folders
                .push(sdeck_core::store::tree_prefs::FolderPrefs {
                    id: id.into(),
                    name: name.into(),
                    projects,
                });
        }
        for (id, tag) in [("w1", "web-tag"), ("a1", "api-tag")] {
            f.app
                .store
                .update_session(
                    id,
                    &sdeck_core::store::deck_store::SessionPatch {
                        tags: sdeck_core::store::deck_store::Patch::Set(vec![tag.into()]),
                        ..Default::default()
                    },
                )
                .unwrap();
        }
        f.app.project_scope = Some(("c:\\repos\\web".into(), "C:\\repos\\web".into()));
        f.app.rebuild_rows();
        assert!(!f.app.rows.iter().any(|r| matches!(r, TreeRow::Folder { .. })));
        assert!(matches!(
            f.app.rows[0],
            TreeRow::Project { ref label, depth: 0, .. } if label == "web"
        ));
        assert_eq!(
            f.app
                .rows
                .iter()
                .filter(|r| matches!(r, TreeRow::Project { .. }))
                .count(),
            1
        );
        let tags: Vec<&str> = f
            .app
            .rows
            .iter()
            .filter_map(|r| match r {
                TreeRow::Tag { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(tags, vec!["web-tag"]);
        f.keys("0");
        assert_eq!(f.app.project_scope, None);
        assert!(f
            .app
            .rows
            .iter()
            .any(|r| matches!(r, TreeRow::Folder { name, .. } if name == "Work")));
    }

    #[test]
    fn a_project_scope_with_no_sessions_is_the_project_new_sessions_start_in() {
        let mut f = fixture();
        let scope = ("c:\\repos\\new".to_string(), "C:\\repos\\new".to_string());
        f.app.project_scope = Some(scope.clone());
        f.app.rebuild_rows();
        assert!(!f.app.rows.iter().any(|r| matches!(r, TreeRow::Session { .. })));
        assert_eq!(f.app.selected_project(), Some(scope));
    }

    #[test]
    fn rediscovery_keeps_the_selection_on_the_same_session() {
        let mut f = fixture();
        f.keys("jjj");
        assert_eq!(f.selected(), "s:d1");
        let found = vec![
            found("a1", "api", 50),
            found("a2", "api", 10),
            found("w1", "web", 40),
            found("d1", "docs", 31),
            found("n1", "api", 5),
        ];
        f.app.handle(AppEvent::Discovered(found), f.now);
        assert_eq!(f.selected(), "s:d1");
        assert_eq!(f.app.sessions.len(), 5);
    }

    fn found(id: &str, project: &str, mtime: i64) -> crate::sessions::FoundSession {
        let root = format!("C:\\repos\\{project}");
        crate::sessions::FoundSession {
            agent: "claude".into(),
            id: id.into(),
            file: None,
            cwd: root.clone(),
            project_root: root.clone(),
            project_key: root.to_lowercase(),
            title: id.into(),
            mtime_ms: mtime,
            account: None,
        }
    }
}
