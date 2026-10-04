use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};

use indexmap::IndexMap;
use notify::{RecursiveMode, Watcher};
use serde_json::{Map, Value};

use super::atomic::write_atomic;
use super::deck_config::extras;
use super::tree_prefs::{
    default_tree_prefs, is_default_tree, parse_tree_prefs, tree_prefs_to_value, SessionPin, TreePrefs,
};
use crate::format::now_ms;
use crate::paths::deck_home;

/// Per-session preferences, keyed by session id.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionPrefs {
    /// sdeck's own name override. Wins over the agent's own title.
    pub name: Option<String>,
    pub archived: bool,
    /// Kept at the top or bottom of its project, above/below the sort order.
    pub pin: Option<SessionPin>,
    /// `updatedAt` of the "done"/"error" status the user has seen. A newer one is unseen.
    pub seen_at: Option<i64>,
    /// Free-form labels, this session only. Empty lists are dropped.
    pub tags: Vec<String>,
    /// Fields this version doesn't know, kept so a write never drops them.
    pub extra: Map<String, Value>,
}

impl SessionPrefs {
    fn is_empty(&self) -> bool {
        self.name.is_none()
            && !self.archived
            && self.pin.is_none()
            && self.seen_at.is_none()
            && self.tags.is_empty()
            && self.extra.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemePreference {
    Dark,
    Light,
    System,
}

impl ThemePreference {
    fn as_str(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::System => "system",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        [Self::Dark, Self::Light, Self::System]
            .into_iter()
            .find(|t| t.as_str() == s)
    }
}

/// Terminal UI preferences.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UiPrefs {
    pub theme: Option<ThemePreference>,
    /// Sessions panel width, as a percentage of the terminal width.
    pub sidebar_pct: Option<f64>,
    /// Which agent `n` (new session) starts, and whose info popups are shown. An unrecognized or
    /// removed catalog id falls back to `claude`.
    pub active_agent: Option<String>,
    pub extra: Map<String, Value>,
}

impl UiPrefs {
    fn is_empty(&self) -> bool {
        self.theme.is_none()
            && self.sidebar_pct.is_none()
            && self.active_agent.is_none()
            && self.extra.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeckStateFile {
    pub sessions: IndexMap<String, SessionPrefs>,
    pub ui: Option<UiPrefs>,
    pub tree: Option<TreePrefs>,
    /// Project keys removed from the list (see `DeckStore::set_project_hidden`). A session discovered
    /// later for one of these keys stays off the list until the project is added again.
    pub hidden_projects: Vec<String>,
    pub extra: Map<String, Value>,
}

pub const SIDEBAR_PCT_MIN: f64 = 15.0;
pub const SIDEBAR_PCT_MAX: f64 = 70.0;

pub fn deck_state_path() -> PathBuf {
    deck_home().join("state.json")
}

/// `None` for anything that isn't a readable state file. Invalid entries are dropped rather than
/// failing the whole file.
pub fn parse_deck_state(raw: &str) -> Option<DeckStateFile> {
    let Ok(Value::Object(root)) = serde_json::from_str::<Value>(raw) else {
        return None;
    };
    let mut state = DeckStateFile {
        extra: extras(&root, &["version", "sessions", "ui", "tree", "hiddenProjects"]),
        ..DeckStateFile::default()
    };
    state.ui = root.get("ui").and_then(parse_ui_prefs);
    state.tree = root.get("tree").and_then(parse_tree_prefs);
    if let Some(items) = root.get("hiddenProjects").and_then(Value::as_array) {
        let keys: Vec<String> = items
            .iter()
            .filter_map(|k| k.as_str().map(str::to_string))
            .collect();
        state.hidden_projects = dedupe(keys);
    }
    let Some(sessions) = root.get("sessions").and_then(Value::as_object) else {
        return Some(state);
    };
    for (id, value) in sessions {
        let Some(obj) = value.as_object() else { continue };
        let mut prefs = SessionPrefs {
            extra: extras(obj, &["name", "archived", "pin", "seenAt", "tags"]),
            ..SessionPrefs::default()
        };
        if let Some(name) = obj.get("name").and_then(Value::as_str) {
            if !name.trim().is_empty() {
                prefs.name = Some(name.to_string());
            }
        }
        prefs.archived = obj.get("archived") == Some(&Value::Bool(true));
        prefs.pin = match obj.get("pin").and_then(Value::as_str) {
            Some("top") => Some(SessionPin::Top),
            Some("bottom") => Some(SessionPin::Bottom),
            _ => None,
        };
        prefs.seen_at = obj
            .get("seenAt")
            .and_then(Value::as_f64)
            .filter(|n| *n > 0.0)
            .map(|n| n as i64);
        if let Some(tags) = obj.get("tags").and_then(Value::as_array) {
            prefs.tags = normalize_tags(tags.iter().filter_map(Value::as_str));
        }
        if !prefs.is_empty() {
            state.sessions.insert(id.clone(), prefs);
        }
    }
    Some(state)
}

fn parse_ui_prefs(ui: &Value) -> Option<UiPrefs> {
    let obj = ui.as_object()?;
    let prefs = UiPrefs {
        theme: obj
            .get("theme")
            .and_then(Value::as_str)
            .and_then(ThemePreference::parse),
        sidebar_pct: obj
            .get("sidebarPct")
            .and_then(Value::as_f64)
            .filter(|p| (SIDEBAR_PCT_MIN..=SIDEBAR_PCT_MAX).contains(p)),
        active_agent: obj
            .get("activeAgent")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        extra: extras(obj, &["theme", "sidebarPct", "activeAgent"]),
    };
    (!prefs.is_empty()).then_some(prefs)
}

fn dedupe(items: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    items.into_iter().filter(|s| seen.insert(s.clone())).collect()
}

/// Trims, drops blanks, and dedupes (exact match) while keeping first-seen order.
fn normalize_tags<'a>(tags: impl Iterator<Item = &'a str>) -> Vec<String> {
    dedupe(
        tags.map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .collect(),
    )
}

/// A JSON number that prints as an integer when it is one.
fn number_value(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        Value::from(n as i64)
    } else {
        Value::from(n)
    }
}

fn ui_prefs_to_value(ui: &UiPrefs) -> Value {
    let mut obj = Map::new();
    if let Some(t) = ui.theme {
        obj.insert("theme".into(), t.as_str().into());
    }
    if let Some(p) = ui.sidebar_pct {
        obj.insert("sidebarPct".into(), number_value(p));
    }
    if let Some(a) = &ui.active_agent {
        obj.insert("activeAgent".into(), a.clone().into());
    }
    obj.extend(ui.extra.clone());
    Value::Object(obj)
}

fn session_prefs_to_value(prefs: &SessionPrefs) -> Value {
    let mut obj = Map::new();
    if let Some(n) = &prefs.name {
        obj.insert("name".into(), n.clone().into());
    }
    if prefs.archived {
        obj.insert("archived".into(), true.into());
    }
    if let Some(p) = prefs.pin {
        obj.insert("pin".into(), p.as_str().into());
    }
    if let Some(s) = prefs.seen_at {
        obj.insert("seenAt".into(), s.into());
    }
    if !prefs.tags.is_empty() {
        obj.insert(
            "tags".into(),
            prefs.tags.iter().cloned().map(Value::String).collect(),
        );
    }
    obj.extend(prefs.extra.clone());
    Value::Object(obj)
}

pub fn deck_state_to_json(state: &DeckStateFile) -> String {
    let mut root = Map::new();
    root.insert("version".into(), 1.into());
    root.insert(
        "sessions".into(),
        Value::Object(
            state
                .sessions
                .iter()
                .map(|(id, p)| (id.clone(), session_prefs_to_value(p)))
                .collect(),
        ),
    );
    if let Some(ui) = &state.ui {
        root.insert("ui".into(), ui_prefs_to_value(ui));
    }
    if let Some(tree) = &state.tree {
        root.insert("tree".into(), tree_prefs_to_value(tree));
    }
    if !state.hidden_projects.is_empty() {
        root.insert(
            "hiddenProjects".into(),
            state.hidden_projects.iter().cloned().map(Value::String).collect(),
        );
    }
    root.extend(state.extra.clone());
    format!(
        "{}\n",
        serde_json::to_string_pretty(&Value::Object(root)).expect("state serializes")
    )
}

/// A field in a patch: leave it alone, clear it, or set it. A blank/`false`/out-of-range `Set` clears
/// the field, same as `Clear`.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Patch<T> {
    #[default]
    Keep,
    Clear,
    Set(T),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct UiPatch {
    pub theme: Patch<ThemePreference>,
    pub sidebar_pct: Patch<f64>,
    pub active_agent: Patch<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionPatch {
    pub name: Patch<String>,
    pub archived: Patch<bool>,
    pub pin: Patch<SessionPin>,
    pub seen_at: Patch<i64>,
    pub tags: Patch<Vec<String>>,
}

fn merge<T: Clone>(current: &Option<T>, patch: &Patch<T>) -> Option<T> {
    match patch {
        Patch::Keep => current.clone(),
        Patch::Clear => None,
        Patch::Set(v) => Some(v.clone()),
    }
}

/// Pure: merges `patch` into the UI prefs. A cleared or out-of-range value clears that field.
pub fn apply_ui_patch(state: &DeckStateFile, patch: &UiPatch) -> DeckStateFile {
    let current = state.ui.clone().unwrap_or_default();
    let merged = UiPrefs {
        theme: merge(&current.theme, &patch.theme),
        sidebar_pct: merge(&current.sidebar_pct, &patch.sidebar_pct)
            .filter(|p| (SIDEBAR_PCT_MIN..=SIDEBAR_PCT_MAX).contains(p)),
        active_agent: merge(&current.active_agent, &patch.active_agent)
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty()),
        extra: current.extra,
    };
    let mut next = state.clone();
    next.ui = (!merged.is_empty()).then_some(merged);
    next
}

/// Pure: a cleared/`false`/blank value in `patch` clears that field, and an entry left with no fields is removed.
pub fn apply_session_patch(state: &DeckStateFile, session_id: &str, patch: &SessionPatch) -> DeckStateFile {
    let mut next = state.sessions.get(session_id).cloned().unwrap_or_default();
    match &patch.name {
        Patch::Keep => {}
        Patch::Set(n) if !n.trim().is_empty() => next.name = Some(n.clone()),
        _ => next.name = None,
    }
    match &patch.archived {
        Patch::Keep => {}
        Patch::Set(true) => next.archived = true,
        _ => next.archived = false,
    }
    match &patch.pin {
        Patch::Keep => {}
        Patch::Set(p) => next.pin = Some(*p),
        Patch::Clear => next.pin = None,
    }
    match &patch.seen_at {
        Patch::Keep => {}
        Patch::Set(s) if *s != 0 => next.seen_at = Some(*s),
        _ => next.seen_at = None,
    }
    match &patch.tags {
        Patch::Keep => {}
        Patch::Set(t) => next.tags = normalize_tags(t.iter().map(String::as_str)),
        Patch::Clear => next.tags = vec![],
    }
    let mut out = state.clone();
    if next.is_empty() {
        out.sessions.shift_remove(session_id);
    } else {
        out.sessions.insert(session_id.to_string(), next);
    }
    out
}

/// Pure: adds or removes a project key from the hidden list.
pub fn apply_hidden_project_patch(state: &DeckStateFile, project_key: &str, hidden: bool) -> DeckStateFile {
    let mut next = state.clone();
    next.hidden_projects.retain(|k| k != project_key);
    if hidden {
        next.hidden_projects.push(project_key.to_string());
    }
    next
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    modified: Option<SystemTime>,
    size: u64,
}

struct Cache {
    stamp: FileStamp,
    state: Arc<DeckStateFile>,
}

/// Keeps a directory watch alive; dropping it stops the watch.
pub struct WatchHandle {
    _watcher: notify::RecommendedWatcher,
}

/// `~/.session-deck/state.json`. There is no cross-process lock: each write re-reads the file,
/// applies only its own patch, and swaps the result in atomically, so readers never see a half-written
/// file and at worst an edit made in the same few milliseconds by another `sdeck` is lost. Writes from
/// threads of this process are serialized.
pub struct DeckStore {
    pub file_path: PathBuf,
    cache: Mutex<Option<Cache>>,
    write_lock: Mutex<()>,
}

fn stamp_of(path: &Path) -> Option<FileStamp> {
    let meta = fs::metadata(path).ok()?;
    Some(FileStamp {
        modified: meta.modified().ok(),
        size: meta.len(),
    })
}

impl DeckStore {
    pub fn new(file_path: PathBuf) -> Self {
        DeckStore {
            file_path,
            cache: Mutex::new(None),
            write_lock: Mutex::new(()),
        }
    }

    pub fn at_default_path() -> Self {
        Self::new(deck_state_path())
    }

    /// Cached by mtime+size, so calling it per tree row costs one `stat`.
    pub fn read(&self) -> Arc<DeckStateFile> {
        let Some(stamp) = stamp_of(&self.file_path) else {
            return Arc::new(DeckStateFile::default());
        };
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = cache.as_ref() {
            if c.stamp == stamp {
                return c.state.clone();
            }
        }
        let Ok(raw) = fs::read_to_string(&self.file_path) else {
            return cache
                .as_ref()
                .map_or_else(|| Arc::new(DeckStateFile::default()), |c| c.state.clone());
        };
        let state = Arc::new(parse_deck_state(&raw).unwrap_or_default());
        *cache = Some(Cache {
            stamp,
            state: state.clone(),
        });
        state
    }

    pub fn get_session(&self, session_id: &str) -> Option<SessionPrefs> {
        self.read().sessions.get(session_id).cloned()
    }

    pub fn get_ui(&self) -> UiPrefs {
        self.read().ui.clone().unwrap_or_default()
    }

    pub fn update_ui(&self, patch: &UiPatch) -> std::io::Result<()> {
        self.modify(|state| apply_ui_patch(&state, patch))
    }

    pub fn get_tree(&self) -> TreePrefs {
        self.read().tree.clone().unwrap_or_else(default_tree_prefs)
    }

    pub fn get_hidden_projects(&self) -> Vec<String> {
        self.read().hidden_projects.clone()
    }

    /// Hides (or brings back) a project by key. See `DeckStateFile::hidden_projects`.
    pub fn set_project_hidden(&self, project_key: &str, hidden: bool) -> std::io::Result<()> {
        self.modify(|state| apply_hidden_project_patch(&state, project_key, hidden))
    }

    /// Every distinct tag in use across all sessions, alphabetically, for the tag summary under the tree.
    pub fn get_all_tags(&self) -> Vec<String> {
        let mut tags: Vec<String> = dedupe(
            self.read()
                .sessions
                .values()
                .flat_map(|p| p.tags.iter().cloned())
                .collect(),
        );
        tags.sort_by_key(|t| t.to_lowercase());
        tags
    }

    /// `change` gets the tree as it is on disk right now (not the cached copy), so edits from another `sdeck` are kept.
    pub fn update_tree(&self, change: impl FnOnce(TreePrefs) -> TreePrefs) -> std::io::Result<()> {
        self.modify(|mut state| {
            let tree = change(state.tree.take().unwrap_or_else(default_tree_prefs));
            state.tree = (!is_default_tree(&tree)).then_some(tree);
            state
        })
    }

    pub fn update_session(&self, session_id: &str, patch: &SessionPatch) -> std::io::Result<()> {
        self.update_sessions(&[(session_id.to_string(), patch.clone())])
    }

    /// Several patches in one write (e.g. a migration).
    pub fn update_sessions(&self, patches: &[(String, SessionPatch)]) -> std::io::Result<()> {
        self.modify(|mut state| {
            for (id, patch) in patches {
                state = apply_session_patch(&state, id, patch);
            }
            state
        })
    }

    /// Loads and re-saves the file, normalizing it. Used by the roundtrip example.
    pub fn resave(&self) -> std::io::Result<()> {
        self.modify(|state| state)
    }

    fn modify(&self, change: impl FnOnce(DeckStateFile) -> DeckStateFile) -> std::io::Result<()> {
        let _guard = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        let next = change(self.read_for_write());
        write_atomic(&self.file_path, &deck_state_to_json(&next))?;
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = None;
        Ok(())
    }

    /// Bypasses the cache. A file that exists but can't be parsed is kept aside as a backup instead of
    /// being overwritten.
    fn read_for_write(&self) -> DeckStateFile {
        let Ok(raw) = fs::read_to_string(&self.file_path) else {
            return DeckStateFile::default();
        };
        if let Some(state) = parse_deck_state(&raw) {
            return state;
        }
        let mut backup = self.file_path.as_os_str().to_owned();
        backup.push(format!(".corrupt-{}", now_ms()));
        let _ = fs::rename(&self.file_path, backup);
        DeckStateFile::default()
    }

    /// Calls `listener` (debounced, 100 ms) whenever the file changes, from this process or another
    /// `sdeck`. The listener runs on a background thread; the binary turns it into an `AppEvent`.
    pub fn watch(&self, listener: impl Fn() + Send + 'static) -> notify::Result<WatchHandle> {
        let dir = self
            .file_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let base = self.file_path.file_name().map(|n| n.to_owned());
        fs::create_dir_all(&dir)?;
        let (tx, rx) = mpsc::channel::<()>();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(event) = res else { return };
            let relevant = event.paths.is_empty()
                || event
                    .paths
                    .iter()
                    .any(|p| p.file_name().map(|n| n.to_owned()) == base);
            if relevant {
                let _ = tx.send(());
            }
        })?;
        watcher.watch(&dir, RecursiveMode::NonRecursive)?;
        thread::spawn(move || {
            // The sender lives in the watcher; dropping the handle disconnects the channel and ends this thread.
            while rx.recv().is_ok() {
                loop {
                    match rx.recv_timeout(Duration::from_millis(100)) {
                        Ok(()) => continue,
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                listener();
            }
        });
        Ok(WatchHandle { _watcher: watcher })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tree_prefs::{create_folder, delete_folder, move_project_to_folder};
    use serde_json::json;

    fn temp_store() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("state.json");
        (dir, file)
    }

    fn session(pairs: Vec<(&str, SessionPrefs)>) -> IndexMap<String, SessionPrefs> {
        pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }

    fn named(name: &str) -> SessionPrefs {
        SessionPrefs {
            name: Some(name.into()),
            ..SessionPrefs::default()
        }
    }

    fn archived() -> SessionPrefs {
        SessionPrefs {
            archived: true,
            ..SessionPrefs::default()
        }
    }

    fn tags(items: &[&str]) -> SessionPrefs {
        SessionPrefs {
            tags: items.iter().map(|t| t.to_string()).collect(),
            ..SessionPrefs::default()
        }
    }

    #[test]
    fn parse_rejects_non_json_and_keeps_only_valid_session_fields() {
        assert!(parse_deck_state("{not json").is_none());
        let state = parse_deck_state(
            &json!({ "version": 1, "sessions": {
                "a": { "name": "A", "archived": true },
                "b": { "name": "  ", "archived": "yes" },
                "c": 5
            } })
            .to_string(),
        )
        .unwrap();
        assert_eq!(
            state.sessions,
            session(vec![(
                "a",
                SessionPrefs {
                    name: Some("A".into()),
                    archived: true,
                    ..SessionPrefs::default()
                }
            )])
        );
    }

    #[test]
    fn apply_session_patch_sets_and_clears_fields_removing_entries_left_empty() {
        let mut state = apply_session_patch(
            &DeckStateFile::default(),
            "s",
            &SessionPatch {
                name: Patch::Set("N".into()),
                archived: Patch::Set(true),
                ..Default::default()
            },
        );
        assert_eq!(
            state.sessions["s"],
            SessionPrefs {
                name: Some("N".into()),
                archived: true,
                ..Default::default()
            }
        );
        state = apply_session_patch(
            &state,
            "s",
            &SessionPatch {
                archived: Patch::Set(false),
                ..Default::default()
            },
        );
        assert_eq!(state.sessions["s"], named("N"));
        state = apply_session_patch(
            &state,
            "s",
            &SessionPatch {
                name: Patch::Clear,
                ..Default::default()
            },
        );
        assert!(!state.sessions.contains_key("s"));
    }

    #[test]
    fn reads_an_empty_state_when_the_file_does_not_exist() {
        let (_dir, file) = temp_store();
        assert_eq!(*DeckStore::new(file).read(), DeckStateFile::default());
    }

    #[test]
    fn a_write_merges_onto_changes_another_front_end_made_meanwhile() {
        let (_dir, file) = temp_store();
        let first = DeckStore::new(file.clone());
        let second = DeckStore::new(file.clone());
        first
            .update_session(
                "a",
                &SessionPatch {
                    name: Patch::Set("From first".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(
            second.get_session("a").unwrap().name.as_deref(),
            Some("From first")
        );
        first
            .update_session(
                "b",
                &SessionPatch {
                    archived: Patch::Set(true),
                    ..Default::default()
                },
            )
            .unwrap();
        second
            .update_session(
                "a",
                &SessionPatch {
                    name: Patch::Set("From second".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(
            DeckStore::new(file).read().sessions,
            session(vec![("a", named("From second")), ("b", archived())])
        );
    }

    #[test]
    fn backs_up_an_unparseable_file_instead_of_overwriting_it() {
        let (dir, file) = temp_store();
        fs::write(&file, "{broken").unwrap();
        DeckStore::new(file.clone())
            .update_session(
                "a",
                &SessionPatch {
                    archived: Patch::Set(true),
                    ..Default::default()
                },
            )
            .unwrap();
        let backups: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("state.json.corrupt-"))
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read_to_string(backups[0].path()).unwrap(), "{broken");
        assert_eq!(
            DeckStore::new(file).read().sessions,
            session(vec![("a", archived())])
        );
    }

    #[test]
    fn parse_keeps_valid_ui_prefs_and_drops_invalid_ones() {
        let state = parse_deck_state(
            &json!({ "version": 1, "sessions": {}, "ui": { "theme": "light", "sidebarPct": 5 } }).to_string(),
        )
        .unwrap();
        assert_eq!(
            state.ui,
            Some(UiPrefs {
                theme: Some(ThemePreference::Light),
                ..Default::default()
            })
        );
        let state =
            parse_deck_state(&json!({ "sessions": {}, "ui": { "theme": "neon" } }).to_string()).unwrap();
        assert_eq!(state.ui, None);
    }

    #[test]
    fn apply_ui_patch_merges_clears_and_drops_an_empty_ui_section() {
        let mut state = apply_ui_patch(
            &DeckStateFile::default(),
            &UiPatch {
                theme: Patch::Set(ThemePreference::Dark),
                sidebar_pct: Patch::Set(40.0),
                ..Default::default()
            },
        );
        assert_eq!(
            state.ui,
            Some(UiPrefs {
                theme: Some(ThemePreference::Dark),
                sidebar_pct: Some(40.0),
                ..Default::default()
            })
        );
        state = apply_ui_patch(
            &state,
            &UiPatch {
                sidebar_pct: Patch::Clear,
                ..Default::default()
            },
        );
        assert_eq!(
            state.ui,
            Some(UiPrefs {
                theme: Some(ThemePreference::Dark),
                ..Default::default()
            })
        );
        state = apply_ui_patch(
            &state,
            &UiPatch {
                theme: Patch::Clear,
                ..Default::default()
            },
        );
        assert_eq!(state.ui, None);
        state = apply_ui_patch(
            &state,
            &UiPatch {
                sidebar_pct: Patch::Set(99.0),
                ..Default::default()
            },
        );
        assert_eq!(state.ui, None);
    }

    #[test]
    fn active_agent_is_trimmed_dropped_when_blank_and_round_trips() {
        let state =
            parse_deck_state(&json!({ "sessions": {}, "ui": { "activeAgent": "codex" } }).to_string())
                .unwrap();
        assert_eq!(
            state.ui,
            Some(UiPrefs {
                active_agent: Some("codex".into()),
                ..Default::default()
            })
        );
        let state =
            parse_deck_state(&json!({ "sessions": {}, "ui": { "activeAgent": "   " } }).to_string()).unwrap();
        assert_eq!(state.ui, None);
        let state = apply_ui_patch(
            &DeckStateFile::default(),
            &UiPatch {
                theme: Patch::Set(ThemePreference::Dark),
                active_agent: Patch::Set("copilot".into()),
                ..Default::default()
            },
        );
        assert_eq!(
            state.ui,
            Some(UiPrefs {
                theme: Some(ThemePreference::Dark),
                active_agent: Some("copilot".into()),
                ..Default::default()
            })
        );
    }

    #[test]
    fn ui_and_sessions_survive_each_other_s_updates() {
        let (_dir, file) = temp_store();
        let store = DeckStore::new(file.clone());
        store
            .update_ui(&UiPatch {
                theme: Patch::Set(ThemePreference::Light),
                ..Default::default()
            })
            .unwrap();
        store
            .update_session(
                "a",
                &SessionPatch {
                    name: Patch::Set("A".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        store
            .update_ui(&UiPatch {
                sidebar_pct: Patch::Set(30.0),
                ..Default::default()
            })
            .unwrap();
        let state = DeckStore::new(file).read();
        assert_eq!(state.sessions, session(vec![("a", named("A"))]));
        assert_eq!(
            state.ui,
            Some(UiPrefs {
                theme: Some(ThemePreference::Light),
                sidebar_pct: Some(30.0),
                ..Default::default()
            })
        );
    }

    #[test]
    fn apply_hidden_project_patch_adds_and_removes_keys() {
        let mut state = apply_hidden_project_patch(&DeckStateFile::default(), "/a", true);
        assert_eq!(state.hidden_projects, vec!["/a"]);
        state = apply_hidden_project_patch(&state, "/b", true);
        assert_eq!(state.hidden_projects, vec!["/a", "/b"]);
        state = apply_hidden_project_patch(&state, "/a", false);
        assert_eq!(state.hidden_projects, vec!["/b"]);
        state = apply_hidden_project_patch(&state, "/b", false);
        assert!(state.hidden_projects.is_empty());
        assert!(!deck_state_to_json(&state).contains("hiddenProjects"));
    }

    #[test]
    fn parse_keeps_unique_valid_hidden_project_keys_and_drops_an_empty_list() {
        let state =
            parse_deck_state(&json!({ "sessions": {}, "hiddenProjects": ["/a", "/a", 5, "/b"] }).to_string())
                .unwrap();
        assert_eq!(state.hidden_projects, vec!["/a", "/b"]);
        let state = parse_deck_state(&json!({ "sessions": {}, "hiddenProjects": [] }).to_string()).unwrap();
        assert!(state.hidden_projects.is_empty());
    }

    #[test]
    fn set_project_hidden_persists_across_instances_and_can_be_undone() {
        let (_dir, file) = temp_store();
        let store = DeckStore::new(file.clone());
        store.set_project_hidden("/proj", true).unwrap();
        assert_eq!(DeckStore::new(file.clone()).get_hidden_projects(), vec!["/proj"]);
        store.set_project_hidden("/proj", false).unwrap();
        assert!(DeckStore::new(file).get_hidden_projects().is_empty());
    }

    #[test]
    fn apply_session_patch_trims_dedupes_and_drops_blank_tags() {
        let mut state = apply_session_patch(
            &DeckStateFile::default(),
            "s",
            &SessionPatch {
                tags: Patch::Set(
                    [" foo ", "foo", "bar", "  "]
                        .iter()
                        .map(|t| t.to_string())
                        .collect(),
                ),
                ..Default::default()
            },
        );
        assert_eq!(state.sessions["s"], tags(&["foo", "bar"]));
        state = apply_session_patch(
            &state,
            "s",
            &SessionPatch {
                tags: Patch::Set(vec![]),
                ..Default::default()
            },
        );
        assert!(!state.sessions.contains_key("s"));
    }

    #[test]
    fn parse_keeps_valid_tag_lists_and_drops_empty_or_invalid_ones() {
        let state = parse_deck_state(
            &json!({ "sessions": {
                "a": { "tags": ["foo", "foo", 5, " bar "] },
                "b": { "tags": [] },
                "c": { "tags": "nope" }
            } })
            .to_string(),
        )
        .unwrap();
        assert_eq!(state.sessions, session(vec![("a", tags(&["foo", "bar"]))]));
    }

    #[test]
    fn tags_persist_and_get_all_tags_is_distinct_and_alphabetical() {
        let (_dir, file) = temp_store();
        let store = DeckStore::new(file.clone());
        let set = |items: &[&str]| SessionPatch {
            tags: Patch::Set(items.iter().map(|t| t.to_string()).collect()),
            ..Default::default()
        };
        store.update_session("a", &set(&["zeta", "alpha"])).unwrap();
        store.update_session("b", &set(&["alpha", "beta"])).unwrap();
        assert_eq!(
            DeckStore::new(file.clone()).get_session("a").unwrap().tags,
            vec!["zeta", "alpha"]
        );
        assert_eq!(
            DeckStore::new(file.clone()).get_all_tags(),
            vec!["alpha", "beta", "zeta"]
        );
        store.update_session("a", &set(&[])).unwrap();
        assert!(DeckStore::new(file.clone()).get_session("a").is_none());
        assert_eq!(DeckStore::new(file).get_all_tags(), vec!["alpha", "beta"]);
    }

    #[test]
    fn unknown_fields_survive_a_rewrite() {
        let (_dir, file) = temp_store();
        fs::write(
            &file,
            json!({ "version": 1, "future": [1, 2], "sessions": { "a": { "name": "A", "color": "red" } },
                "ui": { "zoom": 2 } })
            .to_string(),
        )
        .unwrap();
        let store = DeckStore::new(file.clone());
        store
            .update_session(
                "b",
                &SessionPatch {
                    archived: Patch::Set(true),
                    ..Default::default()
                },
            )
            .unwrap();
        let out: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(out["future"], json!([1, 2]));
        assert_eq!(out["sessions"]["a"]["color"], json!("red"));
        assert_eq!(out["ui"]["zoom"], json!(2));
    }

    #[test]
    fn update_tree_applies_the_change_to_the_tree_on_disk_and_keeps_sessions() {
        let (_dir, file) = temp_store();
        let first = DeckStore::new(file.clone());
        let second = DeckStore::new(file.clone());
        first
            .update_session(
                "s1",
                &SessionPatch {
                    pin: Patch::Set(SessionPin::Top),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(second.get_tree(), default_tree_prefs());
        let mut folder_id = String::new();
        first
            .update_tree(|t| {
                let (tree, id) = create_folder(&t, "Work");
                folder_id = id;
                tree
            })
            .unwrap();
        let id = folder_id.clone();
        second
            .update_tree(|t| move_project_to_folder(&t, "p1", Some(&id)))
            .unwrap();
        let state = DeckStore::new(file.clone()).read();
        let folders = &state.tree.as_ref().unwrap().folders;
        assert_eq!(folders.len(), 1);
        assert_eq!(
            (folders[0].id.as_str(), folders[0].name.as_str()),
            (folder_id.as_str(), "Work")
        );
        assert_eq!(folders[0].projects, vec!["p1"]);
        assert_eq!(state.sessions["s1"].pin, Some(SessionPin::Top));
        second.update_tree(|t| delete_folder(&t, &folder_id)).unwrap();
        second
            .update_tree(|mut t| {
                t.root_order.clear();
                t
            })
            .unwrap();
        assert!(DeckStore::new(file).read().tree.is_none());
    }

    #[test]
    fn watch_sees_an_external_write() {
        let (_dir, file) = temp_store();
        let store = DeckStore::new(file.clone());
        let (tx, rx) = mpsc::channel();
        let _handle = store
            .watch(move || {
                let _ = tx.send(());
            })
            .unwrap();
        thread::sleep(Duration::from_millis(200));
        fs::write(&file, r#"{"version":1,"sessions":{}}"#).unwrap();
        assert!(rx.recv_timeout(Duration::from_secs(5)).is_ok());
    }

    #[test]
    fn watch_ignores_other_files_in_the_folder() {
        let (dir, file) = temp_store();
        let store = DeckStore::new(file);
        let (tx, rx) = mpsc::channel();
        let _handle = store
            .watch(move || {
                let _ = tx.send(());
            })
            .unwrap();
        thread::sleep(Duration::from_millis(200));
        fs::write(dir.path().join("other.txt"), "x").unwrap();
        assert!(rx.recv_timeout(Duration::from_millis(600)).is_err());
    }
}
