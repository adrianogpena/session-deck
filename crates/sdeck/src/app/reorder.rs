//! `K`/`J` and `Shift+↑/↓`: move the selected folder, project, or session (within its project).
//! Port of `reorder`.

use std::time::Instant;

use sdeck_core::store::tree_prefs::{move_folder, move_project, move_session, GroupView};

use super::App;
use crate::tree::TreeRow;

impl App {
    pub(super) fn reorder(&mut self, delta: isize, now: Instant) {
        if self.tree.view == GroupView::Active {
            self.flash("Switch to the normal view (t) to reorder.".into(), now);
            return;
        }
        match self.selected_row().cloned() {
            Some(TreeRow::Folder { folder_id, .. }) => {
                self.change_tree(|t| move_folder(&t, &folder_id, delta));
            }
            Some(TreeRow::Session { uid, .. }) => {
                let Some((id, project_key)) = self
                    .session_by_uid(uid)
                    .and_then(|s| s.id.clone().map(|id| (id, s.project_key.clone())))
                else {
                    return;
                };
                let displayed = self
                    .session_containers
                    .get(&project_key)
                    .cloned()
                    .unwrap_or_default();
                self.change_tree(|t| move_session(&t, &project_key, &id, delta, &displayed));
            }
            Some(TreeRow::Project { project_key, .. }) => {
                let container = self
                    .tree
                    .folders
                    .iter()
                    .find(|f| f.projects.contains(&project_key))
                    .map(|f| f.id.clone())
                    .unwrap_or_default();
                let displayed = self.containers.get(&container).cloned().unwrap_or_default();
                self.change_tree(|t| move_project(&t, &project_key, delta, &displayed));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use sdeck_core::store::tree_prefs::FolderPrefs;

    use crate::app::test_fixture::{fixture, Fixture};
    use crate::sessions::DeckSession;
    use crate::tree::TreeRow;

    fn project_labels(f: &Fixture) -> Vec<String> {
        f.app
            .rows
            .iter()
            .filter_map(|r| match r {
                TreeRow::Project { label, .. } => Some(label.clone()),
                _ => None,
            })
            .collect()
    }

    fn session_uids(f: &Fixture) -> Vec<u64> {
        f.app
            .rows
            .iter()
            .filter_map(|r| match r {
                TreeRow::Session { uid, .. } => Some(*uid),
                _ => None,
            })
            .collect()
    }

    fn two_projects() -> Fixture {
        let mut f = fixture();
        for (id, dir) in [("a1", "alpha"), ("b1", "beta")] {
            let root = f.home.path().join(dir).to_string_lossy().into_owned();
            let mut s = DeckSession::new("claude", Some(id), &root, id, 1);
            s.project_key = root.to_lowercase();
            s.project_root = root;
            f.app.sessions.push(s);
        }
        f.app.rebuild_rows();
        f.app
            .select_where(|r| matches!(r, TreeRow::Project { label, .. } if label == "alpha"));
        f
    }

    #[test]
    fn j_moves_a_project_down_and_k_back_up() {
        let mut f = two_projects();
        assert_eq!(project_labels(&f), ["alpha", "beta"]);
        f.key("J");
        assert_eq!(project_labels(&f), ["beta", "alpha"]);
        f.key("K");
        assert_eq!(project_labels(&f), ["alpha", "beta"]);
    }

    #[test]
    fn shift_arrows_reorder_and_the_ends_do_nothing() {
        let mut f = two_projects();
        f.key("\x1b[1;2A");
        assert_eq!(project_labels(&f), ["alpha", "beta"]);
        f.key("\x1b[1;2B");
        assert_eq!(project_labels(&f), ["beta", "alpha"]);
    }

    #[test]
    fn folders_reorder() {
        let mut f = fixture();
        f.app.tree.folders = ["x", "y"]
            .iter()
            .map(|n| FolderPrefs {
                id: (*n).into(),
                name: (*n).into(),
                projects: vec![],
            })
            .collect();
        f.app.rebuild_rows();
        f.app
            .select_where(|r| matches!(r, TreeRow::Folder { name, .. } if name == "x"));
        f.key("J");
        let names: Vec<_> = f.app.tree.folders.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["y", "x"]);
    }

    #[test]
    fn sessions_reorder_within_their_project_when_sessions_are_not_recency_sorted() {
        let mut f = fixture();
        f.app.config.ui.recent_sessions_first = false;
        let first = f.add(Some("s1"), true);
        f.add(Some("s2"), true);
        let before = session_uids(&f);
        assert_eq!(before.len(), 2);
        f.select(before[0]);
        f.key("J");
        assert_eq!(session_uids(&f), [before[1], before[0]]);
        assert!(before.contains(&first));
    }

    #[test]
    fn the_active_view_refuses_to_reorder() {
        let mut f = two_projects();
        f.key("t");
        f.key("J");
        assert_eq!(f.app.message, "Switch to the normal view (t) to reorder.");
        assert_eq!(project_labels(&f), ["alpha", "beta"]);
    }
}
