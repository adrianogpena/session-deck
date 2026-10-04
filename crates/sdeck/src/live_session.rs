//! Port of `liveSession.ts`: an agent running in a background PTY, mirrored into a headless
//! `vt100` screen so the preview can draw it (and attach can repaint it) at any time.
//!
//! The PTY reader and the exit waiter are threads that send [`AppEvent`]s; the screen itself is
//! fed on the main loop ([`LiveSession::feed`]), so it has a single owner.

use std::borrow::Cow;
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use sdeck_core::agent_catalog::{all_agent_ids, is_builtin_agent};
use sdeck_core::status::account::Account;
use sdeck_core::store::deck_config::DeckConfig;

use crate::event::AppEvent;
use crate::screen_status::{detect_screen_error, screen_lines};

/// Lines of history the mirrored screen keeps, for the preview's own scroll.
const SCROLLBACK: usize = 2000;
/// The screen is checked for errors at most this often, once output settles.
const SCREEN_CHECK_DELAY: Duration = Duration::from_millis(500);
/// `type_line` sends Enter separately, after this, so it isn't taken as part of a paste.
const ENTER_DELAY: Duration = Duration::from_millis(150);

/// What [`LiveSession::spawn`] actually launches: `file`, then `prefix_args`, then the agent's args.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub file: String,
    /// Non-empty only when `file` is the command interpreter wrapping a `.cmd`/`.bat` shim: ConPTY
    /// can start a real `.exe` but not a batch script.
    pub prefix_args: Vec<String>,
}

impl Launch {
    fn direct(file: impl Into<String>) -> Self {
        Launch {
            file: file.into(),
            prefix_args: Vec::new(),
        }
    }

    /// A `.cmd`/`.bat` goes through the command interpreter; anything else runs as-is.
    fn for_path(path: &str) -> Self {
        let lower = path.to_ascii_lowercase();
        if lower.ends_with(".cmd") || lower.ends_with(".bat") {
            let comspec = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into());
            Launch {
                file: comspec,
                prefix_args: vec!["/d".into(), "/s".into(), "/c".into(), path.into()],
            }
        } else {
            Launch::direct(path)
        }
    }
}

/// The npm package that installs each built-in agent's CLI, to find its real binary when it was
/// only installed with `npm install -g`.
fn npm_package(agent: &str) -> Option<&'static str> {
    match agent {
        "claude" => Some("@anthropic-ai/claude-code"),
        "copilot" => Some("@github/copilot"),
        _ => None,
    }
}

/// Given the directory of an agent's npm-global shim, the native `.exe` its package's
/// `package.json#bin` points to, if that exists. `None` for a JS entry point (Copilot's loader) or a
/// catalog agent (no known package).
pub fn resolve_npm_global_executable(agent: &str, shim_dir: &Path) -> Option<PathBuf> {
    let pkg_dir = npm_package(agent)?
        .split('/')
        .fold(shim_dir.join("node_modules"), |dir, part| dir.join(part));
    let raw = std::fs::read_to_string(pkg_dir.join("package.json")).ok()?;
    let pkg: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let bin = match pkg.get("bin")? {
        serde_json::Value::String(s) => s.as_str(),
        other => other.get(agent)?.as_str()?,
    };
    if !bin.ends_with(".exe") {
        return None;
    }
    let exe = bin.split('/').fold(pkg_dir, |dir, part| dir.join(part));
    exe.exists().then_some(exe)
}

/// `where.exe <name>`'s first hit.
fn where_exe(name: &str) -> Option<PathBuf> {
    let output = std::process::Command::new("where.exe")
        .arg(name)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .next()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
}

/// The agent's executable, and whether it was really found (rather than the `<agent>.exe` guess):
/// the `tools.<agent>.command` override as-is (a `.cmd`/`.bat` one through the interpreter, which TS
/// didn't do — ConPTY can't start it otherwise); else, on Windows, `<agent>.exe` on PATH, the native
/// binary of an npm-global install, or its `.cmd` shim through the interpreter.
pub fn resolve_launch(
    agent: &str,
    configured: Option<&str>,
    find: impl Fn(&str) -> Option<PathBuf>,
) -> (Launch, bool) {
    if let Some(command) = configured.filter(|c| !c.is_empty()) {
        return (Launch::for_path(command), true);
    }
    if !cfg!(windows) {
        return (Launch::direct(agent), true);
    }
    if let Some(exe) = find(&format!("{agent}.exe")) {
        return (Launch::direct(exe.to_string_lossy()), true);
    }
    let Some(shim) = find(&format!("{agent}.cmd")) else {
        return (Launch::direct(format!("{agent}.exe")), false);
    };
    let npm_exe = shim
        .parent()
        .and_then(|dir| resolve_npm_global_executable(agent, dir));
    match npm_exe {
        Some(exe) => (Launch::direct(exe.to_string_lossy()), true),
        None => (Launch::for_path(&shim.to_string_lossy()), true),
    }
}

/// Resolved launches per agent, so `where.exe` runs once per agent rather than per spawn.
#[derive(Debug, Default)]
pub struct Executables {
    cache: HashMap<String, (Launch, bool)>,
}

impl Executables {
    pub fn resolve(&mut self, agent: &str, config: &DeckConfig) -> Launch {
        self.entry(agent, config).0.clone()
    }

    /// Whether `agent` resolves to something runnable rather than the unverified guess.
    pub fn is_available(&mut self, agent: &str, config: &DeckConfig) -> bool {
        self.entry(agent, config).1
    }

    /// Pays every agent's lookup up front (startup), not on the first spawn.
    pub fn warm(&mut self, config: &DeckConfig) {
        for agent in all_agent_ids() {
            self.entry(agent, config);
        }
    }

    /// Forgets the resolved commands, e.g. after `tools.*.command` changed.
    pub fn clear(&mut self) {
        self.cache.clear();
    }

    fn entry(&mut self, agent: &str, config: &DeckConfig) -> &(Launch, bool) {
        self.cache.entry(agent.to_string()).or_insert_with(|| {
            let configured = config.tools.get(agent).and_then(|t| t.command.as_deref());
            resolve_launch(agent, configured, where_exe)
        })
    }
}

/// Arguments to resume `session_id`, or to start a new session (Copilot takes a pre-assigned id,
/// Claude assigns its own), then the configured `tools.<agent>.args`. A catalog agent gets only those.
pub fn agent_args(agent: &str, session_id: Option<&str>, is_new: bool, config: &DeckConfig) -> Vec<String> {
    let extra = config
        .tools
        .get(agent)
        .and_then(|t| t.args.clone())
        .unwrap_or_default();
    let base = match (agent, session_id) {
        (_, _) if !is_builtin_agent(agent) => vec![],
        ("copilot", Some(id)) if is_new => vec![format!("--session-id={id}")],
        ("copilot", Some(id)) => vec![format!("--resume={id}")],
        ("claude", Some(id)) if !is_new => vec!["--resume".into(), id.into()],
        _ => vec![],
    };
    base.into_iter().chain(extra).collect()
}

/// Vars a parent Claude Code process sets for its children: inherited, they make the agent think
/// it's a sub-session when sdeck runs inside Claude Code.
const INHERITED_CLAUDE_VARS: [&str; 11] = [
    "CLAUDECODE",
    "CLAUDE_PID",
    "CLAUDE_EFFORT",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_CODE_SSE_PORT",
];

/// sdeck's environment for the agent, minus the inherited Claude Code vars, with truecolor on. With
/// an account (Claude only), a non-default one sets `CLAUDE_CONFIG_DIR` to its dir and the default
/// one removes it: setting it to `~/.claude` would make Claude bootstrap a fresh, logged-out config.
pub fn agent_env(account: Option<&Account>) -> Vec<(OsString, OsString)> {
    let is = |key: &OsString, name: &str| key.to_string_lossy().eq_ignore_ascii_case(name);
    let mut env: Vec<(OsString, OsString)> = std::env::vars_os()
        .filter(|(k, _)| !is(k, "COLORTERM") && !INHERITED_CLAUDE_VARS.iter().any(|v| is(k, v)))
        .filter(|(k, _)| account.is_none() || !is(k, "CLAUDE_CONFIG_DIR"))
        .collect();
    env.push(("COLORTERM".into(), "truecolor".into()));
    if let Some(account) = account.filter(|a| !a.is_default) {
        env.push((
            "CLAUDE_CONFIG_DIR".into(),
            account.config_dir.clone().into_os_string(),
        ));
    }
    env
}

/// Collects the answers to terminal queries the mirror receives (vt100 leaves them unhandled), the
/// way xterm answers them: device attributes and status/cursor reports.
#[derive(Debug, Default)]
struct Replies(Vec<u8>);

impl vt100::Callbacks for Replies {
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        _i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        let first = params.first().and_then(|p| p.first()).copied().unwrap_or(0);
        let (row, col) = screen.cursor_position();
        let reply = match (i1, c, first) {
            (None, 'c', 0) => "\x1b[?1;2c".to_string(),
            (Some(b'>'), 'c', 0) => "\x1b[>0;276;0c".to_string(),
            (None, 'n', 5) => "\x1b[0n".to_string(),
            (None, 'n', 6) => format!("\x1b[{};{}R", row + 1, col + 1),
            (Some(b'?'), 'n', 6) => format!("\x1b[?{};{}R", row + 1, col + 1),
            _ => return,
        };
        self.0.extend_from_slice(reply.as_bytes());
    }
}

/// The process side of a live session; gone once it exited or was disposed.
struct Pty {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
}

static NEXT_LIVE_ID: AtomicU64 = AtomicU64::new(1);

/// A running agent: its PTY plus the screen mirroring it.
pub struct LiveSession {
    /// Routes `PtyOutput`/`PtyExited` to this run (a restart is a new run of the same session).
    pub id: u64,
    pub pid: u32,
    pub exited: bool,
    pub exit_code: Option<u32>,
    /// A problem only visible on the agent's screen, e.g. `sign-in failed · run /login`.
    pub screen_error: Option<String>,
    /// When the agent last printed anything: a prompt is typed only once its output has settled.
    pub last_output_at: Instant,
    screen_check_at: Option<Instant>,
    enter_at: Option<Instant>,
    parser: vt100::Parser<Replies>,
    pty: Option<Pty>,
}

impl std::fmt::Debug for LiveSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveSession")
            .field("id", &self.id)
            .field("pid", &self.pid)
            .field("exited", &self.exited)
            .field("exit_code", &self.exit_code)
            .field("screen_error", &self.screen_error)
            .finish_non_exhaustive()
    }
}

/// What to start, see [`LiveSession::spawn`].
pub struct SpawnRequest<'a> {
    pub launch: &'a Launch,
    pub args: Vec<String>,
    pub cwd: &'a str,
    pub env: Vec<(OsString, OsString)>,
    pub cols: u16,
    pub rows: u16,
}

impl LiveSession {
    fn new(pid: u32, rows: u16, cols: u16, pty: Option<Pty>) -> Self {
        LiveSession {
            id: NEXT_LIVE_ID.fetch_add(1, Ordering::Relaxed),
            pid,
            exited: false,
            exit_code: None,
            screen_error: None,
            last_output_at: Instant::now(),
            screen_check_at: None,
            enter_at: None,
            parser: vt100::Parser::new_with_callbacks(rows, cols, SCROLLBACK, Replies::default()),
            pty,
        }
    }

    /// Starts the agent in a background PTY. Its output arrives as `PtyOutput` (to be passed to
    /// [`feed`](Self::feed)), its end as `PtyExited`.
    pub fn spawn(req: SpawnRequest<'_>, tx: &Sender<AppEvent>) -> anyhow::Result<Self> {
        let pair = native_pty_system().openpty(PtySize {
            rows: req.rows,
            cols: req.cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let mut cmd = CommandBuilder::new(&req.launch.file);
        cmd.args(&req.launch.prefix_args);
        cmd.args(&req.args);
        cmd.cwd(req.cwd);
        cmd.env_clear();
        for (key, value) in &req.env {
            cmd.env(key, value);
        }
        let mut child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let pty = Pty {
            master: pair.master,
            writer,
            killer: child.clone_killer(),
        };
        let live = LiveSession::new(child.process_id().unwrap_or(0), req.rows, req.cols, Some(pty));

        let (id, out_tx) = (live.id, tx.clone());
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 || out_tx.send(AppEvent::PtyOutput(id, buf[..n].to_vec())).is_err() {
                    return;
                }
            }
        });
        let exit_tx = tx.clone();
        std::thread::spawn(move || {
            let code = child.wait().map_or(1, |s| s.exit_code());
            let _ = exit_tx.send(AppEvent::PtyExited(id, code));
        });
        Ok(live)
    }

    /// A live session without a process, for tests of what reads its fields.
    #[cfg(test)]
    pub fn stub(pid: u32) -> Self {
        LiveSession::new(pid, 24, 80, None)
    }

    /// Mirrors output into the screen. Terminal queries in it are answered here unless `attached`
    /// (the real terminal answers those itself; both would double-reply).
    pub fn feed(&mut self, data: &[u8], attached: bool, now: Instant) {
        self.parser.process(data);
        self.last_output_at = now;
        self.screen_check_at.get_or_insert(now + SCREEN_CHECK_DELAY);
        let replies = std::mem::take(&mut self.parser.callbacks_mut().0);
        if !attached && !replies.is_empty() {
            self.write(&replies);
        }
    }

    /// The process ended: the screen stays, the PTY goes (which also ends the reader thread).
    pub fn on_exit(&mut self, code: u32) {
        self.exited = true;
        self.exit_code = Some(code);
        self.pty = None;
    }

    /// Time-based work: the screen-error check once output settled, and a delayed Enter. Returns
    /// whether something visible changed.
    pub fn on_tick(&mut self, now: Instant) -> bool {
        if self.enter_at.is_some_and(|at| now >= at) {
            self.enter_at = None;
            self.write(b"\r");
        }
        if self.screen_check_at.is_some_and(|at| now >= at) {
            self.screen_check_at = None;
            let error = detect_screen_error(&screen_lines(self.parser.screen()));
            if error != self.screen_error {
                self.screen_error = error;
                return true;
            }
        }
        false
    }

    /// Sends input to the agent; a no-op once it's gone.
    pub fn write(&mut self, data: &[u8]) {
        if let Some(pty) = self.pty.as_mut().filter(|_| !self.exited) {
            let _ = pty.writer.write_all(data).and_then(|()| pty.writer.flush());
        }
    }

    /// Types a line and submits it: text and Enter as separate writes, so the input box doesn't take
    /// the Enter as part of a paste.
    pub fn type_line(&mut self, text: &str, now: Instant) {
        self.write(text.as_bytes());
        self.enter_at = Some(now + ENTER_DELAY);
    }

    pub fn size(&self) -> (u16, u16) {
        let (rows, cols) = self.parser.screen().size();
        (cols, rows)
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        if self.exited || self.size() == (cols, rows) {
            return;
        }
        if let Some(pty) = &self.pty {
            let _ = pty.master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            }); // may race with the exit
        }
        self.parser.screen_mut().set_size(rows, cols);
    }

    /// Kills the agent (if still running) and closes its PTY. The screen stays readable.
    pub fn dispose(&mut self) {
        if let Some(mut pty) = self.pty.take() {
            if !self.exited {
                let _ = pty.killer.kill();
            }
        }
    }

    /// The live screen.
    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    /// The screen `offset` lines back into scrollback (0 = live). Clamped to what's there.
    pub fn screen_at(&self, offset: usize) -> Cow<'_, vt100::Screen> {
        if offset == 0 {
            return Cow::Borrowed(self.parser.screen());
        }
        let mut screen = self.parser.screen().clone();
        screen.set_scrollback(offset);
        Cow::Owned(screen)
    }

    /// How many lines of scrollback the screen holds.
    pub fn max_scroll(&self) -> usize {
        let mut screen = self.parser.screen().clone();
        screen.set_scrollback(usize::MAX);
        screen.scrollback()
    }
}

impl Drop for LiveSession {
    fn drop(&mut self) {
        self.dispose();
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::test_support::EnvGuard;
    use std::sync::mpsc::{self, Receiver};
    use std::sync::OnceLock;

    /// The `fake-agent` binary, built once per test run.
    pub(crate) fn fake_agent() -> PathBuf {
        static PATH: OnceLock<PathBuf> = OnceLock::new();
        PATH.get_or_init(|| {
            let status = std::process::Command::new(env!("CARGO"))
                .args(["build", "-q", "-p", "fake-agent"])
                .current_dir(env!("CARGO_MANIFEST_DIR"))
                .status()
                .expect("cargo build fake-agent");
            assert!(status.success());
            let deps = std::env::current_exe().unwrap();
            let dir = deps.parent().unwrap().parent().unwrap();
            dir.join(format!("fake-agent{}", std::env::consts::EXE_SUFFIX))
        })
        .clone()
    }

    pub(crate) struct Run {
        pub live: LiveSession,
        pub rx: Receiver<AppEvent>,
    }

    impl Run {
        pub fn start(launch: &Launch, env: Vec<(OsString, OsString)>) -> Run {
            let (tx, rx) = mpsc::channel();
            let cwd = std::env::temp_dir();
            let live = LiveSession::spawn(
                SpawnRequest {
                    launch,
                    args: vec!["--resume".into(), "abc".into()],
                    cwd: &cwd.to_string_lossy(),
                    env,
                    cols: 80,
                    rows: 24,
                },
                &tx,
            )
            .unwrap();
            Run { live, rx }
        }

        /// Feeds output until the screen contains `text` (or the process exits).
        pub fn wait_for(&mut self, text: &str) -> String {
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                let contents = self.live.screen().contents();
                if contents.contains(text) {
                    return contents;
                }
                let left = deadline.saturating_duration_since(Instant::now());
                match self.rx.recv_timeout(left) {
                    Ok(AppEvent::PtyOutput(id, data)) if id == self.live.id => {
                        self.live.feed(&data, false, Instant::now())
                    }
                    Ok(AppEvent::PtyExited(id, code)) if id == self.live.id => self.live.on_exit(code),
                    Ok(_) => {}
                    Err(_) => panic!("timed out waiting for {text:?}; screen:\n{contents}"),
                }
            }
        }

        pub fn wait_for_exit(&mut self) -> u32 {
            let deadline = Instant::now() + Duration::from_secs(15);
            while !self.live.exited {
                let left = deadline.saturating_duration_since(Instant::now());
                match self.rx.recv_timeout(left).expect("timed out waiting for exit") {
                    AppEvent::PtyOutput(_, data) => self.live.feed(&data, false, Instant::now()),
                    AppEvent::PtyExited(_, code) => self.live.on_exit(code),
                    _ => {}
                }
            }
            self.live.exit_code.unwrap()
        }

        pub fn type_line(&mut self, text: &str) {
            self.live.write(text.as_bytes());
            self.live.write(b"\r");
        }
    }

    #[test]
    fn a_typed_line_shows_on_the_screen_and_the_exit_code_is_captured() {
        let launch = Launch::direct(fake_agent().to_string_lossy());
        let mut run = Run::start(&launch, agent_env(None));
        let screen = run.wait_for("> ");
        assert!(screen.contains("ARGS=--resume abc"), "{screen}");
        run.type_line("hello there");
        run.wait_for("echo: hello there");
        run.type_line("/exit 3");
        assert_eq!(run.wait_for_exit(), 3);
        assert!(run.live.exited);
    }

    #[test]
    fn a_cmd_shim_launches_through_the_interpreter() {
        let dir = tempfile::tempdir().unwrap();
        let shim = dir.path().join("fakeagent.cmd");
        std::fs::write(&shim, format!("@\"{}\" %*\r\n", fake_agent().display())).unwrap();
        let shim_for_find = shim.clone();
        let (launch, found) = resolve_launch("fakeagent", None, move |name| {
            (name == "fakeagent.cmd").then(|| shim_for_find.clone())
        });
        assert!(found);
        assert_eq!(
            launch.prefix_args.last().map(String::as_str),
            Some(shim.to_str().unwrap())
        );
        let mut run = Run::start(&launch, agent_env(None));
        run.wait_for("ARGS=--resume abc");
        run.type_line("/exit");
        assert_eq!(run.wait_for_exit(), 0);
    }

    #[test]
    fn the_account_decides_claude_config_dir() {
        let _g = EnvGuard::new();
        std::env::set_var("CLAUDE_CONFIG_DIR", "C:\\inherited");
        let launch = Launch::direct(fake_agent().to_string_lossy());
        let default = Account {
            config_dir: PathBuf::from("C:\\home\\.claude"),
            email: None,
            is_default: true,
        };
        let second = Account {
            config_dir: PathBuf::from("C:\\home\\.claude-me"),
            email: None,
            is_default: false,
        };
        let mut run = Run::start(&launch, agent_env(Some(&default)));
        run.wait_for("CLAUDE_CONFIG_DIR=unset");
        let mut run = Run::start(&launch, agent_env(Some(&second)));
        run.wait_for("CLAUDE_CONFIG_DIR=C:\\home\\.claude-me");
        let mut run = Run::start(&launch, agent_env(None));
        run.wait_for("CLAUDE_CONFIG_DIR=C:\\inherited");
    }

    #[test]
    fn dispose_kills_a_running_agent() {
        let launch = Launch::direct(fake_agent().to_string_lossy());
        let mut run = Run::start(&launch, agent_env(None));
        run.wait_for("> ");
        run.live.dispose();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            match run
                .rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(AppEvent::PtyExited(..)) => break,
                Ok(_) => {}
                Err(e) => panic!("no exit after dispose: {e}"),
            }
        }
    }

    #[test]
    fn terminal_queries_are_answered_unless_attached() {
        let mut live = LiveSession::stub(1);
        live.parser.process(b"ab\x1b[6n\x1b[c\x1b[>c\x1b[5n");
        let replies = std::mem::take(&mut live.parser.callbacks_mut().0);
        assert_eq!(
            String::from_utf8(replies).unwrap(),
            "\x1b[1;3R\x1b[?1;2c\x1b[>0;276;0c\x1b[0n"
        );
    }

    #[test]
    fn the_screen_error_is_checked_once_output_settles() {
        let mut live = LiveSession::stub(1);
        let now = Instant::now();
        live.feed(b"API Error: 401 authentication_error\r\n", false, now);
        assert!(!live.on_tick(now));
        assert!(live.on_tick(now + SCREEN_CHECK_DELAY));
        assert_eq!(live.screen_error.as_deref(), Some("sign-in failed · run /login"));
    }

    #[test]
    fn scrollback_is_readable_without_moving_the_live_screen() {
        let mut live = LiveSession::stub(1);
        live.resize(20, 3);
        for i in 0..10 {
            live.feed(format!("line {i}\r\n").as_bytes(), false, Instant::now());
        }
        assert_eq!(live.max_scroll(), 8);
        let back = live.screen_at(8);
        assert!(back.contents().starts_with("line 0"), "{}", back.contents());
        assert!(live.screen().contents().starts_with("line 8"));
    }

    #[test]
    fn launch_resolution_prefers_the_override_then_the_exe_then_the_npm_binary() {
        let none = |_: &str| None;
        assert_eq!(
            resolve_launch("claude", Some("C:\\bin\\claude.exe"), none),
            (Launch::direct("C:\\bin\\claude.exe"), true)
        );
        assert_eq!(
            resolve_launch("claude", None, |n: &str| (n == "claude.exe")
                .then(|| "C:\\x\\claude.exe".into())),
            (Launch::direct("C:\\x\\claude.exe"), true)
        );
        assert_eq!(
            resolve_launch("claude", None, none),
            (Launch::direct("claude.exe"), false)
        );

        let shim_dir = tempfile::tempdir().unwrap();
        let pkg = shim_dir
            .path()
            .join("node_modules")
            .join("@anthropic-ai")
            .join("claude-code");
        std::fs::create_dir_all(pkg.join("bin")).unwrap();
        std::fs::write(pkg.join("package.json"), r#"{"bin":{"claude":"bin/claude.exe"}}"#).unwrap();
        std::fs::write(pkg.join("bin/claude.exe"), "").unwrap();
        let shim = shim_dir.path().join("claude.cmd");
        let (launch, found) = resolve_launch("claude", None, |n: &str| {
            (n == "claude.cmd").then(|| shim.clone())
        });
        assert!(found);
        assert_eq!(
            launch,
            Launch::direct(pkg.join("bin").join("claude.exe").to_string_lossy())
        );
    }

    #[test]
    fn npm_global_executable_needs_a_native_exe_that_exists() {
        let make = |pkg_name: &str, bin: &str, files: &[&str]| {
            let dir = tempfile::tempdir().unwrap();
            let pkg = pkg_name
                .split('/')
                .fold(dir.path().join("node_modules"), |d, p| d.join(p));
            std::fs::create_dir_all(&pkg).unwrap();
            std::fs::write(pkg.join("package.json"), format!(r#"{{"bin":{bin}}}"#)).unwrap();
            for f in files {
                std::fs::create_dir_all(pkg.join(f).parent().unwrap()).unwrap();
                std::fs::write(pkg.join(f), "").unwrap();
            }
            dir
        };
        let claude = make(
            "@anthropic-ai/claude-code",
            r#"{"claude":"bin/claude.exe"}"#,
            &["bin/claude.exe"],
        );
        assert!(resolve_npm_global_executable("claude", claude.path()).is_some());
        let copilot = make(
            "@github/copilot",
            r#"{"copilot":"npm-loader.js"}"#,
            &["npm-loader.js"],
        );
        assert_eq!(resolve_npm_global_executable("copilot", copilot.path()), None);
        let missing = make("@anthropic-ai/claude-code", r#"{"claude":"bin/claude.exe"}"#, &[]);
        assert_eq!(resolve_npm_global_executable("claude", missing.path()), None);
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(resolve_npm_global_executable("claude", empty.path()), None);
        assert_eq!(resolve_npm_global_executable("codex", empty.path()), None);
    }

    #[test]
    fn agent_args_resume_or_start_and_append_the_configured_ones() {
        let mut config = DeckConfig::default();
        config.tools.get_mut("claude").unwrap().args = Some(vec!["--verbose".into()]);
        assert_eq!(
            agent_args("claude", Some("id1"), false, &config),
            ["--resume", "id1", "--verbose"]
        );
        assert_eq!(agent_args("claude", None, true, &config), ["--verbose"]);
        assert_eq!(
            agent_args("copilot", Some("c1"), true, &config),
            ["--session-id=c1"]
        );
        assert_eq!(agent_args("copilot", Some("c1"), false, &config), ["--resume=c1"]);
        config.tools.get_mut("codex").unwrap().args = Some(vec!["--x".into()]);
        assert_eq!(agent_args("codex", Some("k"), false, &config), ["--x"]);
    }

    #[test]
    fn the_agent_env_drops_inherited_claude_code_vars() {
        let _g = EnvGuard::new();
        std::env::set_var("CLAUDECODE", "1");
        let env = agent_env(None);
        std::env::remove_var("CLAUDECODE");
        assert!(!env.iter().any(|(k, _)| k == "CLAUDECODE"));
        let colorterm: Vec<_> = env.iter().filter(|(k, _)| k == "COLORTERM").collect();
        assert_eq!(colorterm.len(), 1);
        assert_eq!(colorterm[0].1, "truecolor");
    }
}
