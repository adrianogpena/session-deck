//! Port of `sessions.ts`: the session model, discovery (Claude + Copilot, merged with the live
//! ones), and the status tracker that turns pid files and status files into what the UI shows.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::DateTime;
use sdeck_core::concurrency::map_with_concurrency;
use sdeck_core::discovery::claude_storage::{discover_claude_sessions, find_session_file, read_session_meta};
use sdeck_core::discovery::copilot_storage::list_copilot_sessions;
use sdeck_core::discovery::git_project::resolve_project_root;
use sdeck_core::discovery::path_utils::normalize_fs_path;
use sdeck_core::status::account::Account;
use sdeck_core::status::claude_process_watcher::{list_live_claude_processes, LiveClaudeProcess};
use sdeck_core::status::session_status::{
    is_session_status_unseen, read_session_status, session_status_dir, SessionStatus as CoreStatus,
};
use sdeck_core::store::deck_config::DeckConfig;
use sdeck_core::store::deck_store::DeckStore;
use sdeck_core::store::tree_prefs::SessionCategory;

use crate::ansi::one_line;
use crate::live_session::LiveSession;

const GIT_RESOLVE_CONCURRENCY: usize = 8;

/// The agent every session of the original TS UI is: `claude`, `copilot`, or a catalog agent id.
pub type AgentType = String;

/// What the status tracker reports for a session. `Done`: finished a turn the user hasn't seen yet.
/// `Error`: an error only visible on the screen (e.g. a failed sign-in). `Exited`: the agent process
/// ended with an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatus {
    Running,
    Waiting,
    Done,
    Idle,
    Starting,
    Error,
    Exited,
    Stopped,
}

/// A transcript read for the preview: not asked yet is `None` on the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LastResponse {
    Loading,
    Ready(String),
}

/// What discovery finds on disk. Plain data, so it crosses threads inside an `AppEvent`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundSession {
    pub agent: AgentType,
    pub id: String,
    pub file: Option<PathBuf>,
    pub cwd: String,
    pub project_root: String,
    pub project_key: String,
    pub title: String,
    pub mtime_ms: i64,
    /// The Claude account whose transcript this is; `None` for Copilot and other catalog agents.
    pub account: Option<Account>,
}

static NEXT_UID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
pub struct DeckSession {
    /// Identity across rebuilds and id changes (a brand-new session has no id yet).
    pub uid: u64,
    pub agent: AgentType,
    /// `None` until a brand-new Claude session's id shows up in its pid file.
    pub id: Option<String>,
    /// Copilot only: started with a pre-assigned id and not resumed yet.
    pub is_new: bool,
    pub file: Option<PathBuf>,
    pub cwd: String,
    /// Git root of `cwd` (worktrees and subfolders share one), and its `normalize_fs_path` key.
    pub project_root: String,
    pub project_key: String,
    pub title: String,
    pub mtime_ms: i64,
    pub live: Option<LiveSession>,
    pub last_response: Option<LastResponse>,
    pub title_refreshing: bool,
    /// A one-line prompt (`o`) to type once the agent is ready.
    pub pending_prompt: Option<String>,
    /// Found by discovery: stays listed when stopped.
    pub on_disk: bool,
    /// Set on a session just started via "new session" while its id isn't in its project's manual
    /// order yet: renders it at the top immediately (see `tree::sort_sessions_manual`).
    pub pending_top_order: bool,
    /// The Claude account that owns it; `None` for Copilot and catalog agents.
    pub account: Option<Account>,
}

impl DeckSession {
    /// A session with its project set to `cwd`; callers resolve the git root when they know it.
    pub fn new(agent: &str, id: Option<&str>, cwd: &str, title: &str, mtime_ms: i64) -> Self {
        DeckSession {
            uid: NEXT_UID.fetch_add(1, Ordering::Relaxed),
            agent: agent.to_string(),
            id: id.map(str::to_string),
            is_new: false,
            file: None,
            cwd: cwd.to_string(),
            project_root: cwd.to_string(),
            project_key: normalize_fs_path(cwd),
            title: title.to_string(),
            mtime_ms,
            live: None,
            last_response: None,
            title_refreshing: false,
            pending_prompt: None,
            on_disk: false,
            pending_top_order: false,
            account: None,
        }
    }

    pub fn from_found(f: FoundSession) -> Self {
        DeckSession {
            file: f.file,
            project_root: f.project_root,
            project_key: f.project_key,
            on_disk: true,
            account: f.account,
            ..DeckSession::new(&f.agent, Some(&f.id), &f.cwd, &f.title, f.mtime_ms)
        }
    }

    /// Whether a live PTY of this session is running here.
    pub fn is_live(&self) -> bool {
        self.live.as_ref().is_some_and(|l| !l.exited)
    }
}

/// A Session Deck name override (set in either front end) wins over the agent's own title.
pub fn display_title(s: &DeckSession, store: &DeckStore) -> String {
    s.id.as_deref()
        .and_then(|id| store.get_session(id))
        .and_then(|p| p.name)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| s.title.clone())
}

/// The most recent Claude and Copilot sessions (archived ones included, for the archived view), read
/// from disk. Blocking: runs on a background thread. `merge_found` folds the result into the live list.
pub fn discover_found(accounts: &[Account], config: &DeckConfig) -> Vec<FoundSession> {
    let max_sessions = config.ui.max_sessions_listed as usize;
    let enabled = |agent: &str| config.tools.get(agent).and_then(|t| t.enabled).unwrap_or(true);
    let mut found: Vec<FoundSession> = Vec::new();
    if enabled("claude") {
        found.extend(
            discover_claude_sessions(accounts, max_sessions)
                .into_iter()
                .map(|s| FoundSession {
                    agent: "claude".into(),
                    id: s.id,
                    file: Some(s.file),
                    project_root: s.cwd.clone(),
                    project_key: normalize_fs_path(&s.cwd),
                    cwd: s.cwd,
                    title: s.title,
                    mtime_ms: s.mtime_ms,
                    account: Some(s.account),
                }),
        );
    }
    if enabled("copilot") {
        found.extend(discover_copilot_found(max_sessions));
    }
    // A project is the git root of the session's cwd (cached per cwd by `resolve_project_root`).
    let roots = map_with_concurrency(&found, GIT_RESOLVE_CONCURRENCY, |s, _| {
        resolve_project_root(&s.cwd)
    });
    for (s, root) in found.iter_mut().zip(roots) {
        s.project_key = normalize_fs_path(&root.root);
        s.project_root = root.root;
    }
    found
}

/// Copilot's own session list, most recent first; sessions without a summary never got a turn.
fn discover_copilot_found(max_sessions: usize) -> Vec<FoundSession> {
    list_copilot_sessions()
        .into_iter()
        .filter(|r| r.summary.as_deref().is_some_and(|s| !s.is_empty()))
        .take(max_sessions)
        .map(|r| FoundSession {
            agent: "copilot".into(),
            id: r.id,
            file: None,
            project_root: r.cwd.clone(),
            project_key: normalize_fs_path(&r.cwd),
            cwd: r.cwd,
            title: one_line(r.summary.as_deref().unwrap_or("")),
            mtime_ms: DateTime::parse_from_rfc3339(&r.updated_at).map_or(0, |d| d.timestamp_millis()),
            account: None,
        })
        .collect()
}

/// Known sessions are updated in place, not replaced: the UI tracks the selection (and the previous
/// session) by `uid`, and live ones carry their PTY. Live sessions not found on disk (new, or older
/// than the cutoff) stay listed, before the found ones.
pub fn merge_found(current: Vec<DeckSession>, found: Vec<FoundSession>) -> Vec<DeckSession> {
    let index: HashMap<String, usize> = current
        .iter()
        .enumerate()
        .filter_map(|(i, s)| s.id.clone().map(|id| (id, i)))
        .collect();
    let mut slots: Vec<Option<DeckSession>> = current.into_iter().map(Some).collect();
    let mut merged = Vec::with_capacity(found.len());
    for f in found {
        let existing = index.get(&f.id).and_then(|&i| slots[i].take());
        merged.push(match existing {
            Some(mut s) => {
                if s.mtime_ms != f.mtime_ms && s.live.is_none() {
                    s.last_response = None; // transcript changed since it was loaded
                }
                s.title = f.title;
                s.file = f.file;
                s.mtime_ms = f.mtime_ms;
                s.cwd = f.cwd;
                s.project_root = f.project_root;
                s.project_key = f.project_key;
                s.account = f.account;
                s.on_disk = true;
                s
            }
            None => DeckSession::from_found(f),
        });
    }
    let mut out: Vec<DeckSession> = slots.into_iter().flatten().filter(|s| s.live.is_some()).collect();
    out.extend(merged);
    out
}

fn claude_status(status: &str) -> SessionStatus {
    match status {
        "busy" | "shell" => SessionStatus::Running,
        "waiting" => SessionStatus::Waiting,
        _ => SessionStatus::Idle,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unseen {
    Done,
    Error,
}

/// Status for every session, read once per poll rather than on every frame:
/// - Claude: every live `claude` process on the machine, from its own pid files.
/// - Copilot: Session Deck's status files, written by the Copilot watcher (this UI's or the extension's).
/// - Both: which sessions have an unseen "done"/"error" (status files + the shared seen marks).
#[derive(Debug, Default)]
pub struct StatusTracker {
    pub(crate) by_session: HashMap<String, LiveClaudeProcess>,
    pub(crate) by_pid: HashMap<u32, LiveClaudeProcess>,
    pub(crate) unseen: HashMap<String, Unseen>,
    pub(crate) written: HashMap<String, CoreStatus>,
}

impl StatusTracker {
    pub fn poll(&mut self, store: &DeckStore, accounts: &[Account]) {
        self.poll_unseen(store);
        self.poll_processes(accounts);
    }

    fn poll_unseen(&mut self, store: &DeckStore) {
        self.unseen.clear();
        self.written.clear();
        let Ok(entries) = std::fs::read_dir(session_status_dir()) else {
            return; // no status written yet
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(id) = name.strip_suffix(".json") else {
                continue;
            };
            let Some(record) = read_session_status(id) else {
                continue;
            };
            self.written.insert(id.to_string(), record.status);
            if matches!(record.status, CoreStatus::Done | CoreStatus::Error)
                && is_session_status_unseen(store, id)
            {
                let unseen = if record.status == CoreStatus::Done {
                    Unseen::Done
                } else {
                    Unseen::Error
                };
                self.unseen.insert(id.to_string(), unseen);
            }
        }
    }

    fn poll_processes(&mut self, accounts: &[Account]) {
        self.by_session.clear();
        self.by_pid.clear();
        for rec in list_live_claude_processes(accounts) {
            self.by_session.insert(rec.session_id.clone(), rec.clone());
            self.by_pid.insert(rec.pid, rec);
        }
    }

    pub fn for_pid(&self, pid: u32) -> Option<&LiveClaudeProcess> {
        self.by_pid.get(&pid)
    }

    pub fn status_of(&self, s: &DeckSession) -> SessionStatus {
        let unseen = s.id.as_deref().and_then(|id| self.unseen.get(id)).copied();
        // An idle agent whose last turn the user hasn't seen yet is "done", i.e. waiting for a look.
        let from_process = |rec: &LiveClaudeProcess| {
            let status = claude_status(&rec.status);
            if status == SessionStatus::Idle && unseen == Some(Unseen::Done) {
                SessionStatus::Done
            } else {
                status
            }
        };
        if s.agent != "claude" {
            // Copilot and every basic-tier catalog agent: no process registry of their own, so
            // status comes from the shared status file.
            return self.written_status_of(s, unseen);
        }
        if let Some(live) = &s.live {
            if live.exited {
                return SessionStatus::Exited;
            }
            if live.screen_error.is_some() {
                return SessionStatus::Error;
            }
            return self
                .by_pid
                .get(&live.pid)
                .map_or(SessionStatus::Starting, from_process);
        }
        let external = s.id.as_deref().and_then(|id| self.by_session.get(id));
        if let Some(rec) = external {
            return from_process(rec);
        }
        stopped_or_unseen(unseen)
    }

    /// Copilot and basic-tier catalog agents have no process registry: a live session here reads its
    /// status file; elsewhere can't be told apart from stopped.
    fn written_status_of(&self, s: &DeckSession, unseen: Option<Unseen>) -> SessionStatus {
        let Some(live) = &s.live else {
            return stopped_or_unseen(unseen);
        };
        if live.exited {
            return SessionStatus::Exited;
        }
        match s.id.as_deref().and_then(|id| self.written.get(id)) {
            Some(CoreStatus::Running) => return SessionStatus::Running,
            Some(CoreStatus::Waiting) => return SessionStatus::Waiting,
            _ => {}
        }
        match unseen {
            Some(Unseen::Done) => SessionStatus::Done,
            Some(Unseen::Error) => SessionStatus::Error,
            None => SessionStatus::Idle,
        }
    }

    /// Not running here, but a `claude` process elsewhere (another terminal, VS Code) has it open.
    /// Never true for Copilot (no way to tell).
    pub fn is_elsewhere(&self, s: &DeckSession) -> bool {
        s.agent == "claude"
            && s.live.is_none()
            && s.id.as_deref().is_some_and(|id| self.by_session.contains_key(id))
    }

    /// "done" counts as waiting (it's waiting for a look); a screen error or an error exit counts as error.
    pub fn category_of(&self, s: &DeckSession) -> SessionCategory {
        match self.status_of(s) {
            SessionStatus::Starting | SessionStatus::Running => SessionCategory::Running,
            SessionStatus::Done | SessionStatus::Waiting => SessionCategory::Waiting,
            SessionStatus::Idle => SessionCategory::Idle,
            SessionStatus::Exited | SessionStatus::Error => SessionCategory::Error,
            SessionStatus::Stopped => SessionCategory::Stopped,
        }
    }
}

fn stopped_or_unseen(unseen: Option<Unseen>) -> SessionStatus {
    match unseen {
        Some(Unseen::Done) => SessionStatus::Done,
        Some(Unseen::Error) => SessionStatus::Exited,
        None => SessionStatus::Stopped,
    }
}

/// Live sessions get re-titled as Claude updates its AI title. `read_session_meta` is mtime-cached,
/// so an unchanged transcript costs one stat. Returns whether the title changed.
pub fn refresh_live_title(s: &mut DeckSession, accounts: &[Account]) -> bool {
    if s.agent != "claude" || s.title_refreshing {
        return false;
    }
    let Some(id) = s.id.clone() else {
        return false;
    };
    if s.file.is_none() {
        match find_session_file(accounts, &id) {
            Some((_, file)) => s.file = Some(file),
            None => return false, // brand-new session, nothing written yet
        }
    }
    let Some(file) = s.file.clone() else {
        return false;
    };
    if let Some(mtime) = std::fs::metadata(&file)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
    {
        s.mtime_ms = mtime.as_millis() as i64; // for the "5m ago" column
    }
    let meta = read_session_meta(&file);
    let Some(title) = meta.title.or(meta.first_prompt) else {
        return false;
    };
    let title = one_line(&title);
    if title.is_empty() || title == s.title {
        return false;
    }
    s.title = title;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::EnvGuard;
    use std::fs;
    use std::path::Path;
    use std::time::{Duration, SystemTime};

    fn write_account(home: &Path, dir: &str, email: &str) {
        let config_file = if dir == ".claude" {
            home.join(".claude.json")
        } else {
            home.join(dir).join(".claude.json")
        };
        fs::create_dir_all(config_file.parent().unwrap()).unwrap();
        fs::write(
            config_file,
            format!(r#"{{"oauthAccount":{{"emailAddress":"{email}"}}}}"#),
        )
        .unwrap();
        fs::create_dir_all(home.join(dir).join("projects")).unwrap();
        fs::create_dir_all(home.join(dir).join("sessions")).unwrap();
    }

    fn write_transcript(home: &Path, dir: &str, project: &str, id: &str, prompt: &str, age_secs: u64) {
        let folder = home.join(dir).join("projects").join(project);
        fs::create_dir_all(&folder).unwrap();
        let file = folder.join(format!("{id}.jsonl"));
        let cwd = home.join("repos").join("api");
        let line = serde_json::json!({
            "type": "user", "cwd": cwd.to_string_lossy(),
            "message": {"role": "user", "content": prompt},
        });
        fs::write(&file, format!("{line}\n")).unwrap();
        let when = SystemTime::now() - Duration::from_secs(age_secs);
        fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(when)
            .unwrap();
    }

    fn fixture() -> (EnvGuard, tempfile::TempDir) {
        let guard = EnvGuard::new();
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("SDECK_USER_HOME", home.path());
        fs::create_dir_all(home.path().join("repos").join("api")).unwrap();
        write_account(home.path(), ".claude", "work@x.com");
        write_account(home.path(), ".claude-me", "me@x.com");
        (guard, home)
    }

    fn found_session(id: &str, mtime_ms: i64) -> FoundSession {
        FoundSession {
            agent: "claude".into(),
            id: id.into(),
            file: None,
            cwd: "C:\\repos\\api".into(),
            project_root: "C:\\repos\\api".into(),
            project_key: normalize_fs_path("C:\\repos\\api"),
            title: format!("title {id}"),
            mtime_ms,
            account: None,
        }
    }

    #[test]
    fn discovery_over_fixture_transcripts_finds_each_accounts_sessions() {
        let (_g, home) = fixture();
        write_transcript(home.path(), ".claude", "p1", "id-work", "work prompt", 100);
        write_transcript(home.path(), ".claude-me", "p1", "id-me", "me prompt", 50);
        let accounts = sdeck_core::status::account::discover_accounts();
        let mut config = DeckConfig::default();
        config.tools.get_mut("copilot").unwrap().enabled = Some(false);
        let found = discover_found(&accounts, &config);
        let ids: Vec<_> = found.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["id-me", "id-work"]);
        assert_eq!(
            found[0].account.as_ref().unwrap().email.as_deref(),
            Some("me@x.com")
        );
        assert_eq!(found[1].title, "work prompt");
        assert_eq!(found[0].project_key, found[1].project_key);
    }

    #[test]
    fn a_disabled_claude_tool_lists_no_claude_sessions() {
        let (_g, home) = fixture();
        write_transcript(home.path(), ".claude", "p1", "id-work", "work prompt", 100);
        let accounts = sdeck_core::status::account::discover_accounts();
        let mut config = DeckConfig::default();
        config.tools.get_mut("copilot").unwrap().enabled = Some(false);
        config.tools.get_mut("claude").unwrap().enabled = Some(false);
        assert!(discover_found(&accounts, &config).is_empty());
    }

    #[test]
    fn merge_updates_known_sessions_in_place_and_keeps_live_ones_not_on_disk() {
        let mut known = DeckSession::new("claude", Some("a"), "C:\\old", "old title", 1);
        known.last_response = Some(LastResponse::Ready("cached".into()));
        let known_uid = known.uid;
        let mut live_new = DeckSession::new("claude", None, "C:\\new", "(new session)", 5);
        live_new.live = Some(LiveSession::stub(1));
        let gone = DeckSession::new("claude", Some("gone"), "C:\\gone", "gone", 1);
        let merged = merge_found(
            vec![known, live_new, gone],
            vec![found_session("a", 99), found_session("b", 98)],
        );
        let ids: Vec<_> = merged.iter().map(|s| s.id.as_deref()).collect();
        assert_eq!(ids, [None, Some("a"), Some("b")]);
        let a = &merged[1];
        assert_eq!(a.uid, known_uid);
        assert_eq!(
            (a.title.as_str(), a.mtime_ms, a.cwd.as_str()),
            ("title a", 99, "C:\\repos\\api")
        );
        assert!(a.on_disk);
        assert_eq!(a.last_response, None);
    }

    #[test]
    fn merge_keeps_the_cached_response_of_a_live_session() {
        let mut live = DeckSession::new("claude", Some("a"), "C:\\repos\\api", "t", 1);
        live.live = Some(LiveSession::stub(1));
        live.last_response = Some(LastResponse::Ready("cached".into()));
        let merged = merge_found(vec![live], vec![found_session("a", 99)]);
        assert_eq!(
            merged[0].last_response,
            Some(LastResponse::Ready("cached".into()))
        );
    }

    fn account(dir: &str, is_default: bool) -> Account {
        Account {
            config_dir: PathBuf::from(dir),
            email: None,
            is_default,
        }
    }

    fn proc(pid: u32, session_id: &str, status: &str) -> LiveClaudeProcess {
        LiveClaudeProcess {
            pid,
            session_id: session_id.into(),
            status: status.into(),
            cwd: None,
            account: account("C:\\.claude", true),
        }
    }

    fn tracker(
        procs: Vec<LiveClaudeProcess>,
        unseen: &[(&str, Unseen)],
        written: &[(&str, CoreStatus)],
    ) -> StatusTracker {
        let mut t = StatusTracker::default();
        for p in procs {
            t.by_session.insert(p.session_id.clone(), p.clone());
            t.by_pid.insert(p.pid, p);
        }
        t.unseen = unseen.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        t.written = written.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        t
    }

    fn claude(id: &str) -> DeckSession {
        DeckSession::new("claude", Some(id), "C:\\r", "t", 1)
    }

    fn live(mut s: DeckSession, live: LiveSession) -> DeckSession {
        s.live = Some(live);
        s
    }

    #[test]
    fn claude_status_comes_from_pid_files_with_unseen_done_overriding_idle() {
        let t = tracker(
            vec![
                proc(1, "busy-one", "busy"),
                proc(2, "wait-one", "waiting"),
                proc(3, "idle-one", "idle"),
                proc(4, "done-one", "idle"),
            ],
            &[("done-one", Unseen::Done)],
            &[],
        );
        let status = |id: &str| t.status_of(&claude(id));
        assert_eq!(status("busy-one"), SessionStatus::Running);
        assert_eq!(status("wait-one"), SessionStatus::Waiting);
        assert_eq!(status("idle-one"), SessionStatus::Idle);
        assert_eq!(status("done-one"), SessionStatus::Done);
        assert_eq!(status("nobody"), SessionStatus::Stopped);
        assert!(t.is_elsewhere(&claude("busy-one")));
        assert!(!t.is_elsewhere(&claude("nobody")));
    }

    #[test]
    fn a_stopped_claude_session_with_an_unseen_result_reads_done_or_exited() {
        let t = tracker(vec![], &[("d", Unseen::Done), ("e", Unseen::Error)], &[]);
        assert_eq!(t.status_of(&claude("d")), SessionStatus::Done);
        assert_eq!(t.status_of(&claude("e")), SessionStatus::Exited);
    }

    #[test]
    fn a_live_claude_session_reads_its_own_pid_record() {
        let t = tracker(vec![proc(7, "x", "busy")], &[], &[]);
        let running = live(claude("x"), LiveSession::stub(7));
        let starting = live(claude("y"), LiveSession::stub(8));
        let mut exited = live(claude("x"), LiveSession::stub(7));
        exited.live.as_mut().unwrap().exited = true;
        let mut errored = live(claude("x"), LiveSession::stub(7));
        errored.live.as_mut().unwrap().screen_error = Some("sign-in failed".into());
        assert_eq!(t.status_of(&running), SessionStatus::Running);
        assert_eq!(t.status_of(&starting), SessionStatus::Starting);
        assert_eq!(t.status_of(&exited), SessionStatus::Exited);
        assert_eq!(t.status_of(&errored), SessionStatus::Error);
        assert!(!t.is_elsewhere(&running));
    }

    #[test]
    fn copilot_status_comes_from_its_status_file() {
        let t = tracker(
            vec![],
            &[("c-done", Unseen::Done)],
            &[("c-run", CoreStatus::Running), ("c-wait", CoreStatus::Waiting)],
        );
        let copilot = |id: &str, live_session: bool| {
            let s = DeckSession::new("copilot", Some(id), "C:\\r", "t", 1);
            if live_session {
                live(s, LiveSession::stub(9))
            } else {
                s
            }
        };
        assert_eq!(t.status_of(&copilot("c-run", true)), SessionStatus::Running);
        assert_eq!(t.status_of(&copilot("c-wait", true)), SessionStatus::Waiting);
        assert_eq!(t.status_of(&copilot("c-done", true)), SessionStatus::Done);
        assert_eq!(t.status_of(&copilot("other", true)), SessionStatus::Idle);
        assert_eq!(t.status_of(&copilot("c-run", false)), SessionStatus::Stopped);
        assert_eq!(t.status_of(&copilot("c-done", false)), SessionStatus::Done);
        assert!(!t.is_elsewhere(&copilot("c-run", false)));
    }

    #[test]
    fn categories_fold_starting_done_and_exited() {
        let t = tracker(
            vec![proc(1, "w", "waiting"), proc(2, "i", "idle")],
            &[("d", Unseen::Done), ("e", Unseen::Error)],
            &[],
        );
        let category = |s: &DeckSession| t.category_of(s);
        assert_eq!(
            category(&live(claude("z"), LiveSession::stub(99))),
            SessionCategory::Running
        );
        assert_eq!(category(&claude("d")), SessionCategory::Waiting);
        assert_eq!(category(&claude("e")), SessionCategory::Error);
        assert_eq!(category(&claude("w")), SessionCategory::Waiting);
        assert_eq!(category(&claude("i")), SessionCategory::Idle);
        assert_eq!(category(&claude("none")), SessionCategory::Stopped);
    }

    #[test]
    fn poll_reads_pid_files_and_status_files_from_disk() {
        let (_g, home) = fixture();
        let status_dir = session_status_dir();
        fs::create_dir_all(&status_dir).unwrap();
        fs::write(status_dir.join("fin.json"), r#"{"status":"done","updatedAt":5}"#).unwrap();
        fs::write(
            status_dir.join("cop.json"),
            r#"{"status":"running","updatedAt":6}"#,
        )
        .unwrap();
        let pid = std::process::id();
        fs::write(
            home.path()
                .join(".claude-me")
                .join("sessions")
                .join(format!("{pid}.json")),
            format!(r#"{{"pid":{pid},"sessionId":"elsewhere","status":"busy"}}"#),
        )
        .unwrap();
        let store = DeckStore::new(home.path().join("state.json"));
        let mut t = StatusTracker::default();
        t.poll(&store, &sdeck_core::status::account::discover_accounts());
        assert_eq!(t.status_of(&claude("fin")), SessionStatus::Done);
        assert_eq!(t.written.get("cop"), Some(&CoreStatus::Running));
        assert_eq!(t.status_of(&claude("elsewhere")), SessionStatus::Running);
        assert_eq!(
            t.by_session["elsewhere"].account.email.as_deref(),
            Some("me@x.com")
        );
        // Seen once acknowledged: the result stops asking for attention.
        sdeck_core::status::session_status::acknowledge_session_status(&store, "fin").unwrap();
        t.poll(&store, &[]);
        assert_eq!(t.status_of(&claude("fin")), SessionStatus::Stopped);
        assert_eq!(t.status_of(&claude("elsewhere")), SessionStatus::Stopped);
    }

    #[test]
    fn a_name_override_wins_over_the_agent_title() {
        let dir = tempfile::tempdir().unwrap();
        let store = DeckStore::new(dir.path().join("state.json"));
        let s = claude("named");
        assert_eq!(display_title(&s, &store), "t");
        store
            .update_session(
                "named",
                &sdeck_core::store::deck_store::SessionPatch {
                    name: sdeck_core::store::deck_store::Patch::Set("Mine".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(display_title(&s, &store), "Mine");
        assert_eq!(
            display_title(&DeckSession::new("claude", None, "C:\\r", "fresh", 1), &store),
            "fresh"
        );
    }

    #[test]
    fn a_live_title_follows_the_transcripts_ai_title() {
        let (_g, home) = fixture();
        write_transcript(home.path(), ".claude", "p1", "live-id", "first prompt", 10);
        let file = home.path().join(".claude/projects/p1/live-id.jsonl");
        let accounts = sdeck_core::status::account::discover_accounts();
        let mut s = claude("live-id");
        s.title = "(new session)".into();
        assert!(refresh_live_title(&mut s, &accounts));
        assert_eq!(s.title, "first prompt");
        assert_eq!(s.file.as_deref(), Some(file.as_path()));
        assert!(!refresh_live_title(&mut s, &accounts));
        let mut unknown = claude("nowhere");
        assert!(!refresh_live_title(&mut unknown, &accounts));
        assert!(!refresh_live_title(
            &mut DeckSession::new("claude", None, "C:\\r", "t", 1),
            &accounts
        ));
    }
}
