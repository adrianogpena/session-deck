//! Draws a `vt100` screen into a ratatui buffer, port of `ansi.renderTerm`.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::Widget;

/// The screen as it is (pass one scrolled back with `LiveSession::screen_at` for the preview's own
/// scroll), cropped or padded to the area.
///
/// The agent's cursor is positioned by escape codes, not printed, so it's painted in here: as an
/// underline (the cursor style sdeck gives agents; vt100 doesn't track DECSCUSR), only when
/// `show_cursor` (keystrokes go to this session), never while scrolled back or hidden by the agent.
pub struct TermWidget<'a> {
    pub screen: &'a vt100::Screen,
    pub show_cursor: bool,
    /// Highlighted cells (row, column), first and last in reading order, drawn on `selection_bg`.
    pub selection: Option<((u16, u16), (u16, u16))>,
    pub selection_bg: Color,
    /// What the agent's default (unset) foreground and background are drawn as: the theme's, so they
    /// match the rest of the screen; `Color::Reset` for the terminal's own.
    pub default_fg: Color,
    pub default_bg: Color,
}

fn color(c: vt100::Color, default: Color) -> Color {
    match c {
        vt100::Color::Default => default,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

fn cell_style(cell: &vt100::Cell, default_fg: Color, default_bg: Color) -> Style {
    let mut style = Style::new()
        .fg(color(cell.fgcolor(), default_fg))
        .bg(color(cell.bgcolor(), default_bg));
    for (on, modifier) in [
        (cell.bold(), Modifier::BOLD),
        (cell.dim(), Modifier::DIM),
        (cell.italic(), Modifier::ITALIC),
        (cell.underline(), Modifier::UNDERLINED),
        (cell.inverse(), Modifier::REVERSED),
    ] {
        if on {
            style = style.add_modifier(modifier);
        }
    }
    style
}

impl Widget for TermWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let area = area.intersection(buf.area);
        let (rows, cols) = self.screen.size();
        let cursor = (self.show_cursor && self.screen.scrollback() == 0 && !self.screen.hide_cursor())
            .then(|| self.screen.cursor_position());
        let blank = Style::new().fg(self.default_fg).bg(self.default_bg);
        for y in 0..area.height {
            let mut col = 0u16;
            if y < rows {
                for x in 0..cols {
                    if col >= area.width {
                        break;
                    }
                    let Some(cell) = self.screen.cell(y, x) else {
                        break;
                    };
                    if cell.is_wide_continuation() {
                        continue;
                    }
                    let width = if cell.is_wide() { 2 } else { 1 };
                    let mut style = cell_style(cell, self.default_fg, self.default_bg);
                    if cursor == Some((y, x)) {
                        style = style.add_modifier(Modifier::UNDERLINED);
                    }
                    if self
                        .selection
                        .is_some_and(|(first, last)| (y, x) >= first && (y, x) <= last)
                    {
                        style = style.bg(self.selection_bg);
                    }
                    let target = &mut buf[(area.x + col, area.y + y)];
                    if col + width > area.width {
                        target.set_symbol(" ").set_style(style);
                        col += 1;
                        break;
                    }
                    let symbol = if cell.has_contents() { cell.contents() } else { " " };
                    target.set_symbol(symbol).set_style(style);
                    for hidden in 1..width {
                        buf[(area.x + col + hidden, area.y + y)].reset();
                    }
                    col += width;
                }
            }
            for x in col..area.width {
                buf[(area.x + x, area.y + y)].set_symbol(" ").set_style(blank);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(parser: &vt100::Parser, width: u16, height: u16, show_cursor: bool) -> Buffer {
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        TermWidget {
            screen: parser.screen(),
            show_cursor,
            selection: None,
            selection_bg: Color::Reset,
            default_fg: Color::Reset,
            default_bg: Color::Reset,
        }
        .render(buf.area, &mut buf);
        buf
    }

    fn row(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
    }

    #[test]
    fn crops_and_pads_the_screen_keeping_colors() {
        let mut parser = vt100::Parser::new(3, 10, 0);
        parser.process(b"\x1b[1;31mred\x1b[0m plain\r\n\x1b[38;2;1;2;3mrgb");
        let buf = render(&parser, 6, 4, false);
        assert_eq!(row(&buf, 0), "red pl");
        assert_eq!(row(&buf, 1), "rgb   ");
        assert_eq!(row(&buf, 3), "      ");
        assert_eq!(buf[(0, 0)].fg, Color::Indexed(1));
        assert!(buf[(0, 0)].modifier.contains(Modifier::BOLD));
        assert_eq!(buf[(0, 1)].fg, Color::Rgb(1, 2, 3));
        assert_eq!(buf[(4, 0)].fg, Color::Reset);
    }

    #[test]
    fn a_wide_character_that_doesnt_fit_becomes_a_space() {
        let mut parser = vt100::Parser::new(1, 10, 0);
        parser.process("ab日本".as_bytes());
        let buf = render(&parser, 5, 1, false);
        assert_eq!(buf[(2, 0)].symbol(), "日");
        assert_eq!(buf[(4, 0)].symbol(), " ");
    }

    #[test]
    fn the_cursor_is_drawn_only_when_shown_and_not_hidden_by_the_agent() {
        let mut parser = vt100::Parser::new(2, 10, 0);
        parser.process(b"ab");
        let cursor_cell = |buf: &Buffer| buf[(2, 0)].modifier.contains(Modifier::UNDERLINED);
        assert!(cursor_cell(&render(&parser, 10, 2, true)));
        assert!(!cursor_cell(&render(&parser, 10, 2, false)));
        parser.process(b"\x1b[?25l");
        assert!(!cursor_cell(&render(&parser, 10, 2, true)));
    }
}
