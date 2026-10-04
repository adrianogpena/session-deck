use std::io::{stdout, IsTerminal, Write};
use std::sync::mpsc;

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::{Hide, Show};
use ratatui::crossterm::event::{DisableFocusChange, EnableFocusChange};
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
use sdeck_core::store::deck_config::{deck_config_path, read_deck_config};
use sdeck_core::store::deck_store::DeckStore;

/// Puts the real terminal back: also run from the panic hook, so a crash never leaves it raw.
fn restore_terminal() {
    let mut out = stdout();
    let _ = out.write_all(RESET_AGENT_MODES.as_bytes());
    let _ = execute!(out, DisableFocusChange, Show, LeaveAlternateScreen);
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
    app.start_background(discover_accounts(), read_deck_config(&deck_config_path()));
    let _guard = TerminalGuard::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    spawn_input_thread(tx);
    run_loop(&mut app, &mut terminal, &rx, &mut stdout())
}
