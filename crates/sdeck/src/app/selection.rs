//! Click-drag text selection in the preview. With mouse reporting on, the terminal can't select
//! text itself, so sdeck highlights the dragged cells and copies them to the clipboard (OSC 52)
//! on release. Double-click picks a word, triple-click a line.

use std::time::{Duration, Instant};

use ratatui::layout::Rect;

use super::App;
use crate::keys::{MouseKind, MouseReport};
use crate::layout::PANEL_HEADER_ROWS;

const MULTI_CLICK: Duration = Duration::from_millis(400);

/// A (row, column) cell of the previewed screen, 0-based from its top-left.
pub(super) type Cell = (u16, u16);

/// The cells between `anchor` and `head` in reading order, inclusive, on the screen of session
/// `uid` scrolled back by `scroll`. Moving the scroll or the selected session drops it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Selection {
    pub uid: u64,
    pub scroll: usize,
    pub anchor: Cell,
    pub head: Cell,
    dragging: bool,
    /// What was copied, to tell when the screen under the highlight has moved on.
    copied: String,
}

impl Selection {
    /// `(first, last)` in reading order.
    pub fn ordered(&self) -> (Cell, Cell) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }
}

/// The last left press, for telling a double or triple click from a single one.
#[derive(Debug, Clone, Copy)]
pub(super) struct Click {
    at: Instant,
    cell: Cell,
    count: u8,
}

#[derive(PartialEq)]
enum CharClass {
    Blank,
    Word,
    Other,
}

fn class_of(screen: &vt100::Screen, (row, col): Cell) -> CharClass {
    match screen.cell(row, col).map(vt100::Cell::contents) {
        Some(text) => match text.chars().next() {
            None => CharClass::Blank,
            Some(c) if c.is_whitespace() => CharClass::Blank,
            Some(c) if c.is_alphanumeric() || c == '_' => CharClass::Word,
            Some(_) => CharClass::Other,
        },
        None => CharClass::Blank,
    }
}

/// The leading cell of the character at `cell` (a wide character's second cell belongs to it).
fn lead_cell(screen: &vt100::Screen, (row, col): Cell) -> Cell {
    match screen.cell(row, col) {
        Some(c) if c.is_wide_continuation() && col > 0 => (row, col - 1),
        _ => (row, col),
    }
}

/// The run of same-class cells around `cell`, within its row.
fn word_around(screen: &vt100::Screen, cell: Cell) -> (Cell, Cell) {
    let (row, col) = lead_cell(screen, cell);
    let (_, cols) = screen.size();
    let class = class_of(screen, (row, col));
    let mut start = col;
    while start > 0 && class_of(screen, (row, start - 1)) == class {
        start -= 1;
    }
    let mut end = col;
    while end + 1 < cols && class_of(screen, (row, end + 1)) == class {
        end += 1;
    }
    ((row, start), (row, end))
}

/// The text under the selection: wrapped rows join, other rows break, trailing blanks drop.
pub(super) fn selected_text(screen: &vt100::Screen, sel: &Selection) -> String {
    let (first, last) = sel.ordered();
    let (_, cols) = screen.size();
    let wide_last = screen.cell(last.0, last.1).is_some_and(vt100::Cell::is_wide);
    let end_col = (last.1 + 1 + u16::from(wide_last)).min(cols);
    let text = screen.contents_between(first.0, first.1, last.0, end_col);
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    lines.join("\n").trim_end().to_string()
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, b)| acc | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(BASE64[(n >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// OSC 52: asks the terminal to put `text` on the system clipboard.
fn clipboard_sequence(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64(text.as_bytes()))
}

impl App {
    /// Where the selected live session's screen is drawn.
    fn preview_body(&self) -> Option<Rect> {
        let preview = self.layout().preview?;
        Some(Rect {
            y: preview.y + PANEL_HEADER_ROWS,
            height: preview.height.saturating_sub(PANEL_HEADER_ROWS),
            ..preview
        })
    }

    /// The uid of the selected session if its agent runs here (so its screen is what's drawn).
    fn selectable_session(&self) -> Option<u64> {
        let s = self.selected_session()?;
        s.live.as_ref().filter(|l| !l.exited).map(|_| s.uid)
    }

    /// The screen cell under a terminal position; `None` outside the preview's screen unless
    /// `clamp`, which pulls it onto the nearest cell.
    fn screen_cell(&self, report: &MouseReport, clamp: bool) -> Option<Cell> {
        let body = self.preview_body()?;
        let uid = self.selectable_session()?;
        let live = self.session_by_uid(uid)?.live.as_ref()?;
        let (rows, cols) = live.screen().size();
        let (rows, cols) = (rows.min(body.height), cols.min(body.width));
        if rows == 0 || cols == 0 {
            return None;
        }
        let inside = report.col >= body.x
            && report.col < body.x + cols
            && report.row >= body.y
            && report.row < body.y + rows;
        if !inside && !clamp {
            return None;
        }
        let row = report.row.saturating_sub(body.y).min(rows - 1);
        let col = report.col.saturating_sub(body.x).min(cols - 1);
        Some(lead_cell(&live.screen_at(self.preview_scroll), (row, col)))
    }

    /// The highlighted range for the previewed session, in reading order.
    pub(super) fn selection_range(&self) -> Option<(Cell, Cell)> {
        let sel = self.selection.as_ref()?;
        (Some(sel.uid) == self.selectable_session() && sel.scroll == self.preview_scroll)
            .then(|| sel.ordered())
    }

    /// Output reached session `uid`: a finished selection goes once the text under it has changed
    /// (the chat scrolled), since the highlight would otherwise stay where the words were.
    pub(super) fn drop_stale_selection(&mut self, uid: u64) {
        let Some(sel) = self.selection.as_ref().filter(|s| s.uid == uid && !s.dragging) else {
            return;
        };
        let now_text = self
            .session_by_uid(uid)
            .and_then(|s| s.live.as_ref())
            .map(|l| selected_text(&l.screen_at(sel.scroll), sel));
        if now_text.as_deref() != Some(sel.copied.as_str()) {
            self.selection = None;
            self.dirty = true;
        }
    }

    /// A left press, drag or release in the terminal. Returns without effect for anything else.
    pub(super) fn on_mouse_select(&mut self, report: &MouseReport, now: Instant) {
        match report.kind {
            MouseKind::LeftDown => self.begin_selection(report, now),
            MouseKind::LeftDrag => {
                if self.selection.as_ref().is_some_and(|s| s.dragging) {
                    if let Some(cell) = self.screen_cell(report, true) {
                        if let Some(sel) = self.selection.as_mut() {
                            sel.head = cell;
                        }
                        self.dirty = true;
                    }
                }
            }
            MouseKind::LeftUp => self.finish_selection(now),
            MouseKind::Wheel(_) | MouseKind::Other => {}
        }
    }

    fn begin_selection(&mut self, report: &MouseReport, now: Instant) {
        self.selection = None;
        self.dirty = true;
        let (Some(uid), Some(cell)) = (self.selectable_session(), self.screen_cell(report, false)) else {
            self.last_click = None;
            return;
        };
        let count = match self.last_click {
            Some(prev) if now.duration_since(prev.at) <= MULTI_CLICK && prev.cell == cell => {
                prev.count % 3 + 1
            }
            _ => 1,
        };
        self.last_click = Some(Click { at: now, cell, count });
        let scroll = self.preview_scroll;
        let mut sel = Selection {
            uid,
            scroll,
            anchor: cell,
            head: cell,
            dragging: count == 1,
            copied: String::new(),
        };
        if count > 1 {
            let Some(live) = self.session_by_uid(uid).and_then(|s| s.live.as_ref()) else {
                return;
            };
            let screen = live.screen_at(scroll);
            let (_, cols) = screen.size();
            (sel.anchor, sel.head) = if count == 2 {
                word_around(&screen, cell)
            } else {
                ((cell.0, 0), (cell.0, cols.saturating_sub(1)))
            };
            self.selection = Some(sel);
            self.copy_selection(now);
            return;
        }
        self.selection = Some(sel);
    }

    fn finish_selection(&mut self, now: Instant) {
        let Some(sel) = self.selection.as_mut().filter(|s| s.dragging) else {
            return;
        };
        sel.dragging = false;
        if sel.anchor == sel.head {
            self.selection = None;
        } else {
            self.copy_selection(now);
        }
        self.dirty = true;
    }

    /// Puts the selected text on the clipboard; drops the selection if there's nothing in it.
    fn copy_selection(&mut self, now: Instant) {
        let Some(sel) = self.selection.clone() else {
            return;
        };
        let text = self
            .session_by_uid(sel.uid)
            .and_then(|s| s.live.as_ref())
            .map(|l| selected_text(&l.screen_at(sel.scroll), &sel))
            .unwrap_or_default();
        if text.is_empty() {
            self.selection = None;
            return;
        }
        let sequence = clipboard_sequence(&text);
        self.emit(&sequence);
        let count = text.chars().count();
        if let Some(sel) = self.selection.as_mut() {
            sel.copied = text;
        }
        self.flash(
            format!("Copied {count} character{}", if count == 1 { "" } else { "s" }),
            now,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen_with(text: &str) -> vt100::Parser {
        let mut parser = vt100::Parser::new(4, 12, 0);
        parser.process(text.as_bytes());
        parser
    }

    fn select(anchor: Cell, head: Cell) -> Selection {
        Selection {
            uid: 1,
            scroll: 0,
            anchor,
            head,
            dragging: false,
            copied: String::new(),
        }
    }

    #[test]
    fn base64_matches_the_standard_alphabet_and_padding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64("é".as_bytes()), "w6k=");
    }

    #[test]
    fn clipboard_sequence_is_an_osc_52_write() {
        assert_eq!(clipboard_sequence("hi"), "\x1b]52;c;aGk=\x07");
    }

    #[test]
    fn selected_text_covers_the_cells_in_reading_order_and_trims_blanks() {
        let parser = screen_with("hello world\r\nsecond  line\r\nthird");
        let screen = parser.screen();
        assert_eq!(selected_text(screen, &select((0, 6), (0, 10))), "world");
        assert_eq!(selected_text(screen, &select((0, 6), (1, 5))), "world\nsecond");
        assert_eq!(selected_text(screen, &select((1, 5), (0, 6))), "world\nsecond");
        assert_eq!(selected_text(screen, &select((0, 0), (0, 11))), "hello world");
        assert_eq!(selected_text(screen, &select((2, 0), (3, 11))), "third");
    }

    #[test]
    fn selected_text_joins_rows_that_only_wrapped() {
        let parser = screen_with("abcdefghijklmnop");
        assert_eq!(
            selected_text(parser.screen(), &select((0, 0), (1, 3))),
            "abcdefghijklmnop"
        );
    }

    #[test]
    fn selected_text_includes_a_wide_character_at_the_end() {
        let parser = screen_with("a日本");
        assert_eq!(selected_text(parser.screen(), &select((0, 0), (0, 3))), "a日本");
        assert_eq!(selected_text(parser.screen(), &select((0, 1), (0, 2))), "日");
    }

    #[test]
    fn word_around_picks_a_run_of_the_same_kind() {
        let mut parser = vt100::Parser::new(1, 20, 0);
        parser.process(b"foo_bar.baz  x");
        let screen = parser.screen();
        assert_eq!(word_around(screen, (0, 5)), ((0, 0), (0, 6)));
        assert_eq!(word_around(screen, (0, 7)), ((0, 7), (0, 7)));
        assert_eq!(word_around(screen, (0, 10)), ((0, 8), (0, 10)));
        assert_eq!(word_around(screen, (0, 11)), ((0, 11), (0, 12)));
    }

    fn mouse(f: &mut crate::app::test_fixture::Fixture, cb: u32, x: u16, y: u16, release: bool) {
        f.key(&format!(
            "\x1b[<{cb};{};{}{}",
            x + 1,
            y + 1,
            if release { 'm' } else { 'M' }
        ));
    }

    fn live_row_of(f: &crate::app::test_fixture::Fixture, uid: u64, text: &str) -> (u16, u16) {
        let screen = f.session(uid).unwrap().live.as_ref().unwrap().screen();
        let (rows, cols) = screen.size();
        screen
            .rows(0, cols)
            .enumerate()
            .take(usize::from(rows))
            .find_map(|(y, row)| row.find(text).map(|x| (y as u16, x as u16)))
            .unwrap()
    }

    fn typed_session() -> (crate::app::test_fixture::Fixture, u64) {
        let mut f = crate::app::test_fixture::fixture();
        let uid = f.add(Some("abc"), true);
        f.app.set_size(100, 14);
        f.key("i");
        f.wait_for_screen(uid, "> ");
        f.key("hello world\r");
        f.wait_for_screen(uid, "echo: hello world");
        f.app.pending_output.clear();
        (f, uid)
    }

    #[test]
    fn dragging_across_the_preview_highlights_and_copies_the_text() {
        let (mut f, uid) = typed_session();
        let body = f.app.preview_body().unwrap();
        let (row, col) = live_row_of(&f, uid, "hello world");
        mouse(&mut f, 0, body.x + col, body.y + row, false);
        mouse(&mut f, 32, body.x + col + 4, body.y + row, false);
        assert_eq!(f.app.selection_range(), Some(((row, col), (row, col + 4))));
        assert!(f.app.pending_output.is_empty(), "nothing is copied until release");
        mouse(&mut f, 0, body.x + col + 4, body.y + row, true);
        assert_eq!(f.app.pending_output, clipboard_sequence("hello").into_bytes());
        assert_eq!(f.app.message, "Copied 5 characters");
        assert_eq!(f.app.selection_range(), Some(((row, col), (row, col + 4))));
        f.key("x");
        assert_eq!(f.app.selection_range(), None, "a key drops the highlight");
    }

    #[test]
    fn a_plain_click_copies_nothing_and_a_double_click_copies_the_word() {
        let (mut f, uid) = typed_session();
        let body = f.app.preview_body().unwrap();
        let (row, col) = live_row_of(&f, uid, "hello world");
        let (x, y) = (body.x + col + 7, body.y + row);
        mouse(&mut f, 0, x, y, false);
        mouse(&mut f, 0, x, y, true);
        assert_eq!(f.app.selection_range(), None);
        assert!(f.app.pending_output.is_empty());
        mouse(&mut f, 0, x, y, false);
        assert_eq!(f.app.pending_output, clipboard_sequence("world").into_bytes());
        mouse(&mut f, 0, x, y, true);
        f.app.pending_output.clear();
        mouse(&mut f, 0, x, y, false);
        assert_eq!(
            f.app.pending_output,
            clipboard_sequence("> hello world").into_bytes()
        );
    }

    #[test]
    fn a_press_outside_the_preview_or_a_wheel_notch_clears_the_selection() {
        let (mut f, uid) = typed_session();
        let body = f.app.preview_body().unwrap();
        let (row, col) = live_row_of(&f, uid, "hello world");
        mouse(&mut f, 0, body.x + col, body.y + row, false);
        mouse(&mut f, 32, body.x + col + 2, body.y + row, false);
        mouse(&mut f, 0, body.x + col + 2, body.y + row, true);
        assert!(f.app.selection_range().is_some());
        mouse(&mut f, 64, body.x + col, body.y + row, false);
        assert_eq!(f.app.selection_range(), None);
        mouse(&mut f, 0, body.x + col, body.y + row, false);
        mouse(&mut f, 0, 2, 5, false);
        assert_eq!(f.app.selection_range(), None);
    }

    #[test]
    fn a_finished_selection_goes_once_output_moves_the_text_but_not_before() {
        let (mut f, uid) = typed_session();
        let body = f.app.preview_body().unwrap();
        let (row, col) = live_row_of(&f, uid, "hello world");
        mouse(&mut f, 0, body.x + col, body.y + row, false);
        mouse(&mut f, 32, body.x + col + 4, body.y + row, false);
        mouse(&mut f, 0, body.x + col + 4, body.y + row, true);
        let live_id = f.session(uid).unwrap().live.as_ref().unwrap().id;
        f.app
            .on_pty_output(live_id, b"\x1b[20;1Hunrelated", Instant::now());
        assert!(f.app.selection_range().is_some(), "same text under the highlight");
        f.app.on_pty_output(
            live_id,
            b"























",
            Instant::now(),
        );
        assert_eq!(f.app.selection_range(), None);
    }
}
