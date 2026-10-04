//! Rendering, port of `view.ts`: functions from app state to ratatui lines.

pub mod bars;
pub mod list_panel;
pub mod overlay;
pub mod overlays;
pub mod preview_panel;

use sdeck_core::discovery::git_status::GitStatus;

use crate::sessions::SessionStatus;

use ratatui::style::Modifier;
use ratatui::text::{Line, Span};

use crate::ansi::{fit, text_width};
use crate::theme::{Role, Theme};

/// A panel's title line (`SESSIONS`, `PREVIEW`) with an optional dim note, and its underline.
pub fn panel_header(t: Theme, width: usize, title: &str, note: &str) -> [Line<'static>; 2] {
    let note = if note.is_empty() {
        String::new()
    } else {
        format!(" {note}")
    };
    let title_width = text_width(title).min(width);
    [
        Line::from(vec![
            Span::styled(
                fit(title, title_width),
                t.fg(Role::Cyan).add_modifier(Modifier::BOLD),
            ),
            Span::styled(fit(&note, width - title_width), t.fg(Role::TextDim)),
        ]),
        Line::styled("─".repeat(width), t.fg(Role::Border)),
    ]
}

/// Everything a row or the preview header shows about one session.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionView {
    pub title: String,
    pub status: SessionStatus,
    /// Open in another terminal rather than here.
    pub elsewhere: bool,
    pub agent: String,
    pub time_label: String,
    pub cwd: String,
    pub id: Option<String>,
    /// Replaces the plain status text, e.g. the screen error ("sign-in failed · run /login").
    pub detail: Option<String>,
    /// `None` when `ui.gitStatus` is off, or the cwd isn't a git repo.
    pub git: Option<GitStatus>,
    /// The owning account's email local part, for a session of an account that isn't the active
    /// one (only set when more than one account exists).
    pub account_tag: Option<String>,
    /// The session belongs to an account other than the active one (only set when more than one
    /// account exists); its title is dimmed.
    pub other_account: bool,
}

/// Counts shown on a folder or project row.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GroupCounts {
    pub count: usize,
    pub running: usize,
    pub waiting: usize,
}
