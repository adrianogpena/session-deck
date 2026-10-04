//! Centered popups drawn over the panels — port of `pickerOverlay`.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Clear;
use ratatui::Frame;

use crate::ansi::{fit, text_width};
use crate::theme::{Role, Theme};

/// A small centered list to choose from (↑↓, Enter, Esc), e.g. the agent for a new session.
pub fn render_picker(frame: &mut Frame, t: Theme, title: &str, items: &[String], index: usize) {
    let area = frame.area();
    let (cols, rows) = (usize::from(area.width), usize::from(area.height));
    let widest = items.iter().map(|i| text_width(i) + 8).max().unwrap_or(0);
    let width = cols
        .saturating_sub(4)
        .min((text_width(title) + 8).max(widest).max(30));
    if width < 4 {
        return;
    }
    let inner = width - 4;
    let max_items = rows.saturating_sub(6).max(1);
    let first = index
        .saturating_sub(max_items / 2)
        .min(items.len().saturating_sub(max_items));
    let shown = &items[first..items.len().min(first + max_items)];

    let surface = t.bg(Role::Surface);
    let border = surface.patch(t.fg(Role::Purple));
    let text = surface.patch(t.fg(Role::Text));
    let selected = Style::new()
        .bg(t.color(Role::Accent))
        .fg(t.color(Role::Bg))
        .add_modifier(Modifier::BOLD);

    let heading = fit(&format!(" {title} "), inner).trim_end().to_string();
    let top_fill = width.saturating_sub(3 + text_width(&heading));
    let mut lines = vec![Line::from(vec![
        Span::styled("╭─", border),
        Span::styled(heading, border.add_modifier(Modifier::BOLD)),
        Span::styled(format!("{}╮", "─".repeat(top_fill)), border),
    ])];
    for (i, item) in shown.iter().enumerate() {
        let style = if first + i == index { selected } else { text };
        lines.push(Line::from(vec![
            Span::styled("│", border),
            Span::styled(" ", surface),
            Span::styled(fit(&format!(" {item}"), inner), style),
            Span::styled(" ", surface),
            Span::styled("│", border),
        ]));
    }
    lines.push(Line::from(Span::styled(
        format!("╰{}╯", "─".repeat(width - 2)),
        border,
    )));

    let x = (cols - width) / 2;
    let y = rows.saturating_sub(lines.len()) / 2;
    for (i, line) in lines.into_iter().enumerate() {
        let rect = Rect::new(x as u16, (y + i) as u16, width as u16, 1).intersection(area);
        frame.render_widget(Clear, rect);
        frame.render_widget(line, rect);
    }
}
