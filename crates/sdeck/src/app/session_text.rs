//! Copy the last response (`c`), rename (`e`/`F2`) and clear context (`Ctrl+L`). Port of
//! `App.copyLastResponse`, `rename`, `renameSession` and `clearContext`.

use std::time::Instant;

use sdeck_core::discovery::claude_storage::{append_claude_rename_records, read_last_assistant_response};
use sdeck_core::discovery::copilot_storage::last_copilot_assistant_response;
use sdeck_core::store::deck_store::{Patch, SessionPatch};
use sdeck_core::store::tree_prefs;

use super::input::PromptAction;
use super::App;
use crate::ansi::one_line;
use crate::sessions::{display_title, SessionStatus};
use crate::tree::TreeRow;

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, b)| acc | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(BASE64[(n >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

impl App {
    /// Copies through the terminal (OSC 52), which works in Windows Terminal and over SSH.
    pub(super) fn copy_last_response(&mut self, now: Instant) {
        let Some(s) = self.selected_session() else {
            self.flash("Select a session to copy its last response.".into(), now);
            return;
        };
        let text = if s.agent == "copilot" {
            s.id.as_deref().and_then(last_copilot_assistant_response)
        } else {
            s.file.as_deref().and_then(read_last_assistant_response)
        };
        match text.filter(|t| !t.is_empty()) {
            Some(text) => {
                self.emit(&format!("\x1b]52;c;{}\x07", base64(text.as_bytes())));
                let count = text.chars().count();
                self.flash(format!("Copied the last response ({count} characters)"), now);
            }
            None => self.flash("No response to copy yet.".into(), now),
        }
    }

    /// `e`/`F2`: rename the selected session or folder.
    pub(super) fn rename(&mut self, now: Instant) {
        match self.selected_row() {
            Some(TreeRow::Session { uid, .. }) => {
                let uid = *uid;
                let Some(s) = self.session_by_uid(uid) else {
                    return;
                };
                let title = display_title(s, &self.store);
                let value = if title == "(new session)" {
                    String::new()
                } else {
                    title
                };
                self.open_prompt("Rename".into(), value, PromptAction::RenameSession(uid));
            }
            Some(TreeRow::Folder { folder_id, name, .. }) => {
                let (id, name) = (folder_id.clone(), name.clone());
                self.open_prompt("Rename folder".into(), name, PromptAction::RenameFolder(id));
            }
            Some(TreeRow::Project { .. }) => {
                self.flash("Projects are named after their folder on disk.".into(), now);
            }
            _ => {}
        }
    }

    pub(super) fn rename_folder(&mut self, folder_id: &str, value: &str) {
        if !value.trim().is_empty() {
            self.change_tree(|t| tree_prefs::rename_folder(&t, folder_id, value));
        }
    }

    /// Sets the same title Claude's own `/rename` does. A running Claude keeps its title in memory
    /// and re-writes it into the transcript, so a live session is renamed through Claude itself; a
    /// stopped one gets the records appended that `/rename` would have written. Any Session Deck
    /// name override is cleared, so the new title shows in the extension too.
    pub(super) fn rename_session(&mut self, uid: u64, raw_name: &str, now: Instant) {
        let name: String = one_line(raw_name).chars().take(100).collect();
        let Some(s) = self.session_by_uid(uid) else {
            return;
        };
        if name.is_empty() || name == display_title(s, &self.store) {
            return;
        }
        let id = s.id.clone();
        if s.agent != "claude" {
            // No /rename: a Session Deck name, shared with the extension.
            if let Some(id) = id {
                let patch = SessionPatch {
                    name: Patch::Set(name.clone()),
                    ..SessionPatch::default()
                };
                let _ = self.store.update_session(&id, &patch);
                self.dirty = true;
                self.flash(format!("Renamed to \"{name}\""), now);
            }
            return;
        }
        if self.procs.is_elsewhere(s) {
            self.flash(
                "That session is running in another terminal. Rename it there with /rename.".into(),
                now,
            );
            return;
        }
        let file = s.file.clone();
        if s.is_live() {
            if !matches!(self.procs.status_of(s), SessionStatus::Idle | SessionStatus::Done) {
                self.flash(
                    "Session is busy or waiting for you. Rename it once it is idle (○).".into(),
                    now,
                );
                return;
            }
            if let Some(live) = self.session_mut(uid).and_then(|s| s.live.as_mut()) {
                live.type_line(&format!("/rename {name}"), now);
            }
        } else if let (Some(id), Some(file)) = (&id, &file) {
            if let Err(e) = append_claude_rename_records(file, id, &name) {
                self.flash(format!("Could not rename: {e}"), now);
                return;
            }
        } else {
            self.flash("Nothing to rename yet.".into(), now);
            return;
        }
        if let Some(s) = self.session_mut(uid) {
            s.title = name.clone();
        }
        if id.as_deref().is_some_and(|id| {
            self.store
                .get_session(id)
                .is_some_and(|p| p.name.is_some_and(|n| !n.is_empty()))
        }) {
            let patch = SessionPatch {
                name: Patch::Clear,
                ..SessionPatch::default()
            };
            let _ = self
                .store
                .update_session(id.as_deref().unwrap_or_default(), &patch);
        }
        self.dirty = true;
        self.flash(format!("Renamed to \"{name}\""), now);
    }

    /// Ctrl+L: sends `/clear` into the live session, without attaching. Only when there's nothing
    /// to lose by it — idle, done (an unseen reply), or error (e.g. a stuck sign-in screen) — never
    /// mid-turn or mid-prompt.
    pub(super) fn clear_context(&mut self, now: Instant) {
        let Some(s) = self.selected_session() else {
            self.flash("Select a session to clear.".into(), now);
            return;
        };
        let uid = s.uid;
        if self.procs.is_elsewhere(s) {
            self.flash(
                "That session is running in another terminal. Clear it there with /clear.".into(),
                now,
            );
        } else if !s.is_live() {
            self.flash("Select a running session to clear.".into(), now);
        } else if !matches!(
            self.procs.status_of(s),
            SessionStatus::Idle | SessionStatus::Done | SessionStatus::Error
        ) {
            self.flash(
                "Session is busy or waiting for you. Clear once it is idle (○), done, or errored.".into(),
                now,
            );
        } else {
            if let Some(live) = self.session_mut(uid).and_then(|s| s.live.as_mut()) {
                live.type_line("/clear", now);
            }
            if self.config.ui.new_session_full_screen {
                self.attach(uid, now);
            } else {
                self.start_interacting(uid, now);
                self.flash("Cleared".into(), now);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::base64;
    use crate::app::test_fixture::fixture;
    use crate::event::AppEvent;
    use sdeck_core::discovery::claude_storage::find_session_title;
    use std::time::{Duration, Instant};

    #[test]
    fn base64_matches_the_standard_alphabet_with_padding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64("héllo".as_bytes()), "aMOpbGxv");
    }

    fn write_transcript(
        f: &mut crate::app::test_fixture::Fixture,
        uid: u64,
        lines: &[&str],
    ) -> std::path::PathBuf {
        let file = f.home.path().join("t.jsonl");
        std::fs::write(&file, lines.join("\n") + "\n").unwrap();
        f.app.session_mut(uid).unwrap().file = Some(file.clone());
        file
    }

    #[test]
    fn c_emits_the_last_response_as_an_osc_52_copy() {
        let mut f = fixture();
        let uid = f.add(Some("abc"), true);
        write_transcript(
            &mut f,
            uid,
            &[
                r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"first"}]}}"#,
                r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"héllo"}]}}"#,
            ],
        );
        f.key("c");
        assert_eq!(f.app.pending_output, b"\x1b]52;c;aMOpbGxv\x07");
        assert_eq!(f.app.message, "Copied the last response (5 characters)");

        f.app.pending_output.clear();
        f.app.session_mut(uid).unwrap().file = None;
        f.key("c");
        assert!(f.app.pending_output.is_empty());
        assert_eq!(f.app.message, "No response to copy yet.");
    }

    #[test]
    fn renaming_a_stopped_session_writes_the_title_claude_reads() {
        let mut f = fixture();
        let uid = f.add(Some("abc"), true);
        let file = write_transcript(
            &mut f,
            uid,
            &[r#"{"type":"user","message":{"role":"user","content":"hello"}}"#],
        );
        f.key("e");
        assert_eq!(f.app.prompt.as_ref().unwrap().value, "a session");
        f.key("\x15New  name\r");
        assert_eq!(f.session(uid).unwrap().title, "New name");
        assert_eq!(f.app.message, "Renamed to \"New name\"");
        let text = std::fs::read_to_string(file).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(find_session_title(&lines).as_deref(), Some("New name"));
    }

    #[test]
    fn renaming_a_running_idle_session_types_slash_rename_and_a_busy_one_waits() {
        let mut f = fixture();
        let uid = f.add(Some("abc"), true);
        f.key("s");
        f.wait_for_screen(uid, "> ");
        f.set_process_status(uid, "busy");
        f.key("e\x15x\r");
        assert_eq!(
            f.app.message,
            "Session is busy or waiting for you. Rename it once it is idle (○)."
        );
        f.set_process_status(uid, "idle");
        f.key("e\x15x\r");
        f.app
            .handle(AppEvent::Tick, Instant::now() + Duration::from_millis(300));
        f.wait_for_screen(uid, "echo: /rename x");
        assert_eq!(f.session(uid).unwrap().title, "x");
    }

    #[test]
    fn a_folder_renames_and_a_project_does_not() {
        use crate::tree::TreeRow;
        let mut f = fixture();
        f.add(Some("abc"), true);
        f.app
            .change_tree(|t| sdeck_core::store::tree_prefs::create_folder(&t, "Work").0);
        f.app.rebuild_rows();
        f.app.select_where(|r| matches!(r, TreeRow::Folder { .. }));
        f.key("\x1bOQ");
        assert_eq!(f.app.prompt.as_ref().unwrap().label, "Rename folder");
        f.key("\x15Play\r");
        assert!(matches!(f.app.selected_row(), Some(TreeRow::Folder { name, .. }) if name == "Play"));
        f.app.select_where(|r| matches!(r, TreeRow::Project { .. }));
        f.key("e");
        assert_eq!(f.app.message, "Projects are named after their folder on disk.");
    }

    #[test]
    fn ctrl_l_types_clear_only_into_an_idle_running_session() {
        let mut f = fixture();
        let uid = f.add(Some("abc"), true);
        f.key("\x0c");
        assert_eq!(f.app.message, "Select a running session to clear.");
        f.key("s");
        f.wait_for_screen(uid, "> ");
        f.set_process_status(uid, "busy");
        f.key("\x0c");
        assert!(f.app.message.starts_with("Session is busy or waiting"));
        f.app.config.ui.new_session_full_screen = false;
        f.set_process_status(uid, "idle");
        f.key("\x0c");
        assert_eq!(f.app.interacting, Some(uid));
        f.app
            .handle(AppEvent::Tick, Instant::now() + Duration::from_millis(300));
        f.wait_for_screen(uid, "echo: /clear");
    }
}
