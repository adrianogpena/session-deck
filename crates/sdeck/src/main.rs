use std::io::{stdout, IsTerminal, Read, Write};
use std::sync::mpsc;

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::{Hide, Show};
use ratatui::crossterm::event::{DisableFocusChange, DisableMouseCapture, EnableFocusChange};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::Terminal;
use sdeck::app::{run_loop, App, VERSION};
use sdeck::event::spawn_input_thread;
use sdeck::keys::RESET_AGENT_MODES;
use sdeck::theme::read_os_theme;
use sdeck_core::status::account::discover_accounts;
use sdeck_core::status::statusline::run_statusline_hook;
use sdeck_core::store::deck_config::{ctl_enabled, deck_config_path, read_deck_config};
use sdeck_core::store::deck_store::DeckStore;

/// Puts the real terminal back: also run from the panic hook, so a crash never leaves it raw.
fn restore_terminal() {
    let mut out = stdout();
    let _ = out.write_all(RESET_AGENT_MODES.as_bytes());
    let _ = execute!(
        out,
        DisableMouseCapture,
        DisableFocusChange,
        Show,
        LeaveAlternateScreen
    );
    let _ = out.write_all(b"\x1b[?1049l\x1b[?47l");
    let _ = out.flush();
    let _ = disable_raw_mode();
}

/// Restores the terminal when `main` returns or unwinds.
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> std::io::Result<Self> {
        enable_raw_mode()?;
        let guard = TerminalGuard;
        execute!(stdout(), EnterAlternateScreen, Hide, EnableFocusChange)?;
        Ok(guard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

fn main() -> anyhow::Result<()> {
    if std::env::args().skip(1).any(|a| a == "--version" || a == "-V") {
        println!("{VERSION}");
        return Ok(());
    }
    if std::env::args().nth(1).as_deref() == Some("statusline-hook") {
        let mut payload = String::new();
        let _ = std::io::stdin().read_to_string(&mut payload);
        println!("{}", run_statusline_hook(&payload));
        return Ok(());
    }
    if std::env::args().nth(1).as_deref() == Some("ctl") {
        let args: Vec<String> = std::env::args().skip(2).collect();
        std::process::exit(sdeck::ctl_client::run(&args));
    }
    if !std::io::stdin().is_terminal() {
        eprintln!("sdeck needs an interactive terminal.");
        std::process::exit(1);
    }
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        previous_hook(info);
    }));

    let (tx, rx) = mpsc::channel();
    let mut app = App::new(DeckStore::at_default_path(), tx.clone(), Some(read_os_theme));
    app.query_terminal_background = !cfg!(windows);
    let config = read_deck_config(&deck_config_path());
    let ctl_on = ctl_enabled(&config);
    app.start_background(discover_accounts(), config);
    // Outlives `app` (dropped last below), so `ctl.json` is only removed once sdeck is done.
    let ctl_server = if ctl_on { app.start_ctl_server() } else { None };
    #[cfg(windows)]
    app.enable_notifications(Box::new(
        sdeck_core::status::waiting_notifier::WindowsToastSender::new(),
    ));
    let _guard = TerminalGuard::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    spawn_input_thread(tx);
    let result = run_loop(&mut app, &mut terminal, &rx, &mut stdout());
    // Shutting down live sessions can still write to the screen, so do it before leaving it.
    drop(terminal);
    drop(app);
    drop(ctl_server);
    result
}
