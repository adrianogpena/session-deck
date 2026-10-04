//! Live sessions: start (`s`), stop (`x`), restart (`R`), PTY events, and what the 1 s process poll
//! does for them (adopting a new session's id, a queued prompt, seen marks, toasts, the window
//! title). Port of `App.start`, `kill`, `ensureLive`, `restart`, `pollProcs`, `sendPendingPrompt`,
//! `markSeen`, `notifyChanges`, `updateTitle` and `resizeAllToPane`.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use sdeck_core::agent_catalog::{agent_display_name, is_builtin_agent};
use sdeck_core::status::session_status::{
    acknowledge_session_status, clear_session_status, write_session_status, SessionStatus as CoreStatus,
};
use sdeck_core::status::waiting_notifier::{CheckOptions, OnActivated, WatchedSession};
use sdeck_core::store::tree_prefs::{prepend_session, rename_session_id, TreePrefs};

use super::App;
use crate::event::AppEvent;
use crate::filters::StatusCategory;
use crate::layout::pty_size_for;
use crate::live_session::{agent_args, agent_env, LiveSession, SpawnRequest};
use crate::sessions::{display_title, DeckSession, SessionStatus};
use crate::tree::{project_labels, TreeRow};

/// A queued prompt is typed only once the agent's output has been quiet this long.
const PROMPT_SETTLE: Duration = Duration::from_millis(1000);

impl App {
    pub(super) fn session_mut(&mut self, uid: u64) -> Option<&mut DeckSession> {
        self.sessions.iter_mut().find(|s| s.uid == uid)
    }

    /// PTY size for background agents: the preview's body. In list-only layout there's no preview,
    /// so agents keep their size.
    pub(super) fn pty_size(&self) -> Option<(u16, u16)> {
        pty_size_for(&self.layout())
    }

    /// Starts the session's agent in a background PTY. A resumed Claude session runs as its own
    /// account, a new one as the active account. `false` if it couldn't start (already flashed why).
    pub(super) fn start(&mut self, uid: u64, now: Instant) -> bool {
        let Some(s) = self.session_by_uid(uid) else {
            return false;
        };
        if !Path::new(&s.cwd).exists() {
            let text = format!("Folder no longer exists: {}", s.cwd);
            self.flash(text, now);
            return false;
        }
        let (agent, session_id, is_new, cwd) = (s.agent.clone(), s.id.clone(), s.is_new, s.cwd.clone());
        let account = if agent == "claude" {
            s.id.as_ref()
                .and(s.account.clone())
                .or_else(|| self.active_account().cloned())
        } else {
            None
        };
        let (cols, rows) = self.pty_size().unwrap_or((80, 24));
        let launch = self.executables.resolve(&agent, &self.config);
        let request = SpawnRequest {
            launch: &launch,
            args: agent_args(&agent, session_id.as_deref(), is_new, &self.config),
            cwd: &cwd,
            env: agent_env(account.as_ref()),
            cols,
            rows,
        };
        let live = match LiveSession::spawn(request, &self.tx) {
            Ok(live) => live,
            Err(e) => {
                // Most often a catalog agent that isn't installed.
                let text = format!("Could not start {}: {e}", agent_display_name(&agent));
                self.flash(text, now);
                return false;
            }
        };
        if let Some(id) = &session_id {
            if agent == "copilot" {
                self.copilot_watcher.start(id);
            } else if !is_builtin_agent(&agent) {
                // No status of its own to tail: liveness is all there is, read back while the PTY lives.
                let _ = write_session_status(id, CoreStatus::Running);
            }
        }
        if let Some(s) = self.session_mut(uid) {
            s.live = Some(live);
            s.is_new = false; // from now on it resumes its own id
            if s.account.is_none() {
                s.account = account;
            }
        }
        self.rebuild_rows();
        true
    }

    /// Ends the session's PTY (if any) and its status tracking; the session stays listed.
    fn clear_live(&mut self, uid: u64) {
        let Some(s) = self.session_mut(uid) else {
            return;
        };
        if let Some(mut live) = s.live.take() {
            live.dispose();
        }
        let (agent, id) = (s.agent.clone(), s.id.clone());
        if let Some(id) = id {
            if agent == "copilot" {
                self.copilot_watcher.stop(&id);
            } else if !is_builtin_agent(&agent) {
                clear_session_status(&id);
            }
        }
    }

    /// Stops the session. It stays listed only if it's resumable: found on disk, or its transcript
    /// exists by now. A new session with nothing sent has neither.
    pub(super) fn kill(&mut self, uid: u64) {
        self.clear_live(uid);
        let resumable = self
            .session_by_uid(uid)
            .is_some_and(|s| s.id.is_some() && (s.on_disk || s.file.as_ref().is_some_and(|f| f.exists())));
        if !resumable {
            self.sessions.retain(|s| s.uid != uid);
        }
        self.rebuild_rows();
    }

    /// Refuses a session open elsewhere and starts a stopped one. `false` if it couldn't be readied
    /// (already flashed why); a brand-new session that never got to run is dropped from the list.
    #[allow(dead_code)] // attach and interact (09)
    pub(super) fn ensure_live(&mut self, uid: u64, now: Instant) -> bool {
        let Some(s) = self.session_by_uid(uid) else {
            return false;
        };
        if self.procs.is_elsewhere(s) {
            self.flash(
                "That session is running in another terminal. Close it there first.".into(),
                now,
            );
            return false;
        }
        if s.is_live() {
            return true;
        }
        self.clear_live(uid);
        if self.start(uid, now) {
            return true;
        }
        let resumable = self
            .session_by_uid(uid)
            .is_some_and(|s| s.on_disk || s.file.as_ref().is_some_and(|f| f.exists()));
        if !resumable {
            self.sessions.retain(|s| s.uid != uid);
            self.rebuild_rows();
        }
        false
    }

    /// `s`: starts the selected session in the background.
    pub(super) fn start_selected(&mut self, now: Instant) {
        let Some(s) = self.selected_session() else {
            return;
        };
        let uid = s.uid;
        if self.procs.is_elsewhere(s) {
            self.flash("That session is running in another terminal.".into(), now);
        } else if !s.is_live() {
            self.clear_live(uid);
            if self.start(uid, now) {
                self.flash("Started in background".into(), now);
            }
        }
    }

    /// `x`: stops the selected session.
    pub(super) fn stop_selected(&mut self, now: Instant) {
        if let Some(uid) = self
            .selected_session()
            .filter(|s| s.live.is_some())
            .map(|s| s.uid)
        {
            self.kill(uid);
            self.flash("Session stopped".into(), now);
        }
    }

    /// `R`: a fresh agent process on the same conversation (e.g. to pick up changed settings).
    pub(super) fn restart(&mut self, now: Instant) {
        let Some(s) = self.selected_session() else {
            self.flash("Select a session to restart.".into(), now);
            return;
        };
        if self.procs.is_elsewhere(s) {
            self.flash("That session is running in another terminal.".into(), now);
            return;
        }
        let uid = s.uid;
        self.clear_live(uid);
        if let Some(s) = self.session_mut(uid) {
            if s.agent == "claude" && s.id.is_some() && !s.file.as_ref().is_some_and(|f| f.exists()) {
                s.id = None; // nothing was sent yet: no conversation to resume, start a new one
            }
        }
        if self.start(uid, now) {
            self.flash("Restarted".into(), now);
        }
    }

    fn live_mut(&mut self, live_id: u64) -> Option<&mut DeckSession> {
        self.sessions
            .iter_mut()
            .find(|s| s.live.as_ref().is_some_and(|l| l.id == live_id))
    }

    pub(super) fn on_pty_output(&mut self, live_id: u64, data: &[u8], now: Instant) {
        let selected = self.selected_session().map(|s| s.uid);
        if let Some(s) = self.live_mut(live_id) {
            if let Some(live) = s.live.as_mut() {
                live.feed(data, false, now);
            }
            if Some(s.uid) == selected {
                self.dirty = true;
            }
        }
    }

    pub(super) fn on_pty_exit(&mut self, live_id: u64, code: u32) {
        let Some(s) = self.live_mut(live_id) else {
            return;
        };
        if let Some(live) = s.live.as_mut() {
            live.on_exit(code);
        }
        let (agent, id) = (s.agent.clone(), s.id.clone());
        if let Some(id) = id {
            if agent == "copilot" {
                self.copilot_watcher.stop(&id);
            } else if !is_builtin_agent(&agent) {
                clear_session_status(&id);
            }
        }
        self.rebuild_rows();
        self.dirty = true;
    }

    /// Time-based work of every live session (screen-error checks, a delayed Enter).
    pub(super) fn tick_live(&mut self, now: Instant) {
        for s in &mut self.sessions {
            if let Some(live) = s.live.as_mut() {
                if live.on_tick(now) {
                    self.dirty = true;
                }
            }
        }
    }

    /// Fits every background agent to the preview's body.
    pub(super) fn resize_all_to_pane(&mut self) {
        let Some((cols, rows)) = self.pty_size() else {
            return;
        };
        for s in &mut self.sessions {
            if let Some(live) = s.live.as_mut() {
                live.resize(cols, rows);
            }
        }
    }

    /// A brand-new session (or one that ran `/clear`) learns its id only from Claude's own pid file.
    pub(super) fn adopt_live_ids(&mut self) {
        let recent_first = self.config.ui.recent_sessions_first;
        let mut tree_changes: Vec<Box<dyn Fn(TreePrefs) -> TreePrefs>> = Vec::new();
        for s in &mut self.sessions {
            let Some(rec) = s
                .live
                .as_ref()
                .filter(|l| !l.exited)
                .and_then(|l| self.procs.for_pid(l.pid))
            else {
                continue;
            };
            if s.id.as_deref() == Some(rec.session_id.as_str()) {
                continue;
            }
            let new_id = rec.session_id.clone();
            let old_id = s.id.replace(new_id.clone());
            s.file = None;
            let key = s.project_key.clone();
            if s.pending_top_order {
                s.pending_top_order = false;
                if !recent_first {
                    tree_changes.push(Box::new(move |t| prepend_session(&t, &key, &new_id)));
                }
            } else if let Some(old_id) = old_id {
                // /clear: same terminal, new id, and the old title no longer describes the (now
                // empty) conversation.
                s.title = if s.agent == "copilot" {
                    "(new Copilot session)".into()
                } else {
                    "(new session)".into()
                };
                if !recent_first {
                    // Keeps its manual position instead of falling to the back.
                    tree_changes.push(Box::new(move |t| {
                        let renamed = rename_session_id(&t, &key, &old_id, &new_id);
                        if renamed != t {
                            renamed
                        } else {
                            prepend_session(&t, &key, &new_id)
                        }
                    }));
                }
            }
        }
        for change in tree_changes {
            self.tree = change(self.tree.clone());
            let _ = self.store.update_tree(&change);
        }
    }

    /// Types a queued prompt once the agent is ready for input: idle (or just done), and quiet for a
    /// moment so a freshly started agent has drawn its input box. Never into a "waiting" prompt.
    pub(super) fn send_pending_prompt(&mut self, uid: u64, now: Instant) {
        let Some(s) = self.session_by_uid(uid) else {
            return;
        };
        let status = self.procs.status_of(s);
        let ready = s.pending_prompt.is_some()
            && s.live.as_ref().is_some_and(|l| {
                !l.exited && now.saturating_duration_since(l.last_output_at) >= PROMPT_SETTLE
            })
            && matches!(status, SessionStatus::Idle | SessionStatus::Done);
        if !ready {
            return;
        }
        if let Some(s) = self.session_mut(uid) {
            if let (Some(text), Some(live)) = (s.pending_prompt.take(), s.live.as_mut()) {
                live.type_line(&text, now);
            }
        }
        self.mark_seen(uid);
    }

    /// Attaching, detaching or prompting counts as seeing the session: its "done"/"error" stops
    /// asking for attention, in both front ends.
    pub(super) fn mark_seen(&mut self, uid: u64) {
        let Some(id) = self.session_by_uid(uid).and_then(|s| s.id.clone()) else {
            return;
        };
        let _ = acknowledge_session_status(&self.store, &id);
        self.procs.poll(&self.store, &self.accounts);
        self.rebuild_rows();
        self.dirty = true;
    }

    /// Toasts for watched sessions (live here or elsewhere, every account's, shown in the tree or
    /// not) that need you; a click selects the session here.
    pub(super) fn notify_changes(&mut self) {
        if !self.config.ui.notifications {
            return;
        }
        let watched_sessions: Vec<&DeckSession> = self
            .sessions
            .iter()
            .filter(|s| s.id.is_some() && (s.is_live() || self.procs.is_elsewhere(s)))
            .collect();
        let roots: Vec<String> = watched_sessions.iter().map(|s| s.project_root.clone()).collect();
        let labels = project_labels(&roots);
        let watched: Vec<WatchedSession> = watched_sessions
            .iter()
            .map(|s| WatchedSession {
                session_id: s.id.clone().unwrap_or_default(),
                label: display_title(s, &self.store),
                project: labels.get(&s.project_root).cloned(),
            })
            .collect();
        let Some(notifier) = self.notifier.as_mut() else {
            return;
        };
        let tx = self.tx.clone();
        let on_activated: OnActivated = Arc::new(move |id: &str| {
            let _ = tx.send(AppEvent::ToastClicked(id.to_string()));
        });
        // 09 skips the attached/interacting session here.
        let skip = |_: &str| false;
        let options = CheckOptions {
            statuses: &self.config.ui.notify_statuses,
            skip: &skip,
            focused: self.focused,
        };
        notifier.check(&self.store, &watched, &options, &on_activated);
    }

    /// A toast was clicked: select its session (the terminal can't be brought to the front).
    pub(super) fn on_toast_clicked(&mut self, id: &str, now: Instant) {
        let Some(uid) = self
            .sessions
            .iter()
            .find(|s| s.id.as_deref() == Some(id))
            .map(|s| s.uid)
        else {
            return;
        };
        let selected = self.select_where(|r| matches!(r, TreeRow::Session { uid: u, .. } if *u == uid));
        let other_account_hidden = !self.config.accounts.show_all_sessions
            && self.session_by_uid(uid).is_some_and(|s| {
                s.account.as_ref().map(|a| &a.config_dir) != self.active_account().map(|a| &a.config_dir)
            });
        if !selected && other_account_hidden {
            self.flash(
                "That session belongs to another account, hidden from the list.".into(),
                now,
            );
        }
        self.dirty = true;
    }

    /// "Session Deck · ◐ 2 need you" in the terminal's title, visible from other windows.
    pub(super) fn update_title(&mut self) {
        let hidden = self.hidden_projects();
        let waiting = self.counts(&hidden).0.get(StatusCategory::Waiting);
        let title = match waiting {
            0 => "Session Deck".to_string(),
            1 => "Session Deck · ◐ 1 needs you".to_string(),
            n => format!("Session Deck · ◐ {n} need you"),
        };
        if title != self.last_title {
            self.pending_output.push_str(&format!("\x1b]0;{title}\x07"));
            self.last_title = title;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::mpsc::{self, Receiver};
    use std::sync::Mutex;

    use sdeck_core::status::session_status::session_status_dir;
    use sdeck_core::status::waiting_notifier::{Toast, ToastSender, WaitingNotifier};
    use sdeck_core::store::deck_config::DeckConfig;
    use sdeck_core::store::deck_store::DeckStore;

    use super::*;
    use crate::live_session::tests::fake_agent;
    use crate::test_support::EnvGuard;

    struct Fixture {
        app: App,
        rx: Receiver<AppEvent>,
        toasts: Arc<Mutex<Vec<Toast>>>,
        home: tempfile::TempDir,
        _guard: EnvGuard,
    }

    struct FakeToasts(Arc<Mutex<Vec<Toast>>>);

    impl ToastSender for FakeToasts {
        fn send(&self, toast: Toast, _on_activated: Box<dyn Fn() + Send + Sync>) {
            self.0.lock().unwrap().push(toast);
        }
    }

    /// An app whose `claude` is `fake-agent`, over a temp home.
    fn fixture() -> Fixture {
        let guard = EnvGuard::new();
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("SDECK_USER_HOME", home.path());
        let (tx, rx) = mpsc::channel();
        let mut app = App::new(DeckStore::new(home.path().join("state.json")), tx, None);
        let mut config = DeckConfig::default();
        config.tools.get_mut("claude").unwrap().command = Some(fake_agent().to_string_lossy().into_owned());
        app.config = config;
        let toasts = Arc::new(Mutex::new(Vec::new()));
        app.notifier = Some(WaitingNotifier::new(Box::new(FakeToasts(Arc::clone(&toasts)))));
        Fixture {
            app,
            rx,
            toasts,
            home,
            _guard: guard,
        }
    }

    impl Fixture {
        fn add(&mut self, id: Option<&str>, on_disk: bool) -> u64 {
            let cwd = self.home.path().to_string_lossy().into_owned();
            let mut s = DeckSession::new("claude", id, &cwd, "a session", 1);
            s.on_disk = on_disk;
            let uid = s.uid;
            self.app.sessions.push(s);
            self.app.rebuild_rows();
            self.app
                .select_where(|r| matches!(r, TreeRow::Session { uid: u, .. } if *u == uid));
            uid
        }

        fn key(&mut self, key: &str) {
            self.app.handle(AppEvent::Input(key.into()), Instant::now());
        }

        fn session(&self, uid: u64) -> Option<&DeckSession> {
            self.app.session_by_uid(uid)
        }

        /// Pumps PTY events into the app until the selected session's screen shows `text`.
        fn wait_for_screen(&mut self, uid: u64, text: &str) {
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                let contents = self
                    .session(uid)
                    .and_then(|s| s.live.as_ref())
                    .map(|l| l.screen().contents())
                    .unwrap_or_default();
                if contents.contains(text) {
                    return;
                }
                let left = deadline.saturating_duration_since(Instant::now());
                let ev = self
                    .rx
                    .recv_timeout(left)
                    .unwrap_or_else(|_| panic!("timed out waiting for {text:?}; screen:\n{contents}"));
                self.app.handle(ev, Instant::now());
            }
        }
    }

    #[test]
    fn s_starts_x_stops_and_capital_r_restarts() {
        let mut f = fixture();
        let uid = f.add(Some("abc"), true);
        let transcript = f.home.path().join("abc.jsonl");
        fs::write(&transcript, "").unwrap();
        f.app.session_mut(uid).unwrap().file = Some(transcript);
        f.key("s");
        assert_eq!(f.app.message, "Started in background");
        f.wait_for_screen(uid, "ARGS=--resume abc");
        assert_eq!(
            f.app.procs.status_of(f.session(uid).unwrap()),
            SessionStatus::Starting
        );
        let first = f.session(uid).unwrap().live.as_ref().unwrap().id;

        f.key("R");
        assert_eq!(f.app.message, "Restarted");
        let second = f.session(uid).unwrap().live.as_ref().unwrap().id;
        assert_ne!(first, second);
        f.wait_for_screen(uid, "ARGS=--resume abc");

        // Nothing sent yet (no transcript): a restart starts a new conversation instead.
        let fresh = f.add(Some("def"), true);
        f.key("R");
        f.wait_for_screen(fresh, "> ");
        assert_eq!(f.session(fresh).unwrap().id, None);
        f.app
            .select_where(|r| matches!(r, TreeRow::Session { uid: u, .. } if *u == uid));

        f.key("x");
        assert_eq!(f.app.message, "Session stopped");
        assert!(
            f.session(uid).unwrap().live.is_none(),
            "stays listed: it's on disk"
        );
    }

    #[test]
    fn the_preview_shows_the_live_agent_screen() {
        let mut f = fixture();
        let uid = f.add(Some("abc"), true);
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 12)).unwrap();
        f.app.set_size(100, 12);
        f.key("s");
        f.wait_for_screen(uid, "> ");
        terminal.draw(|frame| f.app.draw(frame)).unwrap();
        let buf = terminal.backend().buffer();
        let rows: Vec<String> = (0..12)
            .map(|y| {
                (36..100)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect();
        assert_eq!(rows[2], "⟳ a session");
        assert!(rows[3].starts_with("─ starting · claude · "), "{}", rows[3]);
        assert_eq!(&rows[4..7], ["CLAUDE_CONFIG_DIR=unset", "ARGS=--resume abc", ">"]);
        // The agent got the preview body's size.
        assert_eq!(f.session(uid).unwrap().live.as_ref().unwrap().size(), (64, 7));
    }

    #[test]
    fn a_new_session_with_nothing_sent_vanishes_on_stop() {
        let mut f = fixture();
        let uid = f.add(None, false);
        f.key("s");
        f.wait_for_screen(uid, "ARGS=");
        f.key("x");
        assert!(f.session(uid).is_none());
    }

    #[test]
    fn the_exit_code_shows_in_the_status_and_a_stopped_session_restarts() {
        let mut f = fixture();
        let uid = f.add(Some("abc"), true);
        f.key("s");
        f.wait_for_screen(uid, "> ");
        f.app
            .session_mut(uid)
            .unwrap()
            .live
            .as_mut()
            .unwrap()
            .write(b"/exit 4\r");
        let deadline = Instant::now() + Duration::from_secs(15);
        while !f.session(uid).unwrap().live.as_ref().unwrap().exited {
            let ev = f.rx.recv_timeout(deadline - Instant::now()).unwrap();
            f.app.handle(ev, Instant::now());
        }
        let s = f.session(uid).unwrap();
        assert_eq!(f.app.procs.status_of(s), SessionStatus::Exited);
        assert_eq!(s.live.as_ref().unwrap().exit_code, Some(4));
        f.key("s");
        assert!(f.session(uid).unwrap().is_live());
    }

    #[test]
    fn a_resumed_session_launches_as_its_own_account_and_a_new_one_as_the_active_account() {
        let mut f = fixture();
        let me = sdeck_core::status::account::Account {
            config_dir: f.home.path().join(".claude-me"),
            email: Some("me@x.com".into()),
            is_default: false,
        };
        let work = sdeck_core::status::account::Account {
            config_dir: f.home.path().join(".claude"),
            email: Some("work@x.com".into()),
            is_default: true,
        };
        f.app.accounts = vec![work.clone(), me.clone()];
        f.app.config.ui.active_account_config_dir = Some(me.config_dir.to_string_lossy().into_owned());

        let resumed = f.add(Some("abc"), true);
        f.app.session_mut(resumed).unwrap().account = Some(work);
        f.key("s");
        f.wait_for_screen(resumed, "CLAUDE_CONFIG_DIR=unset");

        let fresh = f.add(None, false);
        f.key("s");
        let expected = format!("CLAUDE_CONFIG_DIR={}", me.config_dir.display());
        f.wait_for_screen(fresh, &expected);
        assert_eq!(f.session(fresh).unwrap().account.as_ref(), Some(&me));
    }

    #[test]
    fn a_missing_folder_or_command_flashes_instead_of_starting() {
        let mut f = fixture();
        let uid = f.add(Some("abc"), true);
        f.app.session_mut(uid).unwrap().cwd = f.home.path().join("gone").to_string_lossy().into_owned();
        f.key("s");
        assert!(
            f.app.message.starts_with("Folder no longer exists"),
            "{}",
            f.app.message
        );

        let uid = f.add(Some("def"), true);
        f.app.config.tools.get_mut("claude").unwrap().command = Some("Z:\\nope\\missing.exe".into());
        f.app.executables.clear();
        f.key("s");
        assert!(
            f.app.message.starts_with("Could not start Claude"),
            "{}",
            f.app.message
        );
        assert!(!f.session(uid).unwrap().is_live());
    }

    #[test]
    fn a_session_running_elsewhere_is_refused() {
        let mut f = fixture();
        let uid = f.add(Some("abc"), true);
        f.app.procs.by_session.insert(
            "abc".into(),
            sdeck_core::status::claude_process_watcher::LiveClaudeProcess {
                pid: 1,
                session_id: "abc".into(),
                status: "idle".into(),
                cwd: None,
                account: sdeck_core::status::account::Account {
                    config_dir: "C:\\.claude".into(),
                    email: None,
                    is_default: true,
                },
            },
        );
        f.key("s");
        assert_eq!(f.app.message, "That session is running in another terminal.");
        assert!(!f.app.ensure_live(uid, Instant::now()));
        f.key("R");
        assert_eq!(f.app.message, "That session is running in another terminal.");
    }

    #[test]
    fn the_notifier_toasts_a_live_session_that_needs_you() {
        let mut f = fixture();
        let uid = f.add(Some("n1"), true);
        f.key("s");
        f.wait_for_screen(uid, "> ");
        f.app.focused = false;
        fs::create_dir_all(session_status_dir()).unwrap();
        let status = |s: &str, at: i64| {
            fs::write(
                session_status_dir().join("n1.json"),
                format!(r#"{{"status":"{s}","updatedAt":{at}}}"#),
            )
            .unwrap();
        };
        status("running", 1);
        f.app.notify_changes(); // first sight: silent
        status("waiting", 2);
        f.app.notify_changes();
        status("waiting", 2);
        f.app.notify_changes(); // no transition
        let toasts = f.toasts.lock().unwrap().clone();
        assert_eq!(toasts.len(), 1);
        assert_eq!(toasts[0].message, "Waiting: a session");

        f.app.handle(AppEvent::ToastClicked("n1".into()), Instant::now());
        assert_eq!(f.app.selected_session().map(|s| s.uid), Some(uid));
    }

    #[test]
    fn a_prompt_queued_for_a_new_agent_is_typed_once_its_output_settles() {
        let mut f = fixture();
        let uid = f.add(Some("p1"), true);
        f.key("s");
        f.wait_for_screen(uid, "> ");
        // Idle per its pid record; a stub stands in for Claude's own pid file.
        let pid = f.session(uid).unwrap().live.as_ref().unwrap().pid;
        f.app.procs.by_pid.insert(
            pid,
            sdeck_core::status::claude_process_watcher::LiveClaudeProcess {
                pid,
                session_id: "p1".into(),
                status: "idle".into(),
                cwd: None,
                account: sdeck_core::status::account::Account {
                    config_dir: "C:\\.claude".into(),
                    email: None,
                    is_default: true,
                },
            },
        );
        f.app.session_mut(uid).unwrap().pending_prompt = Some("hello agent".into());
        let now = Instant::now();
        f.app.send_pending_prompt(uid, now);
        assert!(
            f.session(uid).unwrap().pending_prompt.is_some(),
            "output not settled yet"
        );
        f.app.send_pending_prompt(uid, now + PROMPT_SETTLE);
        assert!(f.session(uid).unwrap().pending_prompt.is_none());
        f.app
            .handle(AppEvent::Tick, now + PROMPT_SETTLE + Duration::from_millis(200));
        f.wait_for_screen(uid, "echo: hello agent");
    }

    #[test]
    fn a_new_session_adopts_its_id_from_its_pid_record() {
        let mut f = fixture();
        f.app.config.ui.recent_sessions_first = false;
        let uid = f.add(None, false);
        f.app.session_mut(uid).unwrap().pending_top_order = true;
        f.key("s");
        let pid = f.session(uid).unwrap().live.as_ref().unwrap().pid;
        f.app.procs.by_pid.insert(
            pid,
            sdeck_core::status::claude_process_watcher::LiveClaudeProcess {
                pid,
                session_id: "adopted".into(),
                status: "idle".into(),
                cwd: None,
                account: sdeck_core::status::account::Account {
                    config_dir: "C:\\.claude".into(),
                    email: None,
                    is_default: true,
                },
            },
        );
        f.app.adopt_live_ids();
        let s = f.session(uid).unwrap();
        assert_eq!(s.id.as_deref(), Some("adopted"));
        assert!(!s.pending_top_order);
        let key = s.project_key.clone();
        assert_eq!(
            f.app.tree.session_order.get(&key).map(Vec::as_slice),
            Some(&["adopted".to_string()][..])
        );
    }

    #[test]
    fn the_window_title_counts_sessions_that_need_you() {
        let mut f = fixture();
        f.app.update_title();
        assert_eq!(f.app.pending_output, "\x1b]0;Session Deck\x07");
        f.app.pending_output.clear();
        f.app.update_title();
        assert!(f.app.pending_output.is_empty());
    }
}
