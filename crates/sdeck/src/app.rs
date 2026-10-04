//! App state and the main loop, port of the `App` class in `app.ts`. One thread owns this state;
//! everything else reaches it as an [`AppEvent`].

use std::io::Write;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use ratatui::backend::Backend;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::{Frame, Terminal};
use sdeck_core::store::deck_store::{DeckStore, Patch, ThemePreference, UiPatch};

use crate::ansi::fit;
use crate::event::AppEvent;
use crate::filters::{StatusCounts, TimeFilter};
use crate::keys::{extract_focus_events, split_keys};
use crate::layout::{compute_layout, DEFAULT_SIDEBAR_PCT};
use crate::theme::{
    extract_background_reply, next_theme_preference, preference_label, theme_label, Role, Theme, ThemeName,
    OSC11_QUERY,
};
use crate::view::{bars, panel_header};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const THEME_POLL: Duration = Duration::from_millis(5000);
const MESSAGE_DURATION: Duration = Duration::from_millis(4000);
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
    /// Sequences for the real terminal (e.g. the OSC 11 query), written by the loop before drawing.
    pending_output: String,
    dirty: bool,
    quit: bool,
}

impl App {
    pub fn new(store: DeckStore, tx: Sender<AppEvent>, os_theme_probe: Option<OsThemeProbe>) -> Self {
        let ui = store.get_ui();
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
            pending_output: String::new(),
            dirty: true,
            quit: false,
        }
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
            AppEvent::Resize(..) => self.dirty = true,
            AppEvent::Tick => self.on_tick(now),
            AppEvent::OsTheme(theme) => {
                self.os_probe_running = false;
                if let Some(theme) = theme.filter(|_| !self.terminal_reports_background) {
                    self.set_system_theme(theme);
                }
            }
        }
    }

    fn on_tick(&mut self, now: Instant) {
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
        if self.theme_preference != ThemePreference::System {
            return;
        }
        self.pending_output.push_str(OSC11_QUERY);
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
        // Answers to our background-color query arrive mixed into the input.
        let (theme, rest) = extract_background_reply(&data);
        if let Some(theme) = theme {
            self.terminal_reports_background = true;
            self.set_system_theme(theme);
        }
        for key in split_keys(&rest) {
            self.on_key(&key, now);
            if self.quit {
                return;
            }
        }
    }

    fn on_key(&mut self, key: &str, now: Instant) {
        match key {
            "T" => self.cycle_theme(now),
            // A confirm popup arrives in 14.1.
            "q" | "\x03" => self.quit = true,
            _ => {}
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Rendering
    // ---------------------------------------------------------------------------------------------

    pub fn draw(&self, frame: &mut Frame) {
        let t = self.theme();
        let area = frame.area();
        let cols = usize::from(area.width);
        let row = |y: u16| Rect::new(0, y, area.width, 1).intersection(area);
        let counts = StatusCounts::default();
        let label = theme_label(self.theme_preference, t.name);
        frame.render_widget(bars::header(t, cols, &counts, 0, 0, &label, VERSION), row(0));
        frame.render_widget(
            bars::pills(t, cols, 0, &counts, 0, &[], TimeFilter::All, None),
            row(1),
        );

        let layout = compute_layout(area.width, area.height, self.sidebar_pct, self.sidebar_visible);
        if let Some(list) = layout.list {
            let mut lines = panel_header(t, usize::from(list.width), "SESSIONS", "").to_vec();
            lines.push(Line::default());
            let empty = fit("  No Claude sessions found.", usize::from(list.width));
            lines.push(Line::styled(empty, t.fg(Role::TextDim)));
            place_lines(frame, list, lines);
        }
        if let Some(preview) = layout.preview {
            place_lines(
                frame,
                preview,
                panel_header(t, usize::from(preview.width), "PREVIEW", "").to_vec(),
            );
        }
        if let (Some(x), Some(list)) = (layout.divider_x, layout.list) {
            for y in 0..list.height {
                frame.render_widget(
                    Line::styled("│", t.fg(Role::Border)),
                    Rect::new(x, list.y + y, 1, 1).intersection(area),
                );
            }
        }

        let bottom = row(area.height.saturating_sub(1));
        if self.message.is_empty() {
            frame.render_widget(bars::help_bar(t, cols), bottom);
        } else {
            frame.render_widget(bars::message_bar(t, cols, &self.message), bottom);
        }
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
    loop {
        if !app.pending_output.is_empty() {
            out.write_all(std::mem::take(&mut app.pending_output).as_bytes())?;
            out.flush()?;
        }
        if std::mem::take(&mut app.dirty) {
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
        assert_eq!(app.pending_output, OSC11_QUERY);
        assert!(rx.try_recv().is_err());
    }
}
