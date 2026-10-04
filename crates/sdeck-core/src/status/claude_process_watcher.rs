use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use super::account::Account;
use super::process::is_process_alive;
use super::session_status::{clear_session_status, read_session_status, write_session_status, SessionStatus};
use super::watch::{watch_path, PathWatch};

/// `busy`/`shell` both map to "running". `waiting` means "needs your input". `idle` is the resting
/// state once a turn completes: "done". Anything else is ignored.
pub fn classify_claude_process_status(status: &str) -> Option<SessionStatus> {
    match status {
        "busy" | "shell" => Some(SessionStatus::Running),
        "waiting" => Some(SessionStatus::Waiting),
        "idle" => Some(SessionStatus::Done),
        _ => None,
    }
}

/// Claude Code's own live per-process file: one `<pid>.json` per running `claude`, with
/// `sessionId`/`status`. Undocumented format, so best-effort.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ClaudeProcessFile {
    pid: u32,
    session_id: String,
    status: String,
    cwd: Option<String>,
}

fn parse_claude_process_file(raw: &str) -> Option<ClaudeProcessFile> {
    // A partial write mid-save fails to parse and is skipped.
    let parsed: Value = serde_json::from_str(raw).ok()?;
    let pid = parsed.get("pid")?.as_f64()?;
    if !(0.0..=f64::from(u32::MAX)).contains(&pid) {
        return None;
    }
    Some(ClaudeProcessFile {
        pid: pid as u32,
        session_id: parsed.get("sessionId")?.as_str()?.to_string(),
        status: parsed.get("status")?.as_str()?.to_string(),
        cwd: parsed.get("cwd").and_then(Value::as_str).map(str::to_string),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveClaudeProcess {
    pub pid: u32,
    pub session_id: String,
    /// Claude's own value: `busy`, `waiting`, `idle`, `shell`...
    pub status: String,
    /// The cwd Claude started in; `None` only for a hand-crafted or corrupted pid file.
    pub cwd: Option<String>,
    /// The account whose `sessions/` dir holds the pid file.
    pub account: Account,
}

fn read_process_files(account: &Account) -> Vec<ClaudeProcessFile> {
    let Ok(entries) = fs::read_dir(account.sessions_dir()) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".json"))
        // Deleted between read_dir and read: a harmless race.
        .filter_map(|e| fs::read_to_string(e.path()).ok())
        .filter_map(|raw| parse_claude_process_file(&raw))
        .collect()
}

/// Every `claude` process running right now, on any terminal and under any account (from its pid
/// file, checked for liveness).
pub fn list_live_claude_processes(accounts: &[Account]) -> Vec<LiveClaudeProcess> {
    accounts
        .iter()
        .flat_map(|account| {
            read_process_files(account)
                .into_iter()
                .filter(|p| is_process_alive(p.pid))
                .map(move |p| LiveClaudeProcess {
                    pid: p.pid,
                    session_id: p.session_id,
                    status: p.status,
                    cwd: p.cwd,
                    account: account.clone(),
                })
        })
        .collect()
}

#[derive(Default)]
struct State {
    last_written: HashMap<String, SessionStatus>,
    /// Session ids backed by a live pid file at the last scan: one disappearing entirely between
    /// scans means its process ended.
    known_session_ids: HashSet<String>,
}

struct Inner {
    accounts: Vec<Account>,
    log: Box<dyn Fn(&str) + Send + Sync>,
    state: Mutex<State>,
}

/// Live status for Claude Code sessions, zero setup: one watcher covering every running `claude`
/// process of every account. Only writes on an actual status *change* (rewriting an unchanged status
/// would defeat "done"'s acknowledge-by-timestamp matching). A session's first observed status is a
/// baseline, never displayed: a freshly resumed process starts `idle`.
pub struct ClaudeProcessWatcher {
    inner: Arc<Inner>,
    _watches: Vec<PathWatch>,
}

impl ClaudeProcessWatcher {
    /// Scans once, then rescans whenever a `sessions/` dir changes.
    pub fn start(accounts: Vec<Account>, log: impl Fn(&str) + Send + Sync + 'static) -> Self {
        let inner = Arc::new(Inner {
            accounts,
            log: Box::new(log),
            state: Mutex::new(State::default()),
        });
        let mut watches = Vec::new();
        for account in &inner.accounts {
            let dir = account.sessions_dir();
            if let Err(e) = fs::create_dir_all(&dir) {
                (inner.log)(&format!(
                    "[claude-status] Failed to ensure {} exists: {e}",
                    dir.display()
                ));
                continue;
            }
            let for_scan = Arc::clone(&inner);
            match watch_path(&dir, move || for_scan.scan()) {
                Ok(w) => watches.push(w),
                Err(e) => (inner.log)(&format!("[claude-status] Failed to watch {}: {e}", dir.display())),
            }
        }
        inner.scan();
        ClaudeProcessWatcher {
            inner,
            _watches: watches,
        }
    }

    /// One pass over every account's pid files; also what the directory watch triggers.
    pub fn scan(&self) {
        self.inner.scan();
    }
}

impl Inner {
    fn scan(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut current: HashSet<String> = HashSet::new();
        for account in &self.accounts {
            if !Path::new(&account.sessions_dir()).is_dir() {
                continue;
            }
            for parsed in read_process_files(account) {
                current.insert(parsed.session_id.clone());
                if !is_process_alive(parsed.pid) {
                    Self::clear(&mut state, &parsed.session_id);
                    continue;
                }
                let Some(status) = classify_claude_process_status(&parsed.status) else {
                    continue;
                };
                match state.last_written.get(&parsed.session_id) {
                    // First sight is a baseline, not an event.
                    None => {
                        state.last_written.insert(parsed.session_id, status);
                    }
                    Some(&last) if last != status => {
                        // The other sdeck's watcher may have written this same transition already;
                        // writing it again would give it a new updatedAt and un-see a "done".
                        let already = read_session_status(&parsed.session_id).map(|r| r.status);
                        if already != Some(status) {
                            if let Err(e) = write_session_status(&parsed.session_id, status) {
                                (self.log)(&format!("[claude-status] Failed to write status: {e}"));
                            }
                        }
                        state.last_written.insert(parsed.session_id, status);
                    }
                    Some(_) => {}
                }
            }
        }
        let ended: Vec<String> = state.known_session_ids.difference(&current).cloned().collect();
        for session_id in ended {
            Self::clear(&mut state, &session_id);
        }
        state.known_session_ids = current;
    }

    fn clear(state: &mut State, session_id: &str) {
        clear_session_status(session_id);
        state.last_written.remove(session_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_env::EnvGuard;
    use crate::status::account::discover_accounts;
    use std::process::{Command, Stdio};

    #[test]
    fn classify_maps_busy_and_shell_to_running() {
        assert_eq!(
            classify_claude_process_status("busy"),
            Some(SessionStatus::Running)
        );
        assert_eq!(
            classify_claude_process_status("shell"),
            Some(SessionStatus::Running)
        );
    }

    #[test]
    fn classify_maps_waiting_and_idle() {
        assert_eq!(
            classify_claude_process_status("waiting"),
            Some(SessionStatus::Waiting)
        );
        assert_eq!(classify_claude_process_status("idle"), Some(SessionStatus::Done));
    }

    #[test]
    fn classify_ignores_unknown_statuses() {
        assert_eq!(classify_claude_process_status("gone"), None);
        assert_eq!(
            classify_claude_process_status("something-future-versions-might-add"),
            None
        );
    }

    #[test]
    fn parse_requires_pid_session_id_and_status() {
        let ok =
            parse_claude_process_file(r#"{"pid":7,"sessionId":"s","status":"busy","cwd":"C:\\p"}"#).unwrap();
        assert_eq!((ok.pid, ok.cwd.as_deref()), (7, Some("C:\\p")));
        assert_eq!(
            parse_claude_process_file(r#"{"pid":"7","sessionId":"s","status":"busy"}"#),
            None
        );
        assert_eq!(parse_claude_process_file(r#"{"pid":7,"status":"busy"}"#), None);
        assert_eq!(
            parse_claude_process_file(r#"{"pid":7,"sessionId":"s","sta"#),
            None
        );
    }

    struct Fixture {
        _g: EnvGuard,
        home: tempfile::TempDir,
        accounts: Vec<Account>,
    }

    fn fixture() -> Fixture {
        let g = EnvGuard::new();
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("SDECK_USER_HOME", home.path());
        std::env::set_var("SESSION_DECK_STATUS_DIR", home.path().join("status"));
        let login = r#"{"oauthAccount":{"emailAddress":"me@x.com"}}"#;
        fs::create_dir_all(home.path().join(".claude/sessions")).unwrap();
        fs::create_dir_all(home.path().join(".claude-me/sessions")).unwrap();
        fs::write(home.path().join(".claude-me/.claude.json"), login).unwrap();
        let accounts = discover_accounts();
        Fixture {
            _g: g,
            home,
            accounts,
        }
    }

    fn put_pid_file(dir: &Path, pid: u32, session_id: &str, status: &str) {
        let body = format!(r#"{{"pid":{pid},"sessionId":"{session_id}","status":"{status}","cwd":"C:\\p"}}"#);
        fs::write(dir.join(format!("{pid}.json")), body).unwrap();
    }

    fn dead_pid() -> u32 {
        let mut child = Command::new("git")
            .arg("--version")
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        child.wait().unwrap();
        pid
    }

    #[test]
    fn lists_live_processes_per_account_and_skips_dead_pids() {
        let f = fixture();
        let me = std::process::id();
        put_pid_file(
            &f.home.path().join(".claude/sessions"),
            me,
            "live-default",
            "busy",
        );
        put_pid_file(
            &f.home.path().join(".claude-me/sessions"),
            me + 1_000_000,
            "x",
            "busy",
        );
        put_pid_file(
            &f.home.path().join(".claude-me/sessions"),
            dead_pid(),
            "dead",
            "busy",
        );
        fs::write(f.home.path().join(".claude-me/sessions/9.json"), "{broken").unwrap();
        // The same live pid under the second account too.
        fs::write(
            f.home.path().join(".claude-me/sessions/live.json"),
            format!(r#"{{"pid":{me},"sessionId":"live-me","status":"idle"}}"#),
        )
        .unwrap();

        let mut live = list_live_claude_processes(&f.accounts);
        live.sort_by(|a, b| a.session_id.cmp(&b.session_id));
        let found: Vec<_> = live
            .iter()
            .map(|p| (p.session_id.as_str(), p.account.is_default))
            .collect();
        assert_eq!(found, [("live-default", true), ("live-me", false)]);
        assert_eq!(live[1].account.email.as_deref(), Some("me@x.com"));
        assert_eq!(live[1].cwd, None);
    }

    #[test]
    fn scan_baselines_then_writes_changes_and_clears_ended_sessions() {
        let f = fixture();
        let dir = f.home.path().join(".claude-me/sessions");
        let me = std::process::id();
        let watcher = ClaudeProcessWatcher::start(f.accounts.clone(), |_| {});

        put_pid_file(&dir, me, "sess-1", "idle");
        watcher.scan();
        assert_eq!(read_session_status("sess-1"), None, "first sight is a baseline");

        put_pid_file(&dir, me, "sess-1", "busy");
        watcher.scan();
        assert_eq!(
            read_session_status("sess-1").map(|r| r.status),
            Some(SessionStatus::Running)
        );

        let stamp = read_session_status("sess-1").unwrap().updated_at;
        watcher.scan();
        assert_eq!(
            read_session_status("sess-1").unwrap().updated_at,
            stamp,
            "unchanged: no rewrite"
        );

        fs::remove_file(dir.join(format!("{me}.json"))).unwrap();
        watcher.scan();
        assert_eq!(read_session_status("sess-1"), None, "ended session is cleared");
    }

    #[test]
    fn scan_clears_the_status_of_a_dead_pid() {
        let f = fixture();
        let dir = f.home.path().join(".claude/sessions");
        write_session_status("sess-2", SessionStatus::Running).unwrap();
        put_pid_file(&dir, dead_pid(), "sess-2", "busy");
        let watcher = ClaudeProcessWatcher::start(f.accounts.clone(), |_| {});
        watcher.scan();
        assert_eq!(read_session_status("sess-2"), None);
    }
}
