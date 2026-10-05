//! What the preview panel shows and its own scroll. Port of `App.groupPreview`, `syncLivePreview`,
//! `lastResponseOf`, `previewMaxScroll`, `scrollPreview`, `onArrow` and `onPageKey`.

use sdeck_core::discovery::claude_storage::read_last_assistant_response;
use sdeck_core::discovery::copilot_storage::last_copilot_assistant_response;

use super::App;
use crate::event::AppEvent;
use crate::sessions::LastResponse;
use crate::tree::{node_id_of, TreeRow};
use crate::view::preview_panel::{GroupPreview, PreviewContent};
use crate::view::GroupCounts;

impl App {
    /// Keeps the preview's state on the selected session: its scroll resets when the selection moves,
    /// the transcript tailer follows a session open elsewhere, and a stopped session's last reply
    /// starts loading the first time it's previewed.
    pub(super) fn sync_preview(&mut self) {
        let current = self.selected_session().map(|s| s.uid);
        if current != self.scrolled_session {
            self.scrolled_session = current;
            self.preview_scroll = 0;
            self.selection = None;
        }
        self.sync_live_preview(current);
        if let Some(uid) = current {
            self.load_last_response(uid);
        }
    }

    /// Attaches the tailer to the previewed session if it's open elsewhere (only that one), re-synced
    /// whenever the selection changes.
    fn sync_live_preview(&mut self, current: Option<u64>) {
        if current == self.live_preview_for {
            return;
        }
        self.live_preview_for = current;
        self.live_preview_turns.clear();
        let target = current
            .and_then(|uid| self.session_by_uid(uid))
            .filter(|s| self.procs.is_elsewhere(s))
            .and_then(|s| s.file.clone().map(|f| (s.uid, f)));
        match target {
            Some((uid, file)) => {
                let tx = self.tx.clone();
                self.live_preview_tailer.attach(&file, move |turns| {
                    let _ = tx.send(AppEvent::LiveTurns(uid, turns));
                });
            }
            None => self.live_preview_tailer.detach(),
        }
    }

    fn load_last_response(&mut self, uid: u64) {
        let tx = self.tx.clone();
        let Some(s) = self.session_mut(uid).filter(|s| s.last_response.is_none()) else {
            return;
        };
        if s.agent == "copilot" {
            if let Some(id) = &s.id {
                let text = last_copilot_assistant_response(id).unwrap_or_default();
                s.last_response = Some(LastResponse::Ready(text));
            }
        } else if let Some(file) = s.file.clone() {
            s.last_response = Some(LastResponse::Loading);
            std::thread::spawn(move || {
                let text = read_last_assistant_response(&file).unwrap_or_default();
                let _ = tx.send(AppEvent::LastResponse(uid, text));
            });
        }
    }

    pub(super) fn on_last_response(&mut self, uid: u64, text: String) {
        if let Some(s) = self.session_mut(uid) {
            if s.last_response == Some(LastResponse::Loading) {
                s.last_response = Some(LastResponse::Ready(text));
                self.dirty = true;
            }
        }
    }

    pub(super) fn on_live_turns(
        &mut self,
        uid: u64,
        turns: Vec<sdeck_core::status::claude_transcript_tailer::TailedTurn>,
    ) {
        if self.live_preview_for == Some(uid) {
            self.live_preview_turns = turns;
            self.dirty = true;
        }
    }

    /// Summary shown while a folder or project row is selected.
    pub(super) fn group_preview(&self) -> Option<GroupPreview> {
        let now = sdeck_core::format::now_ms();
        let split = !self.config.accounts.share_projects;
        let (is_folder, name, nodes, counts, detail) = match self.selected_row()? {
            TreeRow::Project {
                node_id,
                label,
                root,
                count,
                running,
                waiting,
                ..
            } => (
                false,
                label.clone(),
                vec![node_id.clone()],
                (*count, *running, *waiting),
                Self::short_path(root),
            ),
            TreeRow::Folder {
                folder_id,
                name,
                count,
                running,
                waiting,
                ..
            } => {
                let keys = self.containers.get(folder_id).cloned().unwrap_or_default();
                let detail = format!("{} project{}", keys.len(), if keys.len() == 1 { "" } else { "s" });
                (true, name.clone(), keys, (*count, *running, *waiting), detail)
            }
            _ => return None,
        };
        // From the sessions themselves, not the rows: a collapsed group has no session rows.
        let mut sessions: Vec<_> = self
            .sessions
            .iter()
            .filter(|s| {
                let node = if is_folder {
                    s.project_key.clone()
                } else {
                    node_id_of(s, split)
                };
                nodes.contains(&node) && self.is_visible(s, now)
            })
            .collect();
        sessions.sort_by_key(|s| std::cmp::Reverse(s.mtime_ms));
        Some(GroupPreview {
            is_folder,
            name,
            detail,
            counts: GroupCounts {
                count: counts.0,
                running: counts.1,
                waiting: counts.2,
            },
            sessions: sessions.into_iter().map(|s| self.view_of(s)).collect(),
        })
    }

    /// What the preview shows for the selected session, if one is selected.
    pub(super) fn preview_content(&self) -> Option<PreviewContent<'_>> {
        let s = self.selected_session()?;
        let live = s.live.as_ref().filter(|l| !l.exited);
        Some(PreviewContent {
            view: self.view_of(s),
            screen: live.map(|l| l.screen_at(self.preview_scroll)),
            interacting: self.interacting == Some(s.uid),
            exit_code: s.live.as_ref().and_then(|l| l.exit_code),
            last_response: s.last_response.as_ref(),
            live_turns: self
                .procs
                .is_elsewhere(s)
                .then_some(self.live_preview_turns.as_slice()),
            scroll_offset: self.preview_scroll,
            selection: self.selection_range(),
        })
    }

    /// How far back the preview can scroll: what the selected live session's screen holds in
    /// scrollback, 0 with nothing live.
    pub(super) fn preview_max_scroll(&self) -> usize {
        self.selected_session()
            .and_then(|s| s.live.as_ref())
            .filter(|l| !l.exited)
            .map_or(0, |l| l.max_scroll())
    }

    /// `delta` lines further back (positive) or toward the live bottom (negative), clamped.
    pub(super) fn scroll_preview(&mut self, delta: isize) {
        let max = self.preview_max_scroll();
        self.preview_scroll = self.preview_scroll.saturating_add_signed(delta).min(max);
    }

    /// ↑/↓: scrolls the selected session's preview while it can go that way, else moves the
    /// selection (so ↓ at the live bottom of a preview with scrollback still leaves the session).
    pub(super) fn on_arrow(&mut self, delta: isize) {
        let max = self.preview_max_scroll();
        let can_scroll = if delta < 0 {
            self.preview_scroll < max
        } else {
            self.preview_scroll > 0
        };
        if self.selected_session().is_some() && can_scroll {
            self.scroll_preview(-delta);
        } else {
            self.move_selection(delta);
        }
    }

    /// PageUp/PageDown/Home/End: a preview page, or all the way to the top/bottom of scrollback.
    /// Not handled (`false`) when there's nothing to scroll.
    pub(super) fn on_page_key(&mut self, key: &str) -> bool {
        let max = self.preview_max_scroll();
        if self.selected_session().is_none() || max == 0 {
            return false;
        }
        let page = self.pty_size().map_or(10, |(_, rows)| rows.max(1) as isize);
        let max = max as isize;
        match key {
            "\x1b[5~" => self.scroll_preview(page),
            "\x1b[6~" => self.scroll_preview(-page),
            "\x1b[H" | "\x1b[1~" => self.scroll_preview(max),
            "\x1b[F" | "\x1b[4~" => self.scroll_preview(-max),
            _ => return false,
        }
        true
    }
}
