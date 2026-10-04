//! `/`: full-text search over every session's prompts and replies. Port of `openSearch`,
//! `loadSearchText`, `onSearchInput`, `applySearchFilter` and `onSearchKey`. The text is read once,
//! on a background thread, when the first character is typed; after that every keystroke
//! re-filters synchronously.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use sdeck_core::concurrency::map_with_concurrency;
use sdeck_core::discovery::claude_storage::read_session_search_text;
use sdeck_core::discovery::copilot_storage::copilot_session_search_text;
use sdeck_core::fuzzy_match::fuzzy_match;

use super::overlays::{typed_text, Overlay};
use super::App;
use crate::event::AppEvent;
use crate::filters::filter_key_category;
use crate::sessions::DeckSession;
use crate::tree::project_labels;
use crate::view::overlays::SearchResultRow;

/// How many transcripts are read at once.
const SEARCH_READ_CONCURRENCY: usize = 8;

pub(super) struct SearchState {
    /// Tells this search's text apart from one opened (and closed) earlier.
    pub id: u64,
    pub query: String,
    /// Matching sessions, best first, by `DeckSession::uid`.
    pub results: Vec<u64>,
    pub index: usize,
    pub loading: bool,
    /// Session id → prompts and replies; `None` until read.
    pub text: Option<HashMap<String, String>>,
}

impl App {
    /// Sessions of projects that haven't been removed from the list.
    fn unhidden_sessions(&self) -> Vec<&DeckSession> {
        let hidden = self.hidden_projects();
        self.sessions
            .iter()
            .filter(|s| !hidden.contains(&s.project_key))
            .collect()
    }

    pub(super) fn open_search(&mut self) {
        self.next_overlay_id += 1;
        self.overlay = Some(Overlay::Search(SearchState {
            id: self.next_overlay_id,
            query: String::new(),
            results: Vec::new(),
            index: 0,
            loading: false,
            text: None,
        }));
    }

    pub(super) fn on_search_key(&mut self, key: &str, now: Instant) {
        let Some(Overlay::Search(search)) = &mut self.overlay else {
            return;
        };
        match key {
            "\x1b" | "\x03" => self.overlay = None,
            "\r" => {
                let target = search.results.get(search.index).copied();
                self.overlay = None;
                if let Some(uid) = target {
                    self.jump_to_session(uid, now);
                }
            }
            "\x1b[A" | "\x1bOA" => search.index = search.index.saturating_sub(1),
            "\x1b[B" | "\x1bOB" => {
                search.index = (search.index + 1).min(search.results.len().saturating_sub(1))
            }
            "\x7f" | "\x08" => {
                search.query.pop();
                self.on_search_input();
            }
            "\x15" => {
                search.query.clear();
                self.on_search_input();
            }
            _ => {
                if let Some(text) = typed_text(key) {
                    search.query.push_str(&text);
                    self.on_search_input();
                }
            }
        }
    }

    /// Reads the content on first use, then re-filters.
    fn on_search_input(&mut self) {
        let Some(Overlay::Search(search)) = &mut self.overlay else {
            return;
        };
        if search.text.is_some() {
            self.apply_search_filter();
            return;
        }
        if search.loading {
            // Already reading: the filter runs once that arrives, against the latest query.
            return;
        }
        search.loading = true;
        let id = search.id;
        let items: Vec<(String, Option<PathBuf>, bool)> = self
            .unhidden_sessions()
            .into_iter()
            .filter_map(|s| Some((s.id.clone()?, s.file.clone(), s.agent == "claude")))
            .collect();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let pairs = map_with_concurrency(
                &items,
                SEARCH_READ_CONCURRENCY,
                |(session_id, file, claude), _| {
                    let text = if *claude {
                        file.as_deref().map(read_session_search_text).unwrap_or_default()
                    } else {
                        copilot_session_search_text(session_id)
                    };
                    (session_id.clone(), text)
                },
            );
            let _ = tx.send(AppEvent::SearchText(id, pairs.into_iter().collect()));
        });
    }

    pub(super) fn on_search_text(&mut self, id: u64, text: HashMap<String, String>) {
        let Some(Overlay::Search(search)) = &mut self.overlay else {
            return;
        };
        if search.id != id {
            // Closed, or reopened, while this was loading.
            return;
        }
        search.text = Some(text);
        search.loading = false;
        self.apply_search_filter();
        self.dirty = true;
    }

    /// A leading `!`/`@`/`#`/`&`/`~` filters by status, same as the main filter pills.
    fn apply_search_filter(&mut self) {
        let Some(Overlay::Search(search)) = &self.overlay else {
            return;
        };
        let results = self.search_results(&search.query, search.text.as_ref());
        let Some(Overlay::Search(search)) = &mut self.overlay else {
            return;
        };
        search.index = search.index.min(results.len().saturating_sub(1));
        search.results = results;
    }

    fn search_results(&self, query: &str, text: Option<&HashMap<String, String>>) -> Vec<u64> {
        let trimmed = query.trim();
        let Some(first) = trimmed.chars().next() else {
            return Vec::new();
        };
        let status = filter_key_category(&first.to_string());
        let needle = if status.is_some() {
            trimmed[first.len_utf8()..].trim()
        } else {
            trimmed
        }
        .to_lowercase();
        let Some(text) = text else {
            return Vec::new();
        };
        let mut scored: Vec<(f64, u64)> = self
            .unhidden_sessions()
            .into_iter()
            .filter_map(|s| {
                let id = s.id.as_deref()?;
                if status.is_some_and(|category| self.procs.category_of(s) != category) {
                    return None;
                }
                let found = fuzzy_match(&needle, text.get(id).map_or("", String::as_str));
                found.matched.then_some((found.score, s.uid))
            })
            .collect();
        scored.sort_by(|a, b| a.0.total_cmp(&b.0));
        scored.into_iter().map(|(_, uid)| uid).collect()
    }

    pub(super) fn search_result_rows(&self, search: &SearchState) -> Vec<SearchResultRow> {
        let roots: Vec<String> = self
            .unhidden_sessions()
            .into_iter()
            .map(|s| s.project_root.clone())
            .collect();
        let labels = project_labels(&roots);
        search
            .results
            .iter()
            .filter_map(|uid| self.session_by_uid(*uid))
            .map(|s| SearchResultRow {
                view: self.view_of(s),
                project_label: labels
                    .get(&s.project_root)
                    .cloned()
                    .unwrap_or_else(|| s.project_root.clone()),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{Duration, Instant};

    use sdeck_core::status::account::Account;
    use sdeck_core::status::claude_process_watcher::LiveClaudeProcess;
    use serde_json::json;

    use crate::app::test_fixture::{fixture, Fixture};
    use crate::app::Overlay;
    use crate::event::AppEvent;

    fn transcript(prompt: &str, reply: &str) -> String {
        let user = json!({"type": "user", "message": {"role": "user", "content": prompt}});
        let answer = json!({
            "type": "assistant",
            "message": {"role": "assistant", "content": [{"type": "text", "text": reply}]}
        });
        format!("{user}\n{answer}\n")
    }

    fn add_with_transcript(f: &mut Fixture, id: &str, prompt: &str, reply: &str) -> u64 {
        let uid = f.add(Some(id), true);
        let file = f.home.path().join(format!("{id}.jsonl"));
        fs::write(&file, transcript(prompt, reply)).unwrap();
        f.app.session_mut(uid).unwrap().file = Some(file);
        uid
    }

    fn pump_search_text(f: &mut Fixture) {
        loop {
            let ev =
                f.rx.recv_timeout(Duration::from_secs(10))
                    .expect("search text arrives");
            let is_text = matches!(ev, AppEvent::SearchText(..));
            f.app.handle(ev, Instant::now());
            if is_text {
                return;
            }
        }
    }

    fn results(f: &Fixture) -> Vec<u64> {
        match &f.app.overlay {
            Some(Overlay::Search(s)) => s.results.clone(),
            _ => panic!("search is open"),
        }
    }

    #[test]
    fn a_word_only_in_a_reply_finds_its_session() {
        let mut f = fixture();
        let a = add_with_transcript(&mut f, "aaa", "fix the build", "restart the daemon");
        let b = add_with_transcript(&mut f, "bbb", "write docs", "done");
        f.key("/");
        assert!(matches!(f.app.overlay, Some(Overlay::Search(_))));
        f.key("d");
        assert!(matches!(&f.app.overlay, Some(Overlay::Search(s)) if s.loading));
        pump_search_text(&mut f);
        f.key("aemon");
        assert_eq!(results(&f), [a]);

        f.key("\x15");
        assert!(results(&f).is_empty());
        f.key("docs");
        assert_eq!(results(&f), [b]);
    }

    #[test]
    fn typing_before_the_text_arrives_filters_once_it_does() {
        let mut f = fixture();
        let a = add_with_transcript(&mut f, "aaa", "x", "needle");
        f.key("/");
        f.key("need");
        f.key("le");
        pump_search_text(&mut f);
        assert_eq!(results(&f), [a]);
        assert!(matches!(&f.app.overlay, Some(Overlay::Search(s)) if !s.loading));
    }

    #[test]
    fn a_status_prefix_limits_the_results_to_that_status() {
        let mut f = fixture();
        let waiting = add_with_transcript(&mut f, "aaa", "common words", "x");
        let idle = add_with_transcript(&mut f, "bbb", "common words", "y");
        f.app.procs.by_session.insert(
            "aaa".into(),
            LiveClaudeProcess {
                pid: 1,
                session_id: "aaa".into(),
                status: "waiting".into(),
                cwd: None,
                account: Account {
                    config_dir: r"C:\.claude".into(),
                    email: None,
                    is_default: true,
                },
            },
        );
        f.key("/");
        f.key("common");
        pump_search_text(&mut f);
        assert_eq!(results(&f).len(), 2);
        f.key("\x15");
        f.key("@common");
        assert_eq!(results(&f), [waiting]);
        assert!(!results(&f).contains(&idle));
    }

    #[test]
    fn enter_selects_the_result_and_escape_cancels() {
        let mut f = fixture();
        let a = add_with_transcript(&mut f, "aaa", "x", "needle");
        let _b = add_with_transcript(&mut f, "bbb", "x", "other");
        f.key("/");
        f.key("needle");
        pump_search_text(&mut f);
        f.key("\r");
        assert!(f.app.overlay.is_none());
        assert_eq!(f.app.selected_session().map(|s| s.uid), Some(a));

        f.key("/");
        f.key("q");
        f.key("\x1b");
        assert!(f.app.overlay.is_none());
    }

    #[test]
    fn text_that_arrives_after_the_search_closed_is_ignored() {
        let mut f = fixture();
        add_with_transcript(&mut f, "aaa", "x", "needle");
        f.key("/");
        f.key("n");
        f.key("\x1b");
        f.key("/");
        pump_search_text(&mut f);
        assert!(matches!(&f.app.overlay, Some(Overlay::Search(s)) if s.text.is_none()));
    }
}
