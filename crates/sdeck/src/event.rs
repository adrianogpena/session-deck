//! Everything the main loop reacts to. Background threads (input, probes, and later PTY readers and
//! watchers) send [`AppEvent`]s over one channel; only the main loop touches app state.

use std::sync::mpsc::Sender;
use std::time::Duration;

use ratatui::crossterm::event::{self, Event};

use crate::git_status_tracker::GitStatuses;
use crate::keys::encode_event;
use crate::sessions::FoundSession;
use crate::theme::ThemeName;

#[derive(Debug, Clone, PartialEq)]
pub enum AppEvent {
    /// One burst of terminal input, re-encoded to the bytes a VT terminal sends (see `keys`).
    Input(String),
    Resize(u16, u16),
    /// Nothing arrived for a while: time-based state (message expiry, theme poll) gets a look.
    Tick,
    /// What an OS theme probe (`theme::read_os_theme`), run off the main thread, found.
    OsTheme(Option<ThemeName>),
    /// What a discovery run (`sessions::discover_found`), off the main thread, found on disk.
    Discovered(Vec<FoundSession>),
    /// What a git poll (`git_status_tracker::read_all`), off the main thread, read.
    GitStatuses(GitStatuses),
    /// The shared state file changed (the other front end, or us): names and flags may differ.
    StoreChanged,
}

/// Reads the real terminal on its own thread. Events already queued are sent as one `Input`, like a
/// stdin chunk in TS, so a pasted or fast-typed run (or a terminal reply such as OSC 11) stays
/// together. Ends when the receiver is gone or the terminal can't be read.
pub fn spawn_input_thread(tx: Sender<AppEvent>) {
    std::thread::spawn(move || {
        while let Ok(first) = event::read() {
            let mut chunk = String::new();
            let mut next = Some(first);
            while let Some(ev) = next.take() {
                if let Event::Resize(cols, rows) = ev {
                    if !chunk.is_empty() && tx.send(AppEvent::Input(std::mem::take(&mut chunk))).is_err() {
                        return;
                    }
                    if tx.send(AppEvent::Resize(cols, rows)).is_err() {
                        return;
                    }
                } else if let Some(bytes) = encode_event(&ev) {
                    chunk.push_str(&bytes);
                }
                if event::poll(Duration::ZERO).unwrap_or(false) {
                    next = event::read().ok();
                }
            }
            if !chunk.is_empty() && tx.send(AppEvent::Input(chunk)).is_err() {
                return;
            }
        }
    });
}
