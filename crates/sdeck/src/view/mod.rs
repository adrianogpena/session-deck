//! Rendering, port of `view.ts`: functions from app state to ratatui lines.

pub mod bars;

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
