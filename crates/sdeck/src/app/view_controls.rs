//! `<` `>` resize the sessions panel (persisted), `b` hides it, and the trash purge at startup.
//! Port of `App.resizeSidebar`, `toggleSidebar` and the `purgeTrash` call in `run`.

use std::time::{Duration, Instant};

use sdeck_core::store::deck_store::{Patch, UiPatch};
use sdeck_core::store::trash::purge_trash;

use super::App;
use crate::layout::step_sidebar;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
const RESIZE_NOTE: Duration = Duration::from_millis(1500);

impl App {
    /// `<` / `>`: moves the panel boundary one step, shows the new width in the list header for a
    /// moment and remembers it.
    pub(super) fn resize_sidebar(&mut self, delta: f64, now: Instant) {
        self.sidebar_pct = step_sidebar(self.sidebar_pct, delta);
        self.resize_note = Some((format!("{}%", self.sidebar_pct), now + RESIZE_NOTE));
        let _ = self.store.update_ui(&UiPatch {
            sidebar_pct: Patch::Set(self.sidebar_pct),
            ..Default::default()
        });
        self.clear_screen = true;
        self.dirty = true;
        self.resize_all_to_pane();
    }

    /// The width note while it's still showing.
    pub(super) fn active_resize_note(&self, now: Instant) -> Option<&str> {
        self.resize_note
            .as_ref()
            .filter(|(_, until)| now < *until)
            .map(|(text, _)| text.as_str())
    }

    /// Drops trashed sessions older than `trash.retentionDays`.
    pub(super) fn purge_expired_trash(&self) {
        let ttl_ms = i64::try_from(self.config.trash.retention_days)
            .unwrap_or(i64::MAX / DAY_MS)
            .saturating_mul(DAY_MS);
        let _ = purge_trash(chrono::Utc::now().timestamp_millis(), ttl_ms);
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_fixture::fixture;
    use super::*;
    use crate::layout::SIDEBAR_STEP;
    use sdeck_core::store::deck_store::{SIDEBAR_PCT_MAX, SIDEBAR_PCT_MIN};

    #[test]
    fn resize_clamps_persists_and_shows_a_note() {
        let mut f = fixture();
        let now = Instant::now();
        f.app.sidebar_pct = SIDEBAR_PCT_MAX;
        f.key(">");
        assert_eq!(f.app.sidebar_pct, SIDEBAR_PCT_MAX);
        f.app.sidebar_pct = SIDEBAR_PCT_MIN;
        f.key("<");
        assert_eq!(f.app.sidebar_pct, SIDEBAR_PCT_MIN);
        f.key(">");
        assert_eq!(f.app.sidebar_pct, SIDEBAR_PCT_MIN + SIDEBAR_STEP);
        assert_eq!(
            f.app.store.get_ui().sidebar_pct,
            Some(SIDEBAR_PCT_MIN + SIDEBAR_STEP)
        );
        assert!(f.app.active_resize_note(Instant::now()).is_some());
        assert!(f.app.active_resize_note(now + Duration::from_secs(5)).is_none());
    }

    #[test]
    fn b_toggles_the_sidebar() {
        let mut f = fixture();
        f.key("b");
        assert!(!f.app.sidebar_visible);
        f.key("b");
        assert!(f.app.sidebar_visible);
    }
}
