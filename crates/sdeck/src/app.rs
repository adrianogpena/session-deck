//! App state and the main loop, port of the `App` class in `app.ts`. One thread owns this state;
//! everything else reaches it as an [`AppEvent`].

mod accounts;
mod archive;
mod attach;
mod bulk;
mod chords;
mod delete;
mod folders;
mod input;
mod interact;
mod lifecycle;
mod marks;
mod navigation;
mod new_session;
mod overlays;
mod preview;
mod prompt;
mod quit;
mod reorder;
mod search;
mod session_text;
mod tags;
#[cfg(test)]
mod test_fixture;
mod trace;
mod view_controls;

use std::collections::HashSet;
use std::io::Write;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use indexmap::IndexMap;
use ratatui::backend::Backend;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::{Frame, Terminal};
use sdeck_core::agent_catalog::all_agent_ids;
use sdeck_core::discovery::claude_storage::clear_session_meta_cache;
use sdeck_core::format::{humanize_since, now_ms};
use sdeck_core::paths::user_home;
use sdeck_core::status::account::Account;
use sdeck_core::status::claude_process_watcher::ClaudeProcessWatcher;
use sdeck_core::status::claude_transcript_tailer::{ClaudeTranscriptTailer, TailedTurn};
use sdeck_core::status::copilot_status_watcher::CopilotStatusWatcher;
use sdeck_core::status::session_usage::{
    read_latest_rate_limit_usage, read_session_usage, read_seven_day_daily_spend,
};
use sdeck_core::status::usage_display::format_reset_time;
use sdeck_core::status::waiting_notifier::{ToastSender, WaitingNotifier};
use sdeck_core::store::deck_config::DeckConfig;
use sdeck_core::store::deck_store::{DeckStore, Patch, ThemePreference, UiPatch, WatchHandle};
use sdeck_core::store::tree_prefs::{freeze_session_order, GroupView, SessionSort, TreePrefs};

use crate::event::AppEvent;
use crate::filters::{
    filter_key_category, matches_status_filter, toggle_status_filter, within_time_filter, StatusCategory,
    StatusCounts, TimeFilter,
};
use crate::git_status_tracker::{read_all, GitStatusTracker};
use crate::keys::{extract_focus_events, split_keys};
use crate::layout::{compute_layout, Layout, DEFAULT_SIDEBAR_PCT, SIDEBAR_STEP};
use crate::live_session::Executables;
use crate::sessions::{
    discover_found, display_title, merge_found, refresh_live_title, DeckSession, SessionStatus, StatusTracker,
};
use crate::theme::{
    extract_background_reply, next_theme_preference, preference_label, theme_label, Role, Theme, ThemeName,
    OSC11_QUERY,
};
use crate::tree::{build_tree, BuiltTree, TreeOptions, TreeRow, UsageSectionInput};
use crate::view::list_panel::{render_list_panel, ListRow};
use crate::view::preview_panel::{group_preview_lines, render_preview_panel};
use crate::view::{bars, overlay, GroupCounts, SessionView};
use input::{Confirm, Picker, TextPrompt};
use overlays::Overlay;
use quit::QuitConfirm;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const THEME_POLL: Duration = Duration::from_millis(5000);
const MESSAGE_DURATION: Duration = Duration::from_millis(4000);
/// How often pid files and status files are re-read.
const PROC_POLL: Duration = Duration::from_millis(1000);
const GIT_STATUS_POLL: Duration = Duration::from_millis(5000);
/// How long the loop waits for an event before handling a [`AppEvent::Tick`].
const TICK: Duration = Duration::from_millis(100);

/// Looks up the OS theme; blocking, so it runs on its own thread.
pub type OsThemeProbe = fn() -> Option<ThemeName>;

pub struct App {
    store: DeckStore,
    tx: Sender<AppEvent>,
    os_theme_probe: Option<OsThemeProbe>,
    theme_preference: ThemePreference,
    system_theme: ThemeName,
    /// Set once the terminal answers an OSC 11 query; from then on the OS setting isn't consulted.
    terminal_reports_background: bool,
    os_probe_running: bool,
    last_theme_poll: Option<Instant>,
    sidebar_pct: f64,
    sidebar_visible: bool,
    message: String,
    message_until: Option<Instant>,
    /// Whether sdeck's window is in front (focus reports); assumed until told otherwise.
    focused: bool,
    /// Bytes for the real terminal (the OSC 11 query, an attached agent's output), written by the
    /// loop before drawing.
    pending_output: Vec<u8>,
    dirty: bool,
    /// The real terminal was drawn over (attach, sidebar toggle): the loop repaints all of it.
    clear_screen: bool,
    /// sdeck's own mouse capture to switch on/off, applied by the loop through crossterm (it reads
    /// console input records on Windows, so raw mode sequences wouldn't do).
    mouse_capture_change: Option<bool>,
    quit: bool,
    /// The popup asking before quitting with sessions running; while open it takes every key.
    quit_confirm: Option<QuitConfirm>,
    /// The new sidebar width, shown in the list header until the instant.
    resize_note: Option<(String, Instant)>,

    /// Full-screen attached session (by uid): sdeck doesn't draw, the agent owns the terminal.
    attached: Option<u64>,
    /// Session typed into while the list and preview keep rendering.
    interacting: Option<u64>,
    /// Ctrl+K arrived while attached/interacting; the next key decides if it's a chord.
    chord_pending: bool,
    /// Footer text input (`p`, `o`, `e`) and the centered picker (`N`, `F3`); while either is open it
    /// takes every key.
    prompt: Option<TextPrompt>,
    picker: Option<Picker>,
    /// The popup opened by `?`, `C`, `w`, `v`, `a`, `/` or `:`; while open it takes every key.
    overlay: Option<Overlay>,
    /// Hands out ids that tell one search apart from the next.
    next_overlay_id: u64,
    /// Footer yes/no question: `y` confirms, any other key cancels.
    confirm: Option<Confirm>,
    /// Sessions checked with Space (by uid) for a batch action.
    multi_selected: HashSet<u64>,
    /// Ids moved to the trash this run, newest last, for Ctrl+Z.
    deleted: Vec<String>,
    /// A restored session's id: selected once the discovery that lists it again arrives.
    select_after_discovery: Option<String>,
    /// The agent `n` starts (`F3`, persisted).
    active_agent: String,
    /// `m`: sdeck's mouse scrolling. Off by default (and always while attached), not persisted.
    mouse_tracking: bool,

    /// Settings from `~/.session-deck/config.json`.
    config: DeckConfig,
    /// Every logged-in Claude account (fixed at startup).
    accounts: Vec<Account>,
    /// Whether discovery, process polling and git polling run (they read the real home, so tests
    /// leave them off and feed events by hand). See [`App::start_background`].
    sources_enabled: bool,
    sessions: Vec<DeckSession>,
    rows: Vec<TreeRow>,
    /// Displayed project order per container (`""` = top level, else folder id), for K/J.
    containers: IndexMap<String, Vec<String>>,
    session_containers: IndexMap<String, Vec<String>>,
    /// Index into `rows`: any row but a divider (or 0 when there are none).
    selected: usize,
    tree: TreePrefs,
    status_filter: Vec<StatusCategory>,
    time_filter: TimeFilter,
    /// Set from a tag row at the bottom of the list (Enter): narrows the tree to sessions carrying that tag.
    tag_filter: Option<String>,
    /// `^`: show archived sessions (only) instead of the active ones.
    archived_view: bool,
    procs: StatusTracker,
    git: GitStatusTracker,
    last_proc_poll: Option<Instant>,
    last_git_poll: Option<Instant>,
    git_polling: bool,
    discovering: bool,
    discover_again: bool,
    /// For `` ` `` (back to the previous session), by `DeckSession::uid`.
    last_session: Option<u64>,
    previous_session: Option<u64>,
    _store_watch: Option<WatchHandle>,

    /// The real terminal's size, from resize events.
    term_size: (u16, u16),
    executables: Executables,
    /// Tails the events log of Copilot sessions running here, writing their status files.
    copilot_watcher: CopilotStatusWatcher,
    /// Turns Claude's process status into status files ("done" after a turn), like the extension.
    _claude_watcher: Option<ClaudeProcessWatcher>,
    /// Toasts for sessions that need you; `None` until enabled (tests inject a fake sender).
    notifier: Option<WaitingNotifier<Box<dyn ToastSender>>>,
    /// Last terminal title written, so it's only rewritten when it changes.
    last_title: String,
    /// Lines the preview is scrolled back from the live bottom, for `scrolled_session`; reset when
    /// the selection moves.
    preview_scroll: usize,
    scrolled_session: Option<u64>,
    /// Live, read-only preview of whichever elsewhere session is previewed (`live_preview_for`).
    live_preview_tailer: ClaudeTranscriptTailer,
    live_preview_turns: Vec<TailedTurn>,
    live_preview_for: Option<u64>,
}

impl App {
    pub fn new(store: DeckStore, tx: Sender<AppEvent>, os_theme_probe: Option<OsThemeProbe>) -> Self {
        let ui = store.get_ui();
        let tree = store.get_tree();
        Self {
            store,
            tx,
            os_theme_probe,
            theme_preference: ui.theme.unwrap_or(ThemePreference::System),
            system_theme: ThemeName::Dark,
            terminal_reports_background: false,
            os_probe_running: false,
            last_theme_poll: None,
            sidebar_pct: ui.sidebar_pct.unwrap_or(DEFAULT_SIDEBAR_PCT),
            sidebar_visible: true,
            message: String::new(),
            message_until: None,
            focused: true,
            pending_output: Vec::new(),
            dirty: true,
            clear_screen: false,
            mouse_capture_change: None,
            quit: false,
            quit_confirm: None,
            resize_note: None,
            attached: None,
            interacting: None,
            chord_pending: false,
            mouse_tracking: false,
            prompt: None,
            picker: None,
            overlay: None,
            next_overlay_id: 0,
            confirm: None,
            multi_selected: HashSet::new(),
            deleted: Vec::new(),
            select_after_discovery: None,
            active_agent: ui
                .active_agent
                .filter(|a| all_agent_ids().contains(&a.as_str()))
                .unwrap_or_else(|| "claude".into()),
            config: DeckConfig::default(),
            accounts: Vec::new(),
            sources_enabled: false,
            sessions: Vec::new(),
            rows: Vec::new(),
            containers: IndexMap::new(),
            session_containers: IndexMap::new(),
            selected: 0,
            tree,
            status_filter: Vec::new(),
            time_filter: TimeFilter::All,
            tag_filter: None,
            archived_view: false,
            procs: StatusTracker::default(),
            git: GitStatusTracker::default(),
            last_proc_poll: None,
            last_git_poll: None,
            git_polling: false,
            discovering: false,
            discover_again: false,
            last_session: None,
            previous_session: None,
            _store_watch: None,
            term_size: (120, 30),
            executables: Executables::default(),
            copilot_watcher: CopilotStatusWatcher::new(|_| {}),
            _claude_watcher: None,
            notifier: None,
            last_title: String::new(),
            preview_scroll: 0,
            scrolled_session: None,
            live_preview_tailer: ClaudeTranscriptTailer::new(|_| {}),
            live_preview_turns: Vec::new(),
            live_preview_for: None,
        }
    }

    /// Turns on the real data sources: the first discovery now, then rediscovery whenever the shared
    /// state file changes, and pid/status/git polling on the loop's ticks.
    pub fn start_background(&mut self, accounts: Vec<Account>, config: DeckConfig) {
        self._claude_watcher = Some(ClaudeProcessWatcher::start(accounts.clone(), |_| {}));
        self.accounts = accounts;
        self.config = config;
        self.sources_enabled = true;
        // Pays the `where.exe` lookups now, not on whichever session starts first.
        self.executables.warm(&self.config);
        self.purge_expired_trash();
        let tx = self.tx.clone();
        self._store_watch = self
            .store
            .watch(move || {
                let _ = tx.send(AppEvent::StoreChanged);
            })
            .ok();
        self.spawn_discovery();
    }

    /// Desktop toasts for sessions that need you (`ui.notifications`).
    pub fn enable_notifications(&mut self, sender: Box<dyn ToastSender>) {
        self.notifier = Some(WaitingNotifier::new(sender));
    }

    pub fn set_size(&mut self, cols: u16, rows: u16) {
        self.term_size = (cols, rows);
        if let Some(uid) = self.attached {
            if let Some(live) = self.session_mut(uid).and_then(|s| s.live.as_mut()) {
                live.resize(cols, rows);
            }
        }
        self.resize_all_to_pane();
        self.dirty = true;
    }

    /// Queues `text` for the real terminal.
    fn emit(&mut self, text: &str) {
        self.pending_output.extend_from_slice(text.as_bytes());
    }

    fn layout(&self) -> Layout {
        compute_layout(
            self.term_size.0,
            self.term_size.1,
            self.sidebar_pct,
            self.sidebar_visible,
        )
    }

    pub fn should_quit(&self) -> bool {
        self.quit
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    pub fn handle(&mut self, ev: AppEvent, now: Instant) {
        match ev {
            AppEvent::Input(data) => self.on_input(&data, now),
            AppEvent::Resize(cols, rows) => self.set_size(cols, rows),
            AppEvent::Tick => self.on_tick(now),
            AppEvent::OsTheme(theme) => {
                self.os_probe_running = false;
                if let Some(theme) = theme.filter(|_| !self.terminal_reports_background) {
                    self.set_system_theme(theme);
                }
            }
            AppEvent::Discovered(found) => {
                self.discovering = false;
                self.sessions = merge_found(std::mem::take(&mut self.sessions), found);
                self.tree = self.store.get_tree();
                self.rebuild_rows();
                if let Some(id) = self.select_after_discovery.take() {
                    let restored = self
                        .sessions
                        .iter()
                        .find(|s| s.id.as_deref() == Some(id.as_str()))
                        .map(|s| s.uid);
                    self.select_where(
                        |r| matches!(r, TreeRow::Session { uid, .. } if Some(*uid) == restored),
                    );
                }
                self.dirty = true;
                if std::mem::take(&mut self.discover_again) {
                    self.spawn_discovery();
                }
            }
            AppEvent::GitStatuses(statuses) => {
                self.git_polling = false;
                self.git.replace(statuses);
                // The project row's badge is computed into `rows`, not read live.
                self.rebuild_rows();
                self.dirty = true;
            }
            AppEvent::StoreChanged => self.spawn_discovery(),
            AppEvent::PtyOutput(id, data) => self.on_pty_output(id, &data, now),
            AppEvent::PtyExited(id, code) => self.on_pty_exit(id, code),
            AppEvent::LastResponse(uid, text) => self.on_last_response(uid, text),
            AppEvent::LiveTurns(uid, turns) => self.on_live_turns(uid, turns),
            AppEvent::SearchText(id, text) => self.on_search_text(id, text),
            AppEvent::Trace(uid, steps) => self.on_trace_steps(uid, steps),
            AppEvent::ToastClicked(id) => self.on_toast_clicked(&id, now),
        }
        self.track_selection();
        self.sync_preview();
    }

    fn on_tick(&mut self, now: Instant) {
        if self.resize_note.as_ref().is_some_and(|(_, until)| now >= *until) {
            self.resize_note = None;
            self.dirty = true;
        }
        if self.message_until.is_some_and(|until| now >= until) {
            self.message.clear();
            self.message_until = None;
            self.dirty = true;
        }
        if self
            .last_theme_poll
            .is_none_or(|last| now.duration_since(last) >= THEME_POLL)
        {
            self.refresh_system_theme(now);
        }
        if self.sources_enabled {
            if self
                .last_proc_poll
                .is_none_or(|last| now.duration_since(last) >= PROC_POLL)
            {
                self.last_proc_poll = Some(now);
                self.poll_procs(now);
            }
            if self
                .last_git_poll
                .is_none_or(|last| now.duration_since(last) >= GIT_STATUS_POLL)
            {
                self.last_git_poll = Some(now);
                self.poll_git_status();
            }
        }
        self.tick_live(now);
    }

    // ---------------------------------------------------------------------------------------------
    // Sessions: discovery, polling, tree
    // ---------------------------------------------------------------------------------------------

    /// Reads the sessions on disk on its own thread; the result comes back as `Discovered`. A request
    /// made while one is running is repeated once it finishes.
    fn spawn_discovery(&mut self) {
        if !self.sources_enabled {
            return;
        }
        if self.discovering {
            self.discover_again = true;
            return;
        }
        self.discovering = true;
        let (tx, accounts, config) = (self.tx.clone(), self.accounts.clone(), self.config.clone());
        std::thread::spawn(move || {
            let _ = tx.send(AppEvent::Discovered(discover_found(&accounts, &config)));
        });
    }

    /// Refreshes every listed session's `git status` on its own thread; a no-op (no subprocess
    /// spawned) while `ui.gitStatus` is off.
    fn poll_git_status(&mut self) {
        if !self.config.ui.git_status || self.git_polling {
            return;
        }
        self.git_polling = true;
        let cwds: Vec<String> = self.sessions.iter().map(|s| s.cwd.clone()).collect();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(AppEvent::GitStatuses(read_all(&cwds)));
        });
    }

    /// Re-reads pid and status files, keeps live sessions' ids and titles and "elsewhere" times
    /// current, types queued prompts, then re-applies the tree (statuses, and so filter matches, move
    /// on their own) and notifies.
    fn poll_procs(&mut self, now: Instant) {
        self.procs.poll(&self.store, &self.accounts);
        self.adopt_live_ids();
        let mut live = Vec::new();
        for s in &mut self.sessions {
            if s.is_live() {
                refresh_live_title(s, &self.accounts);
                live.push(s.uid);
            } else if s.file.is_some() && self.procs.is_elsewhere(s) {
                // Another terminal keeps writing to it: keep its "5m ago" current.
                if let Some(mtime) = s
                    .file
                    .as_ref()
                    .and_then(|f| std::fs::metadata(f).ok())
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                {
                    s.mtime_ms = mtime.as_millis() as i64;
                }
            }
        }
        for uid in live {
            self.send_pending_prompt(uid, now);
        }
        self.rebuild_rows();
        self.notify_changes();
        self.update_title();
        self.dirty = true;
    }

    /// Replaces the session list (tests, and the discovery merge).
    #[cfg(test)]
    fn set_sessions(&mut self, sessions: Vec<DeckSession>) {
        self.sessions = sessions;
        self.rebuild_rows();
    }

    fn is_archived(&self, s: &DeckSession) -> bool {
        s.id.as_deref()
            .and_then(|id| self.store.get_session(id))
            .is_some_and(|p| p.archived)
    }

    fn tags_of(&self, s: &DeckSession) -> Vec<String> {
        s.id.as_deref()
            .and_then(|id| self.store.get_session(id))
            .map(|p| p.tags)
            .unwrap_or_default()
    }

    fn is_visible(&self, s: &DeckSession, now: i64) -> bool {
        self.is_archived(s) == self.archived_view
            && matches_status_filter(self.procs.category_of(s), &self.status_filter)
            && within_time_filter(s.mtime_ms, self.time_filter, now)
            && self
                .tag_filter
                .as_ref()
                .is_none_or(|tag| self.tags_of(s).contains(tag))
    }

    fn filtering(&self) -> bool {
        !self.status_filter.is_empty() || self.time_filter != TimeFilter::All || self.tag_filter.is_some()
    }

    /// The account new sessions launch as: the configured one, else the default account.
    fn active_account(&self) -> Option<&Account> {
        let configured = self.config.ui.active_account_config_dir.as_deref();
        configured
            .and_then(|dir| {
                self.accounts
                    .iter()
                    .find(|a| a.config_dir.as_path() == std::path::Path::new(dir))
            })
            .or_else(|| self.accounts.iter().find(|a| a.is_default))
            .or_else(|| self.accounts.first())
    }

    /// Sessions of projects removed with `d` are never listed.
    fn hidden_projects(&self) -> Vec<String> {
        self.store.get_hidden_projects()
    }

    /// The bottom usage section's data. Context tracks whichever session row is selected; 5h/7d are
    /// account-wide, so they come from whichever session's statusLine reported most recently.
    fn usage_section_input(&self) -> UsageSectionInput {
        let email = self.active_account().and_then(|a| a.email.clone());
        let session_usage = self
            .selected_session()
            .and_then(|s| s.id.as_deref())
            .and_then(read_session_usage);
        let rate_limit = read_latest_rate_limit_usage(email.as_deref());
        let use_24 = self.config.ui.use_24_hour_clock;
        UsageSectionInput {
            context_percent: session_usage.as_ref().and_then(|u| u.context_percent),
            context_updated_at: session_usage.as_ref().map(|u| u.updated_at),
            five_hour_reset_label: rate_limit
                .as_ref()
                .and_then(|r| r.five_hour_resets_at)
                .map(|at| format_reset_time(at, use_24, false)),
            seven_day_reset_label: rate_limit
                .as_ref()
                .and_then(|r| r.seven_day_resets_at)
                .map(|at| format_reset_time(at, use_24, true)),
            rate_limit,
            seven_day_daily_spend: read_seven_day_daily_spend(
                email.as_deref(),
                chrono::Local::now().date_naive(),
            ),
        }
    }

    /// Builds the tree for the current state; `expanded` ignores collapse (for `[`/`]`'s lookup).
    fn build(&self, expanded: bool) -> BuiltTree {
        let now = now_ms();
        let hidden = self.hidden_projects();
        let mut tree = self.tree.clone();
        if expanded {
            tree.collapsed.clear();
        }
        let opts = TreeOptions {
            include: Box::new(move |s| self.is_visible(s, now)),
            category_of: Box::new(|s| self.procs.category_of(s)),
            is_done_unseen: Box::new(|s| self.procs.status_of(s) == SessionStatus::Done),
            pin_of: Box::new(|s| {
                s.id.as_deref()
                    .and_then(|id| self.store.get_session(id))
                    .and_then(|p| p.pin)
            }),
            git_of: Box::new(|s| {
                self.config
                    .ui
                    .git_status
                    .then(|| self.git.get(&s.cwd).cloned())
                    .flatten()
            }),
            listed: Box::new(move |s| !hidden.contains(&s.project_key)),
            filtering: self.filtering(),
            recent_projects_first: self.config.ui.recent_projects_first,
            recent_sessions_first: self.config.ui.recent_sessions_first,
            tags_of: Box::new(|s| self.tags_of(s)),
            usage: (self.config.ui.show_usage && self.sources_enabled).then(|| self.usage_section_input()),
            share_projects: self.config.accounts.share_projects,
            show_all_sessions: self.config.accounts.show_all_sessions,
            active_account_dir: self.active_account().map(|a| a.config_dir.clone()),
        };
        build_tree(&self.sessions, &tree, &opts)
    }

    /// Re-applies the tree, sort and filters, keeping the selected row selected when it's still shown.
    fn rebuild_rows(&mut self) {
        let previous = self.rows.get(self.selected).cloned();
        let previous_index = self.selected;
        let previous_rows = std::mem::take(&mut self.rows);
        let built = self.build(false);
        self.rows = built.rows;
        self.containers = built.containers;
        self.session_containers = built.session_containers;
        if !self.config.ui.recent_sessions_first {
            self.apply_freeze_session_order();
        }
        let sessions = &self.sessions;
        self.multi_selected
            .retain(|uid| sessions.iter().any(|s| s.uid == *uid));
        let keep = previous
            .as_ref()
            .and_then(|p| self.rows.iter().position(|r| same_row(r, p)));
        self.selected = keep
            .or_else(|| self.nearest_surviving_row(&previous_rows, previous_index))
            .unwrap_or_else(|| {
                self.rows
                    .iter()
                    .position(|r| matches!(r, TreeRow::Session { .. }))
                    .unwrap_or(0)
            });
    }

    fn apply_freeze_session_order(&mut self) {
        let displayed: Vec<(String, Vec<String>)> = self
            .session_containers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let next = freeze_session_order(&self.tree, &displayed);
        if next != self.tree {
            self.tree = next;
            let _ = self.store.update_tree(|t| freeze_session_order(&t, &displayed));
        }
    }

    /// When the previously selected row is gone (deleted, archived, filtered out), selects whatever
    /// now sits where it used to be — the row right after it, else the row right before it.
    fn nearest_surviving_row(&self, previous_rows: &[TreeRow], previous_index: usize) -> Option<usize> {
        let find = |p: &TreeRow| self.rows.iter().position(|r| same_row(r, p));
        previous_rows
            .iter()
            .skip(previous_index + 1)
            .find_map(find)
            .or_else(|| previous_rows.iter().take(previous_index).rev().find_map(find))
    }

    /// Applies a tree change right away, and to the shared store (on the tree as it is on disk).
    fn change_tree(&mut self, change: impl Fn(TreePrefs) -> TreePrefs) {
        self.tree = change(self.tree.clone());
        self.rebuild_rows();
        let _ = self.store.update_tree(&change);
    }

    fn selected_row(&self) -> Option<&TreeRow> {
        self.rows.get(self.selected)
    }

    fn session_by_uid(&self, uid: u64) -> Option<&DeckSession> {
        self.sessions.iter().find(|s| s.uid == uid)
    }

    fn selected_session(&self) -> Option<&DeckSession> {
        match self.selected_row() {
            Some(TreeRow::Session { uid, .. }) => self.session_by_uid(*uid),
            _ => None,
        }
    }

    /// Remembers which session was selected before the current one, for `` ` ``.
    fn track_selection(&mut self) {
        let current = self.selected_session().map(|s| s.uid);
        if current.is_some() && current != self.last_session {
            self.previous_session = self.last_session;
            self.last_session = current;
        }
    }

    fn short_path(path: &str) -> String {
        let home = user_home().to_string_lossy().into_owned();
        match path.get(..home.len()) {
            Some(head) if !home.is_empty() && head.eq_ignore_ascii_case(&home) => {
                format!("~{}", &path[home.len()..])
            }
            _ => path.to_string(),
        }
    }

    fn view_of(&self, s: &DeckSession) -> SessionView {
        let status = self.procs.status_of(s);
        let active = s.is_live() && matches!(status, SessionStatus::Running | SessionStatus::Waiting);
        let active_dir = self.active_account().map(|a| &a.config_dir);
        let account_tag = (self.accounts.len() > 1)
            .then_some(s.account.as_ref())
            .flatten()
            .filter(|a| Some(&a.config_dir) != active_dir)
            .and_then(|a| a.email.as_deref())
            .map(|email| email.split('@').next().unwrap_or(email).to_string());
        SessionView {
            title: display_title(s, &self.store),
            status,
            elsewhere: self.procs.is_elsewhere(s),
            agent: s.agent.clone(),
            time_label: if active {
                "now".into()
            } else {
                humanize_since(s.mtime_ms, now_ms())
            },
            cwd: Self::short_path(&s.cwd),
            id: s.id.clone(),
            detail: s
                .live
                .as_ref()
                .filter(|l| !l.exited)
                .and_then(|l| l.screen_error.clone()),
            git: self
                .config
                .ui
                .git_status
                .then(|| self.git.get(&s.cwd).cloned())
                .flatten(),
            account_tag,
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Theme
    // ---------------------------------------------------------------------------------------------

    fn theme(&self) -> Theme {
        Theme::new(match self.theme_preference {
            ThemePreference::Dark => ThemeName::Dark,
            ThemePreference::Light => ThemeName::Light,
            ThemePreference::System => self.system_theme,
        })
    }

    /// Asks the terminal for its background (answered via input, see `on_input`); falls back to the
    /// OS setting until the terminal has answered once.
    pub fn refresh_system_theme(&mut self, now: Instant) {
        self.last_theme_poll = Some(now);
        // Attached, the query would reach the terminal mid-agent-output and its reply the agent.
        if self.theme_preference != ThemePreference::System || self.attached.is_some() {
            return;
        }
        self.emit(OSC11_QUERY);
        if let Some(probe) = self
            .os_theme_probe
            .filter(|_| !self.terminal_reports_background && !self.os_probe_running)
        {
            self.os_probe_running = true;
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(AppEvent::OsTheme(probe()));
            });
        }
    }

    fn set_system_theme(&mut self, theme: ThemeName) {
        if theme != self.system_theme {
            self.system_theme = theme;
            self.dirty = true;
        }
    }

    fn cycle_theme(&mut self, now: Instant) {
        self.theme_preference = next_theme_preference(self.theme_preference);
        let _ = self.store.update_ui(&UiPatch {
            theme: Patch::Set(self.theme_preference),
            ..Default::default()
        });
        self.refresh_system_theme(now);
        self.flash(format!("Theme: {}", preference_label(self.theme_preference)), now);
    }

    // ---------------------------------------------------------------------------------------------
    // Input
    // ---------------------------------------------------------------------------------------------

    fn flash(&mut self, text: String, now: Instant) {
        self.message = text;
        self.message_until = Some(now + MESSAGE_DURATION);
        self.dirty = true;
    }

    fn on_input(&mut self, raw: &str, now: Instant) {
        // Stripped ahead of everything else, so a focus report is never mistaken for a keypress.
        let (focused, data) = extract_focus_events(raw);
        if let Some(focused) = focused {
            self.focused = focused;
        }
        if let Some(uid) = self.attached {
            self.on_live_input(uid, &data, now);
            return;
        }
        // Answers to our background-color query arrive mixed into the input.
        let (theme, rest) = extract_background_reply(&data);
        if let Some(theme) = theme {
            self.terminal_reports_background = true;
            self.set_system_theme(theme);
        }
        let keys = split_keys(&rest);
        for (i, key) in keys.iter().enumerate() {
            if let Some(uid) = self.attached {
                // A key (Enter) just attached a session: the rest of the chunk is typed into it.
                self.on_live_input(uid, &keys[i..].concat(), now);
                return;
            }
            self.on_key(key, now);
            if self.quit {
                return;
            }
        }
    }

    fn on_key(&mut self, key: &str, now: Instant) {
        self.dirty = true;
        if let Some(uid) = self.interacting {
            // With mouse tracking on, a wheel notch scrolls the preview rather than reaching the
            // agent as a plain ↑/↓ (which it would take as prompt-history recall).
            if !self.on_mouse_sequence(key) {
                self.on_live_input(uid, key, now);
            }
            return;
        }
        if self.quit_confirm.is_some() {
            self.on_quit_confirm_key(key);
            return;
        }
        if self.prompt.is_some() {
            self.on_prompt_key(key, now);
            return;
        }
        if self.overlay.is_some() {
            self.on_overlay_key(key, now);
            return;
        }
        if self.picker.is_some() {
            self.on_picker_key(key, now);
            return;
        }
        if self.confirm.is_some() {
            self.on_confirm_key(key, now);
            return;
        }
        if let Some(category) = filter_key_category(key) {
            toggle_status_filter(&mut self.status_filter, category);
            self.rebuild_rows();
            return;
        }
        if let [digit @ b'1'..=b'9'] = key.as_bytes() {
            self.select_hotkey(digit - b'0');
            return;
        }
        if self.on_mouse_sequence(key) || self.on_page_key(key) {
            return;
        }
        match key {
            // Arrows scroll a live preview that has scrollback; k/j always move.
            "\x1b[A" | "\x1bOA" => self.on_arrow(-1),
            "k" => self.move_selection(-1),
            "\x1b[B" | "\x1bOB" => self.on_arrow(1),
            "j" => self.move_selection(1),
            "\x1b[D" | "h" => self.collapse_or_parent(),
            "\x1b[C" | "l" => self.expand_or_child(),
            "\t" => self.toggle_selected_group(),
            "\r" => match self.selected_session().map(|s| s.uid) {
                Some(uid) => self.attach(uid, now),
                None => self.enter_selected_row(),
            },
            "i" => match self.selected_session().map(|s| s.uid) {
                Some(uid) => self.start_interacting(uid, now),
                None => self.flash("Select a session to interact with.".into(), now),
            },
            "n" => {
                let agent = self.active_agent.clone();
                self.new_session(&agent, now);
            }
            "N" => self.open_agent_picker(false, now),
            "\x1bOR" | "\x1b[13~" => self.open_agent_picker(true, now),
            "\x1bOS" | "\x1b[14~" => self.cycle_account(now),
            "p" => self.open_add_project(),
            "o" => self.open_prompt_input(now),
            "c" => self.copy_last_response(now),
            " " => self.toggle_multi_select(),
            "\x1b" => self.clear_multi_select(now),
            "A" => self.toggle_archived(now),
            "^" => self.toggle_archived_view(now),
            "\x1a" => self.undo_delete(now),
            "Z" => self.open_trash_picker(now),
            "u" => self.mark_unread(now),
            "U" => self.mark_read(now),
            "," => self.cycle_pin(now),
            "g" => self.open_new_folder_prompt(),
            "M" => self.open_move_picker(now),
            "L" => self.open_tag_prompt(now),
            "K" | "\x1b[1;2A" => self.reorder(-1, now),
            "J" | "\x1b[1;2B" => self.reorder(1, now),
            "d" => self.delete_selected(now),
            "e" | "\x1bOQ" | "\x1b[12~" => self.rename(now),
            "\x0c" => self.clear_context(now),
            "m" => self.toggle_mouse_tracking(now),
            "`" => self.select_previous_session(now),
            "[" => self.cycle_active_session(-1, now),
            "]" => self.cycle_active_session(1, now),
            "T" => self.cycle_theme(now),
            "?" => self.open_help(),
            "C" => self.open_config(),
            "w" => self.open_skills(),
            "v" => self.open_trace(now),
            "a" => self.open_alerts(),
            "/" => self.open_search(),
            ":" => self.open_palette(),
            "s" => self.start_selected(now),
            "x" if self.multi_selected.is_empty() => self.stop_selected(now),
            "x" => self.bulk_stop(now),
            "R" => self.restart(now),
            "S" => {
                self.change_tree(|t| TreePrefs {
                    sort: if t.sort == SessionSort::Recent {
                        SessionSort::Actionable
                    } else {
                        SessionSort::Recent
                    },
                    ..t
                });
                let text = if self.tree.sort == SessionSort::Actionable {
                    "Sessions: needs attention first (error, waiting, running, idle)"
                } else {
                    "Sessions: most recent first"
                };
                self.flash(text.into(), now);
            }
            "t" => {
                self.change_tree(|t| TreePrefs {
                    view: if t.view == GroupView::Normal {
                        GroupView::Active
                    } else {
                        GroupView::Normal
                    },
                    ..t
                });
                let text = if self.tree.view == GroupView::Active {
                    "View: groups with running or waiting sessions on top"
                } else {
                    "View: normal"
                };
                self.flash(text.into(), now);
            }
            "*" => {
                self.time_filter = self.time_filter.next();
                self.rebuild_rows();
            }
            "0" => {
                self.status_filter.clear();
                self.time_filter = TimeFilter::All;
                self.tag_filter = None;
                self.rebuild_rows();
            }
            "r" => {
                clear_session_meta_cache();
                self.spawn_discovery();
            }
            "<" => self.resize_sidebar(-SIDEBAR_STEP, now),
            ">" => self.resize_sidebar(SIDEBAR_STEP, now),
            "b" => self.toggle_sidebar(),
            "q" | "\x03" => self.quit(),
            _ => {}
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Rendering
    // ---------------------------------------------------------------------------------------------

    fn list_rows(&self) -> Vec<ListRow> {
        self.rows
            .iter()
            .filter_map(|row| {
                Some(match row {
                    TreeRow::Folder {
                        name,
                        count,
                        running,
                        waiting,
                        collapsed,
                        hotkey,
                        ..
                    } => ListRow::Folder {
                        name: name.clone(),
                        counts: GroupCounts {
                            count: *count,
                            running: *running,
                            waiting: *waiting,
                        },
                        collapsed: *collapsed,
                        hotkey: *hotkey,
                    },
                    TreeRow::Project {
                        label,
                        count,
                        running,
                        waiting,
                        collapsed,
                        depth,
                        hotkey,
                        git,
                        ..
                    } => ListRow::Project {
                        label: label.clone(),
                        counts: GroupCounts {
                            count: *count,
                            running: *running,
                            waiting: *waiting,
                        },
                        collapsed: *collapsed,
                        depth: *depth,
                        hotkey: *hotkey,
                        git: git.clone(),
                    },
                    TreeRow::Session {
                        uid,
                        is_last,
                        depth,
                        pin,
                    } => ListRow::Session {
                        view: Box::new(self.view_of(self.session_by_uid(*uid)?)),
                        is_last: *is_last,
                        depth: *depth,
                        pin: *pin,
                        checked: self.multi_selected.contains(uid),
                    },
                    TreeRow::Divider { label } => ListRow::Divider { label: label.clone() },
                    TreeRow::Tag { name, count } => ListRow::Tag {
                        name: name.clone(),
                        count: *count,
                        active: self.tag_filter.as_ref() == Some(name),
                    },
                    TreeRow::Usage {
                        metric,
                        percent,
                        reset_label,
                        ..
                    } => ListRow::Usage {
                        metric: *metric,
                        percent: *percent,
                        reset_label: reset_label.clone(),
                    },
                    TreeRow::UsageBudget { days } => ListRow::UsageBudget { days: days.clone() },
                })
            })
            .collect()
    }

    /// Status counts for the header and pills: a finished-but-unseen session is its own bucket here,
    /// not folded into "waiting" the way `category_of` does for sort and filter purposes.
    fn counts(&self, hidden: &[String]) -> (StatusCounts, usize, usize, usize) {
        let mut counts = StatusCounts::default();
        let (mut done, mut in_view, mut live) = (0, 0, 0);
        for s in self.sessions.iter().filter(|s| !hidden.contains(&s.project_key)) {
            if s.is_live() {
                live += 1;
            }
            if self.is_archived(s) != self.archived_view {
                continue;
            }
            in_view += 1;
            if self.procs.status_of(s) == SessionStatus::Done {
                done += 1;
            } else {
                let category = self.procs.category_of(s);
                counts.set(category, counts.get(category) + 1);
            }
        }
        (counts, done, in_view, live)
    }

    pub fn draw(&self, frame: &mut Frame) {
        let t = self.theme();
        let area = frame.area();
        let cols = usize::from(area.width);
        let row = |y: u16| Rect::new(0, y, area.width, 1).intersection(area);
        let hidden = self.hidden_projects();
        let (counts, done, in_view, live) = self.counts(&hidden);
        let label = theme_label(self.theme_preference, t.name);
        frame.render_widget(
            bars::header(t, cols, &counts, done, live, &label, VERSION),
            row(0),
        );
        frame.render_widget(
            bars::pills(
                t,
                cols,
                in_view,
                &counts,
                done,
                &self.status_filter,
                self.time_filter,
                self.tag_filter.as_deref(),
            ),
            row(1),
        );

        let layout = compute_layout(area.width, area.height, self.sidebar_pct, self.sidebar_visible);
        if let Some(list) = layout.list {
            let mut modes = Vec::new();
            let selected_note = format!("{} selected", self.multi_selected.len());
            if !self.multi_selected.is_empty() {
                modes.push(selected_note.as_str());
            }
            if self.archived_view {
                modes.push("archived");
            }
            if self.filtering() {
                modes.push("filtered");
            }
            if self.tree.sort == SessionSort::Actionable {
                modes.push("actionable");
            }
            if self.tree.view == GroupView::Active {
                modes.push("active on top");
            }
            let note = if let Some(width) = self.active_resize_note(Instant::now()) {
                width.to_string()
            } else if modes.is_empty() {
                String::new()
            } else {
                format!("· {}", modes.join(" · "))
            };
            let empty = if self.sessions.iter().all(|s| hidden.contains(&s.project_key)) {
                "No Claude sessions found."
            } else {
                "Nothing matches the filter. Press 0 to clear it."
            };
            let lines = render_list_panel(
                t,
                usize::from(list.width),
                usize::from(list.height),
                &self.list_rows(),
                self.selected,
                &note,
                empty,
            );
            place_lines(frame, list, lines);
        }
        if let Some(preview) = layout.preview {
            match self.group_preview() {
                Some(group) => place_lines(
                    frame,
                    preview,
                    group_preview_lines(t, usize::from(preview.width), usize::from(preview.height), &group),
                ),
                None => render_preview_panel(t, preview, frame.buffer_mut(), self.preview_content().as_ref()),
            }
        }
        if let (Some(x), Some(list)) = (layout.divider_x, layout.list) {
            for y in 0..list.height {
                frame.render_widget(
                    Line::styled("│", t.fg(Role::Border)),
                    Rect::new(x, list.y + y, 1, 1).intersection(area),
                );
            }
        }

        if let Some(picker) = &self.picker {
            overlay::render_picker(frame, t, &picker.title, &picker.items, picker.index);
        }
        self.draw_overlay(frame, t);
        if let Some(confirm) = &self.quit_confirm {
            overlay::render_quit_confirm(frame, t, confirm.active_count, confirm.yes);
        }
        let bottom = row(area.height.saturating_sub(1));
        if let Some(prompt) = &self.prompt {
            frame.render_widget(bars::prompt_bar(t, cols, &prompt.label, &prompt.value), bottom);
        } else if let Some(confirm) = &self.confirm {
            frame.render_widget(bars::confirm_bar(t, cols, &confirm.question), bottom);
        } else if !self.message.is_empty() {
            frame.render_widget(bars::message_bar(t, cols, &self.message), bottom);
        } else if self.interacting.is_some() {
            let text = "Interacting · Ctrl+Q to stop · Ctrl+K T to attach";
            frame.render_widget(bars::message_bar(t, cols, text), bottom);
        } else {
            frame.render_widget(bars::help_bar(t, cols), bottom);
        }
    }
}

/// Same logical row across rebuilds (rows are recreated each time).
fn same_row(a: &TreeRow, b: &TreeRow) -> bool {
    match (a, b) {
        (TreeRow::Session { uid: a, .. }, TreeRow::Session { uid: b, .. }) => a == b,
        (TreeRow::Project { node_id: a, .. }, TreeRow::Project { node_id: b, .. }) => a == b,
        (TreeRow::Tag { name: a, .. }, TreeRow::Tag { name: b, .. }) => a == b,
        (TreeRow::Folder { folder_id: a, .. }, TreeRow::Folder { folder_id: b, .. }) => a == b,
        _ => false,
    }
}

/// Draws `lines` top-down inside `rect`, one per row, dropping what doesn't fit.
fn place_lines(frame: &mut Frame, rect: Rect, lines: Vec<Line<'static>>) {
    let area = frame.area();
    for (y, line) in (rect.y..rect.y + rect.height).zip(lines) {
        frame.render_widget(line, Rect::new(rect.x, y, rect.width, 1).intersection(area));
    }
}

/// The main loop: handles events until the app quits or every sender is gone, drawing only after a
/// change (several queued events are handled before one draw, like TS `scheduleRender`).
pub fn run_loop<B: Backend>(
    app: &mut App,
    terminal: &mut Terminal<B>,
    rx: &Receiver<AppEvent>,
    out: &mut impl Write,
) -> anyhow::Result<()>
where
    B::Error: std::error::Error + Send + Sync + 'static,
{
    app.refresh_system_theme(Instant::now());
    let size = terminal.size()?;
    app.set_size(size.width, size.height);
    loop {
        if !app.pending_output.is_empty() {
            out.write_all(&std::mem::take(&mut app.pending_output))?;
            out.flush()?;
        }
        if let Some(on) = app.mouse_capture_change.take() {
            // Best effort: there's no console to set in tests.
            let _ = if on {
                ratatui::crossterm::execute!(out, ratatui::crossterm::event::EnableMouseCapture)
            } else {
                ratatui::crossterm::execute!(out, ratatui::crossterm::event::DisableMouseCapture)
            };
        }
        // Attached, the agent owns the screen: nothing is drawn until detach.
        if app.attached.is_none() && std::mem::take(&mut app.dirty) {
            if std::mem::take(&mut app.clear_screen) {
                terminal.clear()?;
            }
            terminal.draw(|f| app.draw(f))?;
        }
        let first = match rx.recv_timeout(TICK) {
            Ok(ev) => ev,
            Err(RecvTimeoutError::Timeout) => AppEvent::Tick,
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        };
        let now = Instant::now();
        app.handle(first, now);
        while let Ok(ev) = rx.try_recv() {
            if app.quit {
                break;
            }
            app.handle(ev, now);
        }
        if app.quit {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use std::sync::mpsc;

    fn store_in(dir: &tempfile::TempDir) -> DeckStore {
        DeckStore::new(dir.path().join("state.json"))
    }

    fn screen(terminal: &Terminal<TestBackend>) -> Vec<String> {
        let buf = terminal.backend().buffer();
        (0..buf.area.height)
            .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect())
            .collect()
    }

    #[test]
    fn app_loop_processes_a_scripted_sequence_ending_in_quit() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let mut app = App::new(store_in(&dir), tx.clone(), None);
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
        let mut out = Vec::new();
        tx.send(AppEvent::Input("\x1b[OT".into())).unwrap();
        tx.send(AppEvent::Resize(100, 20)).unwrap();
        tx.send(AppEvent::Input("j\x1b[A".into())).unwrap();
        tx.send(AppEvent::Input("q".into())).unwrap();
        tx.send(AppEvent::Input("T".into())).unwrap();
        run_loop(&mut app, &mut terminal, &rx, &mut out).unwrap();

        assert!(app.should_quit());
        assert!(!app.is_focused());
        // Default is system; one `T` → dark. The `T` queued after `q` is never handled.
        assert_eq!(store_in(&dir).get_ui().theme, Some(ThemePreference::Dark));
        let rows = screen(&terminal);
        assert!(rows[0].contains("Session Deck"), "{}", rows[0]);
        assert!(rows[2].starts_with("SESSIONS"), "{}", rows[2]);
        assert!(rows[5].contains("No Claude sessions found."), "{}", rows[5]);
        assert!(rows[19].starts_with(" ↑↓ select"), "{}", rows[19]);
        // The initial poll asks the terminal for its background while following the system theme.
        assert_eq!(String::from_utf8(out).unwrap(), OSC11_QUERY);
    }

    #[test]
    fn ctrl_c_quits_and_a_closed_channel_ends_the_loop() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let mut app = App::new(store_in(&dir), tx.clone(), None);
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
        tx.send(AppEvent::Input("\x03".into())).unwrap();
        run_loop(&mut app, &mut terminal, &rx, &mut Vec::new()).unwrap();
        assert!(app.should_quit());

        let (_, dead_rx) = mpsc::channel::<AppEvent>();
        let mut app = App::new(store_in(&dir), tx, None);
        run_loop(&mut app, &mut terminal, &dead_rx, &mut Vec::new()).unwrap();
        assert!(!app.should_quit());
    }

    #[test]
    fn theme_cycling_persists_and_flashes() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, _rx) = mpsc::channel();
        let mut app = App::new(store_in(&dir), tx, None);
        let now = Instant::now();
        let mut seen = vec![];
        for _ in 0..3 {
            app.handle(AppEvent::Input("T".into()), now);
            seen.push((store_in(&dir).get_ui().theme.unwrap(), app.message.clone()));
        }
        assert_eq!(
            seen,
            [
                (ThemePreference::Dark, "Theme: dark".to_string()),
                (ThemePreference::Light, "Theme: light".to_string()),
                (ThemePreference::System, "Theme: system".to_string()),
            ]
        );
        app.handle(AppEvent::Tick, now + MESSAGE_DURATION);
        assert!(app.message.is_empty());
    }

    #[test]
    fn system_theme_follows_the_terminal_reply_over_the_os_probe() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let mut app = App::new(store_in(&dir), tx, Some(|| Some(ThemeName::Light)));
        let now = Instant::now();
        app.handle(AppEvent::Tick, now);
        let probed = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(probed, AppEvent::OsTheme(Some(ThemeName::Light)));
        app.handle(probed, now);
        assert_eq!(app.theme().name, ThemeName::Light);

        app.handle(AppEvent::Input("\x1b]11;rgb:1a1a/1b1b/2626\x07".into()), now);
        assert_eq!(app.theme().name, ThemeName::Dark);
        app.handle(AppEvent::OsTheme(Some(ThemeName::Light)), now);
        assert_eq!(app.theme().name, ThemeName::Dark);
        // Polled again only after the interval, and without the OS probe now that the terminal answers.
        app.pending_output.clear();
        app.handle(AppEvent::Tick, now + Duration::from_millis(100));
        assert!(app.pending_output.is_empty());
        app.handle(AppEvent::Tick, now + THEME_POLL);
        assert_eq!(app.pending_output, OSC11_QUERY.as_bytes());
        assert!(rx.try_recv().is_err());
    }
}
