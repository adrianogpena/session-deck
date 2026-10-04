//! Multi-select (`Space`) and the batch forms of `x`, `A` and `d`. Port of `toggleMultiSelect`,
//! `bulkStop`, `bulkArchive` and `bulkDelete`.

use std::time::Instant;

use super::App;
use crate::tree::TreeRow;

impl App {
    /// The checked sessions' uids, in list order.
    pub(super) fn multi_selection(&self) -> Vec<u64> {
        self.sessions
            .iter()
            .map(|s| s.uid)
            .filter(|uid| self.multi_selected.contains(uid))
            .collect()
    }

    /// `Space`: checks or unchecks the selected session, then moves down like a multi-select list.
    pub(super) fn toggle_multi_select(&mut self) {
        let Some(TreeRow::Session { uid, .. }) = self.selected_row() else {
            return;
        };
        let uid = *uid;
        if !self.multi_selected.remove(&uid) {
            self.multi_selected.insert(uid);
        }
        self.move_selection(1);
    }

    /// `Esc`: drops the checked batch.
    pub(super) fn clear_multi_select(&mut self, now: Instant) {
        if !self.multi_selected.is_empty() {
            self.multi_selected.clear();
            self.flash("Selection cleared".into(), now);
        }
    }

    pub(super) fn bulk_stop(&mut self, now: Instant) {
        let targets = self.multi_selection();
        let mut stopped = 0;
        for &uid in &targets {
            if self.session_by_uid(uid).is_some_and(|s| s.live.is_some()) {
                self.kill(uid);
                stopped += 1;
            }
        }
        self.multi_selected.clear();
        let text = match (stopped, targets.len() - stopped) {
            (0, _) => "None of the selected sessions were running".to_string(),
            (n, 0) => format!("Stopped {n}"),
            (n, rest) => format!("Stopped {n} · {rest} not running"),
        };
        self.flash(text, now);
    }

    /// `A` on a batch: archives every one, or unarchives them in the archived view. Working
    /// sessions are skipped; idle live ones are stopped first.
    pub(super) fn bulk_archive(&mut self, now: Instant) {
        let archive = !self.archived_view;
        let mut ids = Vec::new();
        let mut skipped = 0;
        for uid in self.multi_selection() {
            let Some(s) = self.session_by_uid(uid) else {
                continue;
            };
            let Some(id) = s.id.clone() else {
                skipped += 1;
                continue;
            };
            if archive {
                if self.is_working(s) {
                    skipped += 1;
                    continue;
                }
                if s.is_live() {
                    self.kill(uid);
                }
            }
            ids.push(id);
        }
        self.multi_selected.clear();
        if ids.is_empty() {
            let text = if skipped > 0 {
                format!("Nothing archived — {skipped} still working")
            } else {
                "Nothing to archive".into()
            };
            self.flash(text, now);
            return;
        }
        self.set_archived(&ids, archive);
        let skipped_note = if skipped > 0 {
            format!(" · {skipped} skipped (still working)")
        } else {
            String::new()
        };
        self.flash(
            format!(
                "{} {}{skipped_note}",
                if archive { "Archived" } else { "Unarchived" },
                ids.len()
            ),
            now,
        );
    }

    /// `d` on a batch: moves every deletable one to the trash; Copilot, running and elsewhere
    /// sessions are skipped.
    pub(super) fn bulk_delete(&mut self, now: Instant) {
        let (mut deleted, mut skipped) = (0, 0);
        for uid in self.multi_selection() {
            if self.undeletable_reason(uid).is_some() || self.move_to_trash(uid).is_err() {
                skipped += 1;
            } else {
                deleted += 1;
            }
        }
        self.multi_selected.clear();
        self.rebuild_rows();
        let text = match (deleted, skipped) {
            (0, 0) => "Nothing deletable in the selection".to_string(),
            (0, n) => format!("Nothing deletable in the selection ({n} skipped)"),
            (d, 0) => format!("Moved {d} to the trash · Ctrl+Z to undo"),
            (d, n) => format!("Moved {d} to the trash · {n} skipped · Ctrl+Z to undo"),
        };
        self.flash(text, now);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::app::test_fixture::{fixture, Fixture};

    fn add_on_disk(f: &mut Fixture, id: &str) -> u64 {
        let uid = f.add(Some(id), true);
        let transcript = f.home.path().join(format!("{id}.jsonl"));
        fs::write(&transcript, "{}\n").unwrap();
        f.app.session_mut(uid).unwrap().file = Some(transcript);
        uid
    }

    fn check(f: &mut Fixture, uids: &[u64]) {
        for &uid in uids {
            f.select(uid);
            f.key(" ");
        }
    }

    #[test]
    fn space_checks_moves_down_and_escape_clears() {
        let mut f = fixture();
        let a = f.add(Some("a"), true);
        let b = f.add(Some("b"), true);
        f.select(a);
        f.key(" ");
        assert!(f.app.multi_selected.contains(&a));
        assert_eq!(f.app.selected_session().map(|s| s.uid), Some(b));
        f.key("k");
        f.key(" ");
        assert!(f.app.multi_selected.is_empty());
        check(&mut f, &[a, b]);
        assert_eq!(f.app.multi_selected.len(), 2);
        f.key("\x1b");
        assert!(f.app.multi_selected.is_empty());
        assert_eq!(f.app.message, "Selection cleared");
    }

    #[test]
    fn bulk_delete_counts_deleted_and_skipped() {
        let mut f = fixture();
        let stopped = add_on_disk(&mut f, "stopped");
        let running = add_on_disk(&mut f, "running");
        f.key("s");
        f.wait_for_screen(running, "> ");
        let copilot = add_on_disk(&mut f, "cop");
        f.app.session_mut(copilot).unwrap().agent = "copilot".into();
        check(&mut f, &[stopped, running, copilot]);
        f.key("d");
        assert_eq!(f.app.message, "Moved 1 to the trash · 2 skipped · Ctrl+Z to undo");
        assert!(f.session(stopped).is_none());
        assert!(f.session(running).is_some() && f.session(copilot).is_some());
        assert!(f.app.multi_selected.is_empty());
        f.app.kill(running);
    }

    #[test]
    fn bulk_delete_with_nothing_deletable() {
        let mut f = fixture();
        let a = f.add(Some("a"), true);
        check(&mut f, &[a]);
        f.key("d");
        assert_eq!(f.app.message, "Nothing deletable in the selection (1 skipped)");
    }

    #[test]
    fn bulk_stop_counts_stopped_and_not_running() {
        let mut f = fixture();
        let running = f.add(Some("running"), true);
        f.key("s");
        f.wait_for_screen(running, "> ");
        let stopped = f.add(Some("stopped"), true);
        check(&mut f, &[running, stopped]);
        f.key("x");
        assert_eq!(f.app.message, "Stopped 1 · 1 not running");
        assert!(!f.session(running).unwrap().is_live());

        check(&mut f, &[stopped]);
        f.key("x");
        assert_eq!(f.app.message, "None of the selected sessions were running");
    }

    #[test]
    fn bulk_archive_skips_working_sessions_and_unarchives_in_the_archived_view() {
        let mut f = fixture();
        let busy = f.add(Some("busy"), true);
        f.key("s");
        f.wait_for_screen(busy, "> ");
        f.set_process_status(busy, "busy");
        let idle = f.add(Some("idle"), true);
        let plain = f.add(Some("plain"), true);
        check(&mut f, &[busy, idle, plain]);
        f.key("A");
        assert_eq!(f.app.message, "Archived 2 · 1 skipped (still working)");
        let archived = |f: &Fixture, id: &str| f.app.store.get_session(id).is_some_and(|p| p.archived);
        assert!(archived(&f, "idle") && archived(&f, "plain") && !archived(&f, "busy"));

        check(&mut f, &[busy]);
        f.key("A");
        assert_eq!(f.app.message, "Nothing archived — 1 still working");

        f.key("^");
        let idle_uid = idle;
        check(&mut f, &[idle_uid]);
        f.key("A");
        assert_eq!(f.app.message, "Unarchived 1");
        assert!(!archived(&f, "idle"));
        f.app.kill(busy);
    }

    #[test]
    fn the_list_header_note_counts_the_checked_sessions() {
        let mut f = fixture();
        let a = f.add(Some("a"), true);
        check(&mut f, &[a]);
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 12)).unwrap();
        f.app.set_size(100, 12);
        terminal.draw(|frame| f.app.draw(frame)).unwrap();
        let buf = terminal.backend().buffer();
        let row: String = (0..100).map(|x| buf[(x, 2)].symbol()).collect();
        assert!(row.contains("· 1 selected"), "{row}");
    }
}
