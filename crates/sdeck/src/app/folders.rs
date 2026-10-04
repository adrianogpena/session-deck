//! Folders: `g` creates one, `M` moves projects into one (or back to the top level). Rename
//! (`e`) lives with the other renames and delete (`d`) with the other deletes. Port of
//! `newFolder`, `openMovePicker` and `openBulkMovePicker`.

use std::time::Instant;

use sdeck_core::store::tree_prefs::{create_folder, move_project_to_folder, FolderPrefs};

use super::input::{Picker, PickerAction, PromptAction};
use super::App;
use crate::ansi::one_line;
use crate::tree::TreeRow;

impl App {
    /// `g`.
    pub(super) fn open_new_folder_prompt(&mut self) {
        self.open_prompt(
            "New folder".into(),
            String::new(),
            PromptAction::NewFolder(Vec::new()),
        );
    }

    /// Creates the folder, moving `project_keys` into it when given. The id is made once, so the
    /// local and on-disk trees agree.
    pub(super) fn new_folder(&mut self, raw_name: &str, project_keys: &[String], now: Instant) {
        let name = one_line(raw_name);
        if name.is_empty() {
            return;
        }
        let (tree, folder_id) = create_folder(&self.tree, &name);
        let folder: FolderPrefs = tree.folders.iter().find(|f| f.id == folder_id).cloned().unwrap();
        self.change_tree(|mut t| {
            t.folders.push(folder.clone());
            project_keys
                .iter()
                .fold(t, |acc, key| move_project_to_folder(&acc, key, Some(&folder_id)))
        });
        if project_keys.is_empty() {
            self.select_where(|r| matches!(r, TreeRow::Folder { folder_id: id, .. } if *id == folder_id));
            self.flash(
                format!("Folder \"{name}\" created. Press M on a project to move it in."),
                now,
            );
        } else {
            let moved = match project_keys.len() {
                1 => "project".to_string(),
                n => format!("{n} projects"),
            };
            self.flash(format!("Moved {moved} to \"{name}\""), now);
        }
    }

    /// `M`: the selected project (or the checked batch's projects) to a folder.
    pub(super) fn open_move_picker(&mut self, now: Instant) {
        let bulk = !self.multi_selected.is_empty();
        let (title, project_keys, index) = if bulk {
            let mut keys: Vec<String> = Vec::new();
            for uid in self.multi_selection() {
                if let Some(s) = self.session_by_uid(uid) {
                    if !keys.contains(&s.project_key) {
                        keys.push(s.project_key.clone());
                    }
                }
            }
            let title = format!(
                "Move {} project{} to",
                keys.len(),
                if keys.len() == 1 { "" } else { "s" }
            );
            (title, keys, 0)
        } else {
            let Some((key, root)) = self.selected_project() else {
                self.flash(
                    "Select a project (or one of its sessions) to move it to a folder.".into(),
                    now,
                );
                return;
            };
            let current = self.tree.folders.iter().position(|f| f.projects.contains(&key));
            let label = match root.rsplit(['\\', '/']).find(|p| !p.is_empty()) {
                Some(base) => base.to_string(),
                None => root,
            };
            (
                format!("Move {label} to"),
                vec![key],
                current.map_or(0, |i| i + 1),
            )
        };
        let folders = &self.tree.folders;
        let mut items = vec!["Top level (no folder)".to_string()];
        items.extend(folders.iter().map(|f| f.name.clone()));
        items.push("+ New folder…".into());
        self.picker = Some(Picker {
            title,
            items,
            index,
            action: PickerAction::MoveToFolder {
                project_keys,
                folder_ids: folders.iter().map(|f| f.id.clone()).collect(),
                bulk,
            },
        });
    }

    pub(super) fn move_to_folder(
        &mut self,
        index: usize,
        project_keys: Vec<String>,
        folder_ids: &[String],
        bulk: bool,
    ) {
        if bulk {
            self.multi_selected.clear();
        }
        if index > folder_ids.len() {
            self.open_prompt(
                "New folder".into(),
                String::new(),
                PromptAction::NewFolder(project_keys),
            );
            return;
        }
        let target = index.checked_sub(1).map(|i| folder_ids[i].clone());
        self.change_tree(|t| {
            project_keys
                .iter()
                .fold(t, |acc, key| move_project_to_folder(&acc, key, target.as_deref()))
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::app::test_fixture::{fixture, Fixture};
    use crate::tree::TreeRow;

    fn project_key(f: &Fixture) -> String {
        f.app
            .rows
            .iter()
            .find_map(|r| match r {
                TreeRow::Project { project_key, .. } => Some(project_key.clone()),
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn g_creates_a_folder_and_selects_it() {
        let mut f = fixture();
        f.add(Some("a"), true);
        f.key("g");
        assert_eq!(f.app.prompt.as_ref().unwrap().label, "New folder");
        f.key("Work  stuff\r");
        assert_eq!(f.app.tree.folders.len(), 1);
        assert_eq!(f.app.tree.folders[0].name, "Work stuff");
        assert!(matches!(f.app.selected_row(), Some(TreeRow::Folder { name, .. }) if name == "Work stuff"));
        assert_eq!(
            f.app.message,
            "Folder \"Work stuff\" created. Press M on a project to move it in."
        );
    }

    #[test]
    fn a_blank_folder_name_creates_nothing() {
        let mut f = fixture();
        f.key("g");
        f.key("   \r");
        assert!(f.app.tree.folders.is_empty());
    }

    #[test]
    fn m_moves_a_project_into_a_new_folder_and_deleting_the_folder_returns_it() {
        let mut f = fixture();
        f.add(Some("a"), true);
        let key = project_key(&f);
        f.key("M");
        let picker = f.app.picker.as_ref().unwrap();
        assert_eq!(picker.items, ["Top level (no folder)", "+ New folder…"]);
        f.key("j\r");
        assert_eq!(f.app.prompt.as_ref().unwrap().label, "New folder");
        f.key("Docs\r");
        assert_eq!(f.app.tree.folders[0].projects, std::slice::from_ref(&key));
        assert_eq!(f.app.message, "Moved project to \"Docs\"");
        assert!(f.app.rows.iter().any(|r| matches!(r, TreeRow::Folder { .. })));

        f.app.select_where(|r| matches!(r, TreeRow::Folder { .. }));
        f.key("d");
        assert_eq!(
            f.app.confirm.as_ref().unwrap().question,
            "Delete folder \"Docs\"? Its projects move back to the top level."
        );
        f.key("y");
        assert!(f.app.tree.folders.is_empty());
        assert_eq!(f.app.tree.root_order, [key]);
        assert!(!f.app.rows.iter().any(|r| matches!(r, TreeRow::Folder { .. })));
    }

    #[test]
    fn the_picker_opens_on_the_current_folder_and_top_level_moves_back() {
        let mut f = fixture();
        f.add(Some("a"), true);
        let key = project_key(&f);
        f.key("M");
        f.key("j\r");
        f.key("Docs\r");
        f.app.select_where(|r| matches!(r, TreeRow::Project { .. }));
        f.key("M");
        assert_eq!(f.app.picker.as_ref().unwrap().index, 1);
        f.key("k\r");
        assert!(f.app.tree.folders[0].projects.is_empty());
        assert!(f.app.tree.root_order.contains(&key));
    }

    #[test]
    fn m_with_a_checked_batch_moves_each_distinct_project_once_and_clears_the_checks() {
        let mut f = fixture();
        let a = f.add(Some("a"), true);
        let b = f.add(Some("b"), true);
        f.app.multi_selected.extend([a, b]);
        f.key("M");
        assert_eq!(f.app.picker.as_ref().unwrap().title, "Move 1 project to");
        f.key("j\r");
        f.key("Both\r");
        assert!(f.app.multi_selected.is_empty());
        assert_eq!(f.app.tree.folders[0].projects.len(), 1);
        assert_eq!(f.app.message, "Moved project to \"Both\"");
    }

    #[test]
    fn m_off_a_project_or_session_explains() {
        let mut f = fixture();
        f.key("M");
        assert_eq!(
            f.app.message,
            "Select a project (or one of its sessions) to move it to a folder."
        );
    }
}
