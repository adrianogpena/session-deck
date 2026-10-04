//! An app whose `claude` is `fake-agent`, over a temp home, for the app's PTY tests.

use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sdeck_core::status::waiting_notifier::{Toast, ToastSender, WaitingNotifier};
use sdeck_core::store::deck_config::DeckConfig;
use sdeck_core::store::deck_store::DeckStore;

use super::App;
use crate::event::AppEvent;
use crate::live_session::tests::fake_agent;
use crate::sessions::DeckSession;
use crate::test_support::EnvGuard;
use crate::tree::TreeRow;

pub(crate) struct Fixture {
    pub app: App,
    pub rx: Receiver<AppEvent>,
    pub toasts: Arc<Mutex<Vec<Toast>>>,
    pub home: tempfile::TempDir,
    _guard: EnvGuard,
}

struct FakeToasts(Arc<Mutex<Vec<Toast>>>);

impl ToastSender for FakeToasts {
    fn send(&self, toast: Toast, _on_activated: Box<dyn Fn() + Send + Sync>) {
        self.0.lock().unwrap().push(toast);
    }
}

pub(crate) fn fixture() -> Fixture {
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
    /// Lists a session in the temp home and selects it.
    pub fn add(&mut self, id: Option<&str>, on_disk: bool) -> u64 {
        let cwd = self.home.path().to_string_lossy().into_owned();
        let mut s = DeckSession::new("claude", id, &cwd, "a session", 1);
        s.on_disk = on_disk;
        let uid = s.uid;
        self.app.sessions.push(s);
        self.app.rebuild_rows();
        self.select(uid);
        uid
    }

    pub fn select(&mut self, uid: u64) {
        self.app
            .select_where(|r| matches!(r, TreeRow::Session { uid: u, .. } if *u == uid));
    }

    pub fn key(&mut self, key: &str) {
        self.app.handle(AppEvent::Input(key.into()), Instant::now());
    }

    pub fn session(&self, uid: u64) -> Option<&DeckSession> {
        self.app.session_by_uid(uid)
    }

    /// Pumps PTY events into the app until the session's screen shows `text`. While attached it
    /// plays the real terminal: ConPTY's start-up cursor query gets an answer.
    pub fn wait_for_screen(&mut self, uid: u64, text: &str) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if self.app.attached.is_some() && self.app.pending_output.windows(4).any(|w| w == b"\x1b[6n") {
                self.app.pending_output.clear();
                self.key("\x1b[1;1R");
            }
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

impl Fixture {
    /// Stands in for Claude's own pid file: the session's live process reports `status`.
    pub fn set_process_status(&mut self, uid: u64, status: &str) {
        let pid = self.session(uid).unwrap().live.as_ref().unwrap().pid;
        let id = self.session(uid).unwrap().id.clone().unwrap_or_default();
        self.app.procs.by_pid.insert(
            pid,
            sdeck_core::status::claude_process_watcher::LiveClaudeProcess {
                pid,
                session_id: id,
                status: status.into(),
                cwd: None,
                account: sdeck_core::status::account::Account {
                    config_dir: r"C:\.claude".into(),
                    email: None,
                    is_default: true,
                },
            },
        );
    }
}
