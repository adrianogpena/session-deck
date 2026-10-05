//! Interact mode (typing into a session while the list and preview keep rendering), sdeck's own
//! mouse scrolling, and the sessions panel toggle. Port of `App.startInteracting`,
//! `stopInteracting`, `toggleMouseTracking`, `onMouseSequence` and `toggleSidebar`.

use std::time::Instant;

use super::App;
use crate::keys::{parse_mouse_sequence, Wheel};

impl App {
    /// `i`: typing goes to the session's PTY, already sized to the preview. Mouse tracking stays as
    /// it is (unlike attach): the list is still up, so a wheel notch is still sdeck's to scroll with.
    pub(super) fn start_interacting(&mut self, uid: u64, now: Instant) {
        if !self.ensure_live(uid, now) {
            return;
        }
        self.interacting = Some(uid);
        self.mark_seen(uid);
        self.dirty = true;
    }

    pub(super) fn stop_interacting(&mut self) {
        self.interacting = None;
        self.dirty = true;
    }

    /// `m`, or Ctrl+K M while interacting: sdeck's mouse reporting, off by default (so click-drag
    /// selects text) and not persisted. On, the wheel scrolls the preview.
    pub(super) fn toggle_mouse_tracking(&mut self, now: Instant) {
        self.mouse_tracking = !self.mouse_tracking;
        self.mouse_capture_change = Some(self.mouse_tracking);
        let text = if self.mouse_tracking {
            "Mouse scrolling on — click-drag no longer selects text (m to turn off)"
        } else {
            "Mouse scrolling off — click-drag selects text again (m to turn on)"
        };
        self.flash(text.into(), now);
    }

    /// Ctrl+K B while interacting (and `b`, from 14): hides or shows the sessions panel.
    pub(super) fn toggle_sidebar(&mut self) {
        self.sidebar_visible = !self.sidebar_visible;
        self.clear_screen = true;
        self.dirty = true;
        self.resize_all_to_pane();
    }

    /// A wheel notch or click, reported while mouse tracking is on; `false` if `key` isn't one. A
    /// notch scrolls the preview's scrollback if there is any; agents that keep their own history
    /// (Claude Code) never produce any, so it sends them PageUp/PageDown instead. Clicks are
    /// consumed.
    pub(super) fn on_mouse_sequence(&mut self, key: &str) -> bool {
        let Some(mouse) = parse_mouse_sequence(key) else {
            return false;
        };
        let (Some(wheel), Some(uid)) = (mouse, self.selected_session().map(|s| s.uid)) else {
            return true;
        };
        if self.preview_max_scroll() > 0 {
            self.scroll_preview(if wheel == Wheel::Up { 1 } else { -1 });
        } else if let Some(live) = self.session_mut(uid).and_then(|s| s.live.as_mut()) {
            live.write(if wheel == Wheel::Up {
                b"\x1b[5~"
            } else {
                b"\x1b[6~"
            });
        }
        self.dirty = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use crate::app::test_fixture::fixture;

    #[test]
    fn interact_types_into_the_agent_while_the_list_renders_and_ctrl_q_leaves() {
        let mut f = fixture();
        let uid = f.add(Some("abc"), true);
        f.app.set_size(100, 14);
        f.key("i");
        assert_eq!(f.app.interacting, Some(uid));
        f.wait_for_screen(uid, "> ");
        f.key("hi there\r");
        f.wait_for_screen(uid, "echo: hi there");

        let mut terminal = Terminal::new(TestBackend::new(100, 14)).unwrap();
        terminal.draw(|frame| f.app.draw(frame)).unwrap();
        let buf = terminal.backend().buffer();
        let rows: Vec<String> = (0..14)
            .map(|y| (0..100).map(|x| buf[(x, y)].symbol()).collect())
            .collect();
        assert!(rows[2].starts_with("SESSIONS"), "{}", rows[2]);
        assert!(rows.iter().any(|r| r.contains("echo: hi there")), "{rows:#?}");
        assert!(rows[13].contains("Interacting · Ctrl+Q to stop"), "{}", rows[13]);

        f.key("\x11");
        assert_eq!(f.app.interacting, None);
        f.key("j");
        assert!(
            f.session(uid).unwrap().is_live(),
            "list keys no longer reach the agent"
        );
    }

    #[test]
    fn a_configured_detach_key_replaces_ctrl_q_in_the_banner_and_the_matcher() {
        let mut f = fixture();
        let uid = f.add(Some("abc"), true);
        f.app.set_size(100, 14);
        let index = crate::config_fields::CONFIG_FIELDS
            .iter()
            .position(|c| c.label == "keys.chord.detachKey")
            .unwrap();
        f.app.apply_config_field(index, "nope", std::time::Instant::now());
        assert_eq!(f.app.config.ui.detach_key, "ctrl+q");
        f.app
            .apply_config_field(index, "Ctrl+E", std::time::Instant::now());
        assert_eq!(f.app.config.ui.detach_key, "ctrl+e");

        f.app.message.clear();
        f.key("i");
        assert_eq!(f.app.interacting, Some(uid));
        let mut terminal = Terminal::new(TestBackend::new(100, 14)).unwrap();
        terminal.draw(|frame| f.app.draw(frame)).unwrap();
        let buf = terminal.backend().buffer();
        let bottom: String = (0..100).map(|x| buf[(x, 13)].symbol()).collect();
        assert!(bottom.contains("Interacting · Ctrl+E to stop"), "{bottom}");

        f.key("");
        assert_eq!(f.app.interacting, Some(uid));
        f.key("");
        assert_eq!(f.app.interacting, None);
    }

    #[test]
    fn interact_ctrl_k_with_an_unknown_key_forwards_both() {
        let mut f = fixture();
        let uid = f.add(Some("abc"), true);
        f.key("i");
        f.wait_for_screen(uid, "> ");
        f.key("\x0b");
        assert!(f.app.chord_pending);
        f.key("x");
        f.key("\r");
        f.wait_for_screen(uid, "echo: \\u{b}x");
    }

    #[test]
    fn interact_chords_toggle_mouse_and_sidebar_and_q_stops() {
        let mut f = fixture();
        let uid = f.add(Some("abc"), true);
        f.key("i");
        f.key("\x0bm");
        assert!(f.app.mouse_tracking);
        assert_eq!(f.app.mouse_capture_change, Some(true));
        f.key("\x0bb");
        assert!(!f.app.sidebar_visible);
        f.key("\x0bq");
        assert_eq!(f.app.interacting, None);
        assert!(!f.session(uid).unwrap().is_live());
    }

    #[test]
    fn interact_wheel_scrolls_the_preview_instead_of_reaching_the_agent() {
        let mut f = fixture();
        let uid = f.add(Some("abc"), true);
        f.app.set_size(100, 14);
        f.key("i");
        f.wait_for_screen(uid, "> ");
        for i in 0..20 {
            f.key(&format!("line{i}\r"));
        }
        f.wait_for_screen(uid, "echo: line19");
        f.key("m");
        assert!(
            !f.app.mouse_tracking,
            "m is typed into the agent while interacting"
        );
        f.key("\x1b[<64;50;5M");
        assert_eq!(f.app.preview_scroll, 1);
        f.key("\x1b[<65;50;5M");
        assert_eq!(f.app.preview_scroll, 0);
    }

    #[test]
    fn m_toggles_mouse_scrolling_from_the_list() {
        let mut f = fixture();
        f.key("m");
        assert!(f.app.mouse_tracking);
        assert!(f.app.message.starts_with("Mouse scrolling on"));
        f.key("m");
        assert!(!f.app.mouse_tracking);
        assert_eq!(f.app.mouse_capture_change, Some(false));
    }
}
