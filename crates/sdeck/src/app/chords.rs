//! Input while attached or interacting: forwarded to the agent, except the detach key (leave) and the
//! Ctrl+K chord. Port of `App.resolveChord`, the `is*Chord` helpers and `onLiveInput`.

use std::time::Instant;

use super::App;
use crate::keybindings::{chord_prefix_letter, ChordKeys};
use crate::keys::{find_chord_key, find_detach_key, find_plain_key};

/// What Ctrl+K's resolving key does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChordAction {
    /// `n`: leave, then a new Claude session in the same project (subfolder kept).
    NewSession,
    /// `t`: swap attached and interacting for the same session.
    SwitchMode,
    /// `q`: leave and stop the session, like `x` from the list.
    StopSession,
    /// `m` (interacting only): sdeck's mouse scrolling.
    ToggleMouse,
    /// `b` (interacting only): the sessions panel.
    ToggleSidebar,
}

/// Ctrl+K's resolving key, if `text` starts with one. Only the very next key counts: an `n` typed
/// later in the same chunk (a fast-typed sentence, a paste) must not eat a Ctrl+K meant for the
/// agent. `m` and `b` resolve only while `interacting`: attached, mouse tracking is always off and
/// there's no panel, so they go to the agent like any other key.
pub(crate) fn resolve_chord(text: &str, interacting: bool, keys: &ChordKeys) -> Option<ChordAction> {
    let first = |ch: char| {
        find_plain_key(text, ch) == Some(0) || find_plain_key(text, ch.to_ascii_uppercase()) == Some(0)
    };
    if first(keys.new_session) {
        Some(ChordAction::NewSession)
    } else if first(keys.switch_mode) {
        Some(ChordAction::SwitchMode)
    } else if first(keys.stop_session) {
        Some(ChordAction::StopSession)
    } else if interacting && first(keys.toggle_mouse) {
        Some(ChordAction::ToggleMouse)
    } else if interacting && first(keys.toggle_sidebar) {
        Some(ChordAction::ToggleSidebar)
    } else {
        None
    }
}

impl App {
    fn write_live(&mut self, uid: u64, data: &[u8]) {
        if let Some(live) = self.session_mut(uid).and_then(|s| s.live.as_mut()) {
            live.write(data);
        }
    }

    fn write_live_input(&mut self, uid: u64, data: &str) {
        if let Some(live) = self.session_mut(uid).and_then(|s| s.live.as_mut()) {
            live.write_input(data);
        }
    }

    /// Input for the attached or interacting session `uid`. A chord's resolving key usually arrives
    /// in the same chunk as Ctrl+K, but may come in the next one (`chord_pending`). Keys before
    /// the detach key are forwarded, the rest of that chunk is dropped. Unlike TS, keys after an unresolved
    /// Ctrl+K in the same chunk are forwarded too (TS dropped them as win32-input-mode key-ups,
    /// which crossterm never delivers).
    pub(super) fn on_live_input(&mut self, uid: u64, data: &str, now: Instant) {
        let interacting = self.interacting == Some(uid);
        let keys = ChordKeys::from_config(&self.config);
        let prefix = chord_prefix_letter(&self.config);
        if std::mem::take(&mut self.chord_pending) {
            if let Some(action) = resolve_chord(data, interacting, &keys) {
                self.run_chord(uid, action, now);
                return;
            }
            self.write_live(uid, b"\x0b"); // not a chord after all: the withheld Ctrl+K goes on
        }
        let chord = find_chord_key(data, prefix);
        let detach = find_detach_key(data, self.config.ui.detach_letter());
        let idx = match (chord, detach) {
            (Some((c, _)), Some(d)) => c.min(d),
            (Some((c, _)), None) => c,
            (None, Some(d)) => d,
            (None, None) => {
                self.write_live_input(uid, data);
                return;
            }
        };
        if idx > 0 {
            self.write_live_input(uid, &data[..idx]);
        }
        match chord {
            Some((start, end)) if start == idx => {
                let rest = &data[end..];
                if let Some(action) = resolve_chord(rest, interacting, &keys) {
                    self.run_chord(uid, action, now);
                } else {
                    self.chord_pending = true;
                    if !rest.is_empty() {
                        self.on_live_input(uid, rest, now);
                    }
                }
            }
            _ => self.leave_live(uid),
        }
    }

    /// The chord prefix as shown to the user, e.g. `Ctrl+K`.
    pub(super) fn chord_prefix_label(&self) -> String {
        format!(
            "Ctrl+{}",
            crate::keybindings::chord_prefix_letter(&self.config).to_ascii_uppercase()
        )
    }

    /// The detach key as shown to the user, e.g. `Ctrl+E`.
    pub(super) fn detach_label(&self) -> String {
        format!("Ctrl+{}", self.config.ui.detach_letter().to_ascii_uppercase())
    }

    /// The detach key: detach, or stop interacting.
    fn leave_live(&mut self, uid: u64) {
        if self.attached == Some(uid) {
            self.detach(None);
        } else {
            self.stop_interacting();
        }
    }

    fn run_chord(&mut self, uid: u64, action: ChordAction, now: Instant) {
        let interacting = self.interacting == Some(uid);
        match action {
            ChordAction::NewSession => {
                self.leave_live(uid);
                self.new_session("claude", now);
            }
            ChordAction::SwitchMode if interacting => self.switch_to_attach(uid, now),
            ChordAction::SwitchMode => self.switch_to_interacting(uid, now),
            ChordAction::StopSession => {
                self.leave_live(uid);
                self.kill(uid);
                self.flash("Session stopped".into(), now);
            }
            ChordAction::ToggleMouse => self.toggle_mouse_tracking(now),
            ChordAction::ToggleSidebar => self.toggle_sidebar(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_keys() -> ChordKeys {
        ChordKeys::from_config(&sdeck_core::store::deck_config::DeckConfig::default())
    }

    #[test]
    fn chords_resolve_each_key_in_both_cases_and_only_as_the_very_next_key() {
        use ChordAction::*;
        for (key, action) in [
            ("n", NewSession),
            ("T", SwitchMode),
            ("q", StopSession),
            ("Q", StopSession),
        ] {
            assert_eq!(resolve_chord(key, false, &default_keys()), Some(action), "{key}");
        }
        assert_eq!(resolve_chord("N rest", false, &default_keys()), Some(NewSession));
        assert_eq!(
            resolve_chord("\x1b[78;49;110;1;0;1_", false, &default_keys()),
            Some(NewSession)
        );
        assert_eq!(resolve_chord("an", false, &default_keys()), None);
        assert_eq!(resolve_chord("", false, &default_keys()), None);
        assert_eq!(resolve_chord("x", true, &default_keys()), None);
    }

    #[test]
    fn chords_offer_mouse_and_sidebar_only_while_interacting() {
        assert_eq!(
            resolve_chord("m", true, &default_keys()),
            Some(ChordAction::ToggleMouse)
        );
        assert_eq!(
            resolve_chord("B", true, &default_keys()),
            Some(ChordAction::ToggleSidebar)
        );
        assert_eq!(resolve_chord("m", false, &default_keys()), None);
        assert_eq!(resolve_chord("b", false, &default_keys()), None);
    }
}
