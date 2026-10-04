//! Delete (`d`), undo (`Ctrl+Z`) and the trash picker (`Z`). Port of `deleteSession`,
//! `undoDelete`, `restoreFromTrash` and `openTrashPicker`.

use std::time::Instant;

use sdeck_core::discovery::claude_storage::clear_session_meta_cache;
use sdeck_core::format::{humanize_since, now_ms};
use sdeck_core::status::session_status::clear_session_status;
use sdeck_core::store::trash::{list_trash, restore_session, trash_claude_session};

use super::input::Picker;
use super::input::PickerAction;
use super::App;
use crate::sessions::display_title;
use crate::tree::TreeRow;

impl App {
    /// `d`: the checked batch, else the selected session or project.
    pub(super) fn delete_selected(&mut self, now: Instant) {
        if !self.multi_selected.is_empty() {
            self.bulk_delete(now);
            return;
        }
        match self.selected_row() {
            Some(TreeRow::Session { uid, .. }) => {
                let uid = *uid;
                self.delete_session(uid, now);
            }
            Some(TreeRow::Project {
                project_key, label, ..
            }) => {
                let (key, label) = (project_key.clone(), label.clone());
                self.remove_project(key, &label);
            }
            _ => {}
        }
    }

    /// Why the session can't be moved to the trash, if it can't: only a stopped Claude session
    /// with a transcript on disk can.
    pub(super) fn undeletable_reason(&self, uid: u64) -> Option<&'static str> {
        let s = self.session_by_uid(uid)?;
        if s.agent == "copilot" {
            Some("Copilot sessions live in its own database: archive them (A) instead.")
        } else if s.is_live() || self.procs.is_elsewhere(s) {
            Some("Stop the session before deleting it.")
        } else if s.id.is_none() || !s.file.as_ref().is_some_and(|f| f.exists()) {
            Some("Nothing on disk to delete.")
        } else {
            None
        }
    }

    /// Moves a deletable session's transcript to the trash and drops it from the list.
    pub(super) fn move_to_trash(&mut self, uid: u64) -> Result<(), String> {
        let Some(s) = self.session_by_uid(uid) else {
            return Ok(());
        };
        let (Some(id), Some(file)) = (s.id.clone(), s.file.clone()) else {
            return Ok(());
        };
        let title = display_title(s, &self.store);
        trash_claude_session(&file, &id, &title).map_err(|e| e.to_string())?;
        clear_session_status(&id);
        self.deleted.push(id);
        if let Some(mut live) = self.session_mut(uid).and_then(|s| s.live.take()) {
            live.dispose();
        }
        self.sessions.retain(|s| s.uid != uid);
        Ok(())
    }

    fn delete_session(&mut self, uid: u64, now: Instant) {
        if let Some(reason) = self.undeletable_reason(uid) {
            self.flash(reason.into(), now);
            return;
        }
        if let Err(err) = self.move_to_trash(uid) {
            self.flash(format!("Could not move it to the trash: {err}"), now);
            return;
        }
        self.rebuild_rows();
        self.flash(
            "Moved to the trash · Ctrl+Z to undo · Z to see the trash".into(),
            now,
        );
    }

    /// `Ctrl+Z`: restores the most recently deleted session; repeatable.
    pub(super) fn undo_delete(&mut self, now: Instant) {
        match self.deleted.pop() {
            Some(id) => self.restore_from_trash(&id, now),
            None => self.flash("Nothing to undo. Z shows the trash.".into(), now),
        }
    }

    pub(super) fn restore_from_trash(&mut self, id: &str, now: Instant) {
        match restore_session(id) {
            Ok(entry) => {
                self.deleted.retain(|d| d != id);
                clear_session_meta_cache();
                self.select_after_discovery = Some(id.to_string());
                self.spawn_discovery();
                self.flash(format!("Restored \"{}\"", entry.title), now);
            }
            Err(err) => self.flash(err.to_string(), now),
        }
    }

    /// `Z`: the trash, newest first; Enter restores.
    pub(super) fn open_trash_picker(&mut self, now: Instant) {
        let entries = list_trash();
        if entries.is_empty() {
            self.flash("The trash is empty.".into(), now);
            return;
        }
        self.picker = Some(Picker {
            title: "Trash · Enter restores".into(),
            items: entries
                .iter()
                .map(|e| format!("{} · {}", e.title, humanize_since(e.trashed_at, now_ms())))
                .collect(),
            index: 0,
            action: PickerAction::RestoreFromTrash(entries.into_iter().map(|e| e.session_id).collect()),
        });
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::app::test_fixture::{fixture, Fixture};
    use crate::tree::TreeRow;

    fn add_on_disk(f: &mut Fixture, id: &str) -> u64 {
        let uid = f.add(Some(id), true);
        let transcript = f.home.path().join(format!("{id}.jsonl"));
        fs::write(&transcript, "{}\n").unwrap();
        f.app.session_mut(uid).unwrap().file = Some(transcript);
        uid
    }

    fn listed(f: &Fixture) -> Vec<String> {
        f.app.sessions.iter().filter_map(|s| s.id.clone()).collect()
    }

    #[test]
    fn delete_moves_the_transcript_to_the_trash_and_off_the_list() {
        let mut f = fixture();
        let uid = add_on_disk(&mut f, "abc");
        let transcript = f.session(uid).unwrap().file.clone().unwrap();
        f.key("d");
        assert!(f.session(uid).is_none());
        assert!(!transcript.exists());
        assert_eq!(
            f.app.message,
            "Moved to the trash · Ctrl+Z to undo · Z to see the trash"
        );
        assert_eq!(super::list_trash().len(), 1);
        assert!(!f.app.rows.iter().any(|r| matches!(r, TreeRow::Session { .. })));
    }

    #[test]
    fn delete_refuses_copilot_running_and_missing_transcripts() {
        let mut f = fixture();
        let missing = f.add(Some("nofile"), true);
        f.key("d");
        assert_eq!(f.app.message, "Nothing on disk to delete.");
        assert!(f.session(missing).is_some());

        let running = add_on_disk(&mut f, "run");
        f.key("s");
        f.wait_for_screen(running, "> ");
        f.key("d");
        assert_eq!(f.app.message, "Stop the session before deleting it.");
        f.app.kill(running);

        let copilot = add_on_disk(&mut f, "cop");
        f.app.session_mut(copilot).unwrap().agent = "copilot".into();
        f.key("d");
        assert_eq!(
            f.app.message,
            "Copilot sessions live in its own database: archive them (A) instead."
        );
        assert_eq!(super::list_trash().len(), 0);
    }

    #[test]
    fn ctrl_z_twice_restores_two_in_reverse_order() {
        let mut f = fixture();
        let first = add_on_disk(&mut f, "one");
        let first_file = f.session(first).unwrap().file.clone().unwrap();
        f.key("d");
        let second = add_on_disk(&mut f, "two");
        let second_file = f.session(second).unwrap().file.clone().unwrap();
        f.key("d");
        assert_eq!(f.app.deleted, ["one", "two"]);
        assert!(listed(&f).is_empty());

        f.key("\x1a");
        assert!(second_file.exists() && !first_file.exists());
        assert_eq!(f.app.message, "Restored \"a session\"");
        assert_eq!(f.app.select_after_discovery.as_deref(), Some("two"));
        f.key("\x1a");
        assert!(first_file.exists());
        assert!(f.app.deleted.is_empty());
        assert_eq!(super::list_trash().len(), 0);

        f.key("\x1a");
        assert_eq!(f.app.message, "Nothing to undo. Z shows the trash.");
    }

    #[test]
    fn z_lists_the_trash_and_enter_restores_the_chosen_entry() {
        let mut f = fixture();
        f.key("Z");
        assert_eq!(f.app.message, "The trash is empty.");

        let uid = add_on_disk(&mut f, "abc");
        let transcript = f.session(uid).unwrap().file.clone().unwrap();
        f.key("d");
        f.key("Z");
        let picker = f.app.picker.as_ref().unwrap();
        assert_eq!(picker.title, "Trash · Enter restores");
        assert_eq!(picker.items.len(), 1);
        assert!(picker.items[0].starts_with("a session · "));
        f.key("\r");
        assert!(f.app.picker.is_none());
        assert!(transcript.exists());
    }
}
