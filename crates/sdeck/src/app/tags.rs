//! Tags: `L` edits the selected session's tags (or adds to a checked batch), and `d` on a tag row
//! drops it everywhere. Port of `openTagPrompt`, `parseTagsInput`, `setSessionTags`,
//! `addTagsToSessions` and `removeTagEverywhere`.

use std::time::Instant;

use sdeck_core::store::deck_store::{Patch, SessionPatch};

use super::input::PromptAction;
use super::App;
use crate::sessions::display_title;

/// Comma-separated, trimmed, without blanks or repeats.
fn parse_tags_input(raw: &str) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    for tag in raw.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        if !tags.iter().any(|t| t == tag) {
            tags.push(tag.to_string());
        }
    }
    tags
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

impl App {
    fn write_tags(&mut self, updates: Vec<(String, Vec<String>)>) {
        let patches: Vec<(String, SessionPatch)> = updates
            .into_iter()
            .map(|(id, tags)| {
                let patch = SessionPatch {
                    tags: if tags.is_empty() {
                        Patch::Clear
                    } else {
                        Patch::Set(tags)
                    },
                    ..SessionPatch::default()
                };
                (id, patch)
            })
            .collect();
        let _ = self.store.update_sessions(&patches);
        self.rebuild_rows();
    }

    /// `L`.
    pub(super) fn open_tag_prompt(&mut self, now: Instant) {
        if !self.multi_selected.is_empty() {
            let ids: Vec<String> = self
                .multi_selection()
                .into_iter()
                .filter_map(|uid| self.session_by_uid(uid).and_then(|s| s.id.clone()))
                .collect();
            let label = format!("Add tag(s) to {} session{}", ids.len(), plural(ids.len()));
            self.open_prompt(label, String::new(), PromptAction::AddTags(ids));
            return;
        }
        let Some(s) = self.selected_session() else {
            self.flash("Select a session to tag.".into(), now);
            return;
        };
        let Some(id) = s.id.clone() else {
            self.flash(
                "Send something first: a new session has nothing to tag yet.".into(),
                now,
            );
            return;
        };
        let label = format!("Tags for {} (comma-separated)", display_title(s, &self.store));
        let value = self.tags_of(s).join(", ");
        self.open_prompt(label, value, PromptAction::SetTags(id));
    }

    pub(super) fn set_session_tags(&mut self, id: &str, raw: &str, now: Instant) {
        let tags = parse_tags_input(raw);
        let text = if tags.is_empty() {
            "Tags cleared".to_string()
        } else {
            format!("Tags: {}", tags.join(", "))
        };
        self.write_tags(vec![(id.to_string(), tags)]);
        self.flash(text, now);
    }

    /// Unions the entered tag(s) into every target's existing tags (never replaces them).
    pub(super) fn add_tags_to_sessions(&mut self, ids: &[String], raw: &str, now: Instant) {
        let added = parse_tags_input(raw);
        self.multi_selected.clear();
        if added.is_empty() {
            self.rebuild_rows();
            self.flash("Selection cleared".into(), now);
            return;
        }
        let updates = ids
            .iter()
            .map(|id| {
                let mut tags = self.store.get_session(id).map(|p| p.tags).unwrap_or_default();
                for tag in &added {
                    if !tags.contains(tag) {
                        tags.push(tag.clone());
                    }
                }
                (id.clone(), tags)
            })
            .collect();
        self.write_tags(updates);
        self.flash(
            format!(
                "Added \"{}\" to {} session{}",
                added.join(", "),
                ids.len(),
                plural(ids.len())
            ),
            now,
        );
    }

    /// `d` on a tag row: drops it from every session that carries it.
    pub(super) fn remove_tag_everywhere(&mut self, name: &str, now: Instant) {
        let updates: Vec<(String, Vec<String>)> = self
            .sessions
            .iter()
            .filter(|s| self.tags_of(s).iter().any(|t| t == name))
            .filter_map(|s| {
                let id = s.id.clone()?;
                let tags = self.tags_of(s).into_iter().filter(|t| t != name).collect();
                Some((id, tags))
            })
            .collect();
        if self.tag_filter.as_deref() == Some(name) {
            self.tag_filter = None;
        }
        let count = updates.len();
        self.write_tags(updates);
        self.flash(
            format!("Removed tag \"{name}\" from {count} session{}", plural(count)),
            now,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::parse_tags_input;
    use crate::app::test_fixture::{fixture, Fixture};
    use crate::sessions::DeckSession;
    use crate::tree::TreeRow;

    #[test]
    fn parsing_trims_drops_blanks_and_dedupes() {
        assert_eq!(parse_tags_input(" a , b,,a ,  "), ["a", "b"]);
        assert!(parse_tags_input(" , ").is_empty());
        assert_eq!(parse_tags_input("one tag, two"), ["one tag", "two"]);
    }

    fn tags(f: &Fixture, id: &str) -> Vec<String> {
        f.app.store.get_session(id).map(|p| p.tags).unwrap_or_default()
    }

    #[test]
    fn l_edits_a_sessions_tags_prefilled_and_clears_them() {
        let mut f = fixture();
        f.add(Some("a"), true);
        f.key("L");
        assert!(f
            .app
            .prompt
            .as_ref()
            .unwrap()
            .label
            .ends_with("(comma-separated)"));
        f.key("x, y\r");
        assert_eq!(tags(&f, "a"), ["x", "y"]);
        assert_eq!(f.app.message, "Tags: x, y");
        f.key("L");
        assert_eq!(f.app.prompt.as_ref().unwrap().value, "x, y");
        f.key("\x15\r");
        assert!(tags(&f, "a").is_empty());
        assert_eq!(f.app.message, "Tags cleared");
    }

    #[test]
    fn l_on_a_session_without_an_id_asks_to_send_something_first() {
        let mut f = fixture();
        f.add(None, false);
        f.key("L");
        assert_eq!(
            f.app.message,
            "Send something first: a new session has nothing to tag yet."
        );
        assert!(f.app.prompt.is_none());
    }

    #[test]
    fn a_batch_add_keeps_existing_tags() {
        let mut f = fixture();
        let a = f.add(Some("a"), true);
        let b = f.add(Some("b"), true);
        f.select(a);
        f.key("L");
        f.key("old\r");
        f.app.multi_selected.extend([a, b]);
        f.key("L");
        assert_eq!(f.app.prompt.as_ref().unwrap().label, "Add tag(s) to 2 sessions");
        f.key("new, old\r");
        assert_eq!(tags(&f, "a"), ["old", "new"]);
        assert_eq!(tags(&f, "b"), ["new", "old"]);
        assert!(f.app.multi_selected.is_empty());
        assert_eq!(f.app.message, "Added \"new, old\" to 2 sessions");
    }

    #[test]
    fn the_tag_filter_shows_sessions_across_projects_and_d_removes_the_tag_everywhere() {
        let mut f = fixture();
        let a = f.add(Some("a"), true);
        f.key("L");
        f.key("urgent\r");
        let other = f.home.path().join("other").to_string_lossy().into_owned();
        let mut s = DeckSession::new("claude", Some("z"), &other, "z", 1);
        s.project_key = other.to_lowercase();
        s.project_root = other;
        f.app.sessions.push(s);
        f.app.rebuild_rows();
        f.select(a);
        f.app.select_where(|r| matches!(r, TreeRow::Tag { .. }));
        f.key("\r");
        assert_eq!(f.app.tag_filter.as_deref(), Some("urgent"));
        let visible = f
            .app
            .rows
            .iter()
            .filter(|r| matches!(r, TreeRow::Session { .. }))
            .count();
        assert_eq!(visible, 1);

        f.app.select_where(|r| matches!(r, TreeRow::Tag { .. }));
        f.key("d");
        assert_eq!(
            f.app.confirm.as_ref().unwrap().question,
            "Remove tag \"urgent\" from every project?"
        );
        f.key("y");
        assert!(tags(&f, "a").is_empty());
        assert_eq!(f.app.tag_filter, None);
        assert_eq!(f.app.message, "Removed tag \"urgent\" from 1 session");
    }
}
