//! Port of `keys.ts`. Crossterm decodes the real terminal's input, but every event is re-encoded into
//! the byte string a VT terminal would send ([`encode_event`]), so the app keeps matching the TS key
//! names (`"\x1b[A"`, `"\x1bOQ"`, `"\x03"`, …) and attach/interact can forward the bytes to a PTY as-is.

use std::sync::LazyLock;

use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use regex::Regex;

/// Ctrl+Q, in every encoding the real terminal may use while attached: plain (0x11), kitty CSI-u, and
/// Windows Terminal's win32-input-mode (`ESC[Vk;Sc;Uc;Kd;Cs;Rc_`, Vk 81 = Q, key-down, a Ctrl bit set
/// in Cs, no Alt). ConPTY itself asks for win32-input-mode (`ESC[?9001h`), and that request reaches
/// the real terminal while attached, so after that plain 0x11 never arrives.
static DETACH_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\x11|\x1b\[113;5u|\x1b\[81;\d*;\d*;1;(\d+);\d*_").unwrap());
/// Same encodings as [`DETACH_PATTERN`], for Ctrl+K (0x0b, Vk 75 = K, kitty code point 107 = k).
static CHORD_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\x0b|\x1b\[107;5u|\x1b\[75;\d*;\d*;1;(\d+);\d*_").unwrap());
const CTRL_PRESSED: u32 = 0x04 | 0x08;
const ALT_PRESSED: u32 = 0x01 | 0x02;

/// Terminal modes an agent (or ConPTY) may switch on in the real terminal while attached.
pub const RESET_AGENT_MODES: &str =
    "\x1b[?9001l\x1b[?2004l\x1b[?1004l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1l\x1b[<u\x1b[0m";

/// Basic click/wheel reporting (mode 1000), SGR-encoded (mode 1006). Crossterm reads console input
/// records on Windows, so sdeck's own mouse toggle goes through crossterm's `EnableMouseCapture`;
/// these raw sequences are for restoring the real terminal after an attached agent.
pub const ENABLE_MOUSE: &str = "\x1b[?1000h\x1b[?1006h";
pub const DISABLE_MOUSE: &str = "\x1b[?1000l\x1b[?1006l";

/// Window focus reporting (mode 1004): `ESC[I` on focus-in, `ESC[O` on focus-out. See [`ENABLE_MOUSE`]
/// for why sdeck itself enables it through crossterm (`EnableFocusChange`).
pub const ENABLE_FOCUS_REPORTING: &str = "\x1b[?1004h";

fn match_ctrl_key(data: &str, pattern: &Regex) -> Option<(usize, usize)> {
    pattern.captures_iter(data).find_map(|c| {
        let m = c.get(0)?;
        match c.get(1) {
            None => Some((m.start(), m.end())),
            Some(state) => {
                let state: u32 = state.as_str().parse().unwrap_or(0);
                (state & CTRL_PRESSED != 0 && state & ALT_PRESSED == 0).then_some((m.start(), m.end()))
            }
        }
    })
}

/// Byte index where the detach key starts in `data`.
pub fn find_detach_key(data: &str) -> Option<usize> {
    match_ctrl_key(data, &DETACH_PATTERN).map(|(start, _)| start)
}

/// Where the Ctrl+K chord's prefix starts and ends in `data` (byte indices). `end` matters because a
/// chord typed quickly can arrive with its resolving key already in the same chunk, right after the
/// match.
pub fn find_chord_key(data: &str) -> Option<(usize, usize)> {
    match_ctrl_key(data, &CHORD_PATTERN)
}

/// Byte index where an unmodified, key-down press of `ch` (a single ASCII letter) starts in `data`:
/// the plain character itself, or Windows Terminal's win32-input-mode encoding of it.
pub fn find_plain_key(data: &str, ch: char) -> Option<usize> {
    let re = Regex::new(&format!(
        r"{}|\x1b\[\d+;\d*;{};1;\d*;\d*_",
        regex::escape(&ch.to_string()),
        ch as u32
    ))
    .unwrap();
    re.find(data).map(|m| m.start())
}

/// Strips `ESC[I`/`ESC[O` (focus-in/out) out of raw input. `None` when `data` has neither; several in
/// one chunk: the last one wins.
pub fn extract_focus_events(data: &str) -> (Option<bool>, String) {
    static FOCUS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\[[IO]").unwrap());
    let focused = FOCUS.find_iter(data).last().map(|m| m.as_str() == "\x1b[I");
    (focused, FOCUS.replace_all(data, "").into_owned())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wheel {
    Up,
    Down,
}

/// Whether `data` is one SGR mouse report (`ESC [ < Cb ; Cx ; Cy (M|m)`): `Some(None)` for a plain
/// click/release, `Some(Some(dir))` for a wheel notch.
pub fn parse_mouse_sequence(data: &str) -> Option<Option<Wheel>> {
    static MOUSE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\x1b\[<(\d+);(\d+);(\d+)[Mm]$").unwrap());
    let cb: u32 = MOUSE.captures(data)?[1].parse().ok()?;
    if cb & 0x40 == 0 {
        return Some(None);
    }
    Some(Some(if cb & 1 == 0 { Wheel::Up } else { Wheel::Down }))
}

/// One input chunk as individual keys. CSI (`ESC [ … final`) and SS3 (`ESC O x`) sequences stay whole.
pub fn split_keys(data: &str) -> Vec<String> {
    let chars: Vec<char> = data.chars().collect();
    let mut keys = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '\x1b' || i + 1 >= chars.len() {
            keys.push(chars[i].to_string());
        } else if chars[i + 1] == 'O' && i + 2 < chars.len() {
            keys.push(chars[i..i + 3].iter().collect());
            i += 2;
        } else if chars[i + 1] == '[' {
            let mut end = i + 2;
            while end < chars.len() && !('\x40'..='\x7e').contains(&chars[end]) {
                end += 1;
            }
            let last = end.min(chars.len() - 1);
            keys.push(chars[i..=last].iter().collect());
            i = end;
        } else {
            keys.push("\x1b".to_string()); // lone Esc (or Alt+key, which we don't bind)
        }
        i += 1;
    }
    keys
}

/// xterm modifier parameter: 1 + Shift(1) + Alt(2) + Ctrl(4). 1 means none.
fn modifier_param(m: KeyModifiers) -> u8 {
    1 + u8::from(m.contains(KeyModifiers::SHIFT))
        + 2 * u8::from(m.contains(KeyModifiers::ALT))
        + 4 * u8::from(m.contains(KeyModifiers::CONTROL))
}

/// `ESC [ <final>` unmodified, `ESC [ 1 ; <m> <final>` with modifiers (arrows, Home/End).
fn csi_letter(final_char: char, m: KeyModifiers) -> String {
    match modifier_param(m) {
        1 => format!("\x1b[{final_char}"),
        p => format!("\x1b[1;{p}{final_char}"),
    }
}

/// `ESC [ <n> ~`, or `ESC [ <n> ; <m> ~` with modifiers (PgUp/PgDn, Insert/Delete, F5+).
fn csi_tilde(n: u8, m: KeyModifiers) -> String {
    match modifier_param(m) {
        1 => format!("\x1b[{n}~"),
        p => format!("\x1b[{n};{p}~"),
    }
}

fn encode_key(key: &KeyEvent) -> Option<String> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    let m = key.modifiers;
    Some(match key.code {
        KeyCode::Char(c) if m.contains(KeyModifiers::CONTROL) => {
            let ctrl = match c.to_ascii_lowercase() {
                l @ 'a'..='z' => char::from(l as u8 & 0x1f),
                ' ' | '@' | '2' => '\0',
                '[' => '\x1b',
                '\\' => '\x1c',
                ']' => '\x1d',
                '^' | '6' => '\x1e',
                '_' | '-' => '\x1f',
                _ => c,
            };
            if m.contains(KeyModifiers::ALT) {
                format!("\x1b{ctrl}")
            } else {
                ctrl.to_string()
            }
        }
        KeyCode::Char(c) if m.contains(KeyModifiers::ALT) => format!("\x1b{c}"),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "\r".into(),
        KeyCode::Tab => "\t".into(),
        KeyCode::BackTab => "\x1b[Z".into(),
        KeyCode::Backspace => "\x7f".into(),
        KeyCode::Esc => "\x1b".into(),
        KeyCode::Up => csi_letter('A', m),
        KeyCode::Down => csi_letter('B', m),
        KeyCode::Right => csi_letter('C', m),
        KeyCode::Left => csi_letter('D', m),
        KeyCode::Home => csi_letter('H', m),
        KeyCode::End => csi_letter('F', m),
        KeyCode::Insert => csi_tilde(2, m),
        KeyCode::Delete => csi_tilde(3, m),
        KeyCode::PageUp => csi_tilde(5, m),
        KeyCode::PageDown => csi_tilde(6, m),
        KeyCode::F(n @ 1..=4) => {
            let letter = char::from(b'P' + n - 1);
            match modifier_param(m) {
                1 => format!("\x1bO{letter}"),
                p => format!("\x1b[1;{p}{letter}"),
            }
        }
        KeyCode::F(n @ 5..=12) => csi_tilde([15, 17, 18, 19, 20, 21, 23, 24][usize::from(n - 5)], m),
        _ => return None,
    })
}

/// SGR encoding (`ESC [ < Cb ; Cx ; Cy M|m`, 1-based) of a click or wheel notch; motion isn't reported
/// (mode 1000 doesn't either).
fn encode_mouse(ev: &MouseEvent) -> Option<String> {
    let button = |b: MouseButton| match b {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    };
    let (cb, release) = match ev.kind {
        MouseEventKind::Down(b) => (button(b), false),
        MouseEventKind::Up(b) => (button(b), true),
        MouseEventKind::ScrollUp => (64, false),
        MouseEventKind::ScrollDown => (65, false),
        _ => return None,
    };
    let m = ev.modifiers;
    let cb = cb
        + 4 * u32::from(m.contains(KeyModifiers::SHIFT))
        + 8 * u32::from(m.contains(KeyModifiers::ALT))
        + 16 * u32::from(m.contains(KeyModifiers::CONTROL));
    Some(format!(
        "\x1b[<{cb};{};{}{}",
        ev.column + 1,
        ev.row + 1,
        if release { 'm' } else { 'M' }
    ))
}

/// The bytes a VT terminal would have sent for `ev` — the TS key name. `None` for events with no
/// input bytes (resize, key release, mouse motion).
pub fn encode_event(ev: &Event) -> Option<String> {
    match ev {
        Event::Key(key) => encode_key(key),
        Event::Mouse(mouse) => encode_mouse(mouse),
        Event::FocusGained => Some("\x1b[I".into()),
        Event::FocusLost => Some("\x1b[O".into()),
        Event::Paste(text) => Some(text.clone()),
        Event::Resize(..) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyEventState;

    fn keys(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn split_keys_separates_typed_characters_and_keeps_escape_sequences_whole() {
        assert_eq!(split_keys(">>>"), keys(&[">", ">", ">"]));
        assert_eq!(split_keys("0?"), keys(&["0", "?"]));
        assert_eq!(split_keys("\x1b[Aj\x1b[B"), keys(&["\x1b[A", "j", "\x1b[B"]));
        assert_eq!(split_keys("\x1bOQ\x1b[12~"), keys(&["\x1bOQ", "\x1b[12~"]));
        assert_eq!(split_keys("\x1b"), keys(&["\x1b"]));
        assert_eq!(split_keys("é🙂"), keys(&["é", "🙂"]));
    }

    #[test]
    fn split_keys_keeps_an_unterminated_csi_whole() {
        assert_eq!(split_keys("\x1b[1;"), keys(&["\x1b[1;"]));
    }

    #[test]
    fn find_detach_key_recognizes_ctrl_q_in_every_encoding_and_only_ctrl_q() {
        assert_eq!(find_detach_key("\x11"), Some(0));
        assert_eq!(find_detach_key("\x1b[113;5u"), Some(0));
        assert_eq!(find_detach_key("ab\x1b[81;16;17;1;8;1_"), Some(2));
        assert_eq!(find_detach_key("\x1b[81;16;113;1;0;1_"), None);
        assert_eq!(find_detach_key("\x1b[81;16;17;0;8;1_"), None);
        assert_eq!(find_detach_key("\x1b[81;16;0;1;10;1_"), None);
    }

    #[test]
    fn find_chord_key_recognizes_ctrl_k_in_every_encoding_and_only_ctrl_k() {
        assert_eq!(find_chord_key("\x0b"), Some((0, 1)));
        assert_eq!(find_chord_key("\x1b[107;5u"), Some((0, 8)));
        assert_eq!(find_chord_key("ab\x1b[75;16;17;1;8;1_"), Some((2, 19)));
        assert_eq!(find_chord_key("\x1b[75;16;107;1;0;1_"), None);
        assert_eq!(find_chord_key("\x1b[75;16;17;0;8;1_"), None);
        assert_eq!(find_chord_key("\x1b[75;16;0;1;10;1_"), None);
    }

    #[test]
    fn find_chord_key_exposes_where_its_match_ends() {
        let (_, end) = find_chord_key("\x0bN").unwrap();
        assert_eq!(end, 1);
        assert_eq!(find_plain_key(&"\x0bN"[end..], 'N'), Some(0));
    }

    #[test]
    fn find_plain_key_recognizes_an_unmodified_key_press_in_every_encoding() {
        assert_eq!(find_plain_key("n", 'n'), Some(0));
        assert_eq!(find_plain_key("ab n", 'n'), Some(3));
        assert_eq!(find_plain_key("\x1b[78;49;110;1;0;1_", 'n'), Some(0));
        assert_eq!(find_plain_key("\x1b[78;49;78;1;8;1_", 'N'), Some(0));
        assert_eq!(find_plain_key("\x1b[78;49;110;0;0;1_", 'n'), None);
        assert_eq!(find_plain_key("x", 'n'), None);
    }

    #[test]
    fn split_keys_keeps_an_sgr_mouse_report_whole() {
        assert_eq!(
            split_keys("\x1b[<64;10;5M\x1b[<65;10;5M"),
            keys(&["\x1b[<64;10;5M", "\x1b[<65;10;5M"])
        );
    }

    #[test]
    fn parse_mouse_sequence_recognizes_wheel_direction_and_plain_clicks() {
        assert_eq!(parse_mouse_sequence("\x1b[<64;10;5M"), Some(Some(Wheel::Up)));
        assert_eq!(parse_mouse_sequence("\x1b[<65;10;5M"), Some(Some(Wheel::Down)));
        assert_eq!(parse_mouse_sequence("\x1b[<68;10;5M"), Some(Some(Wheel::Up)));
        assert_eq!(parse_mouse_sequence("\x1b[<69;10;5M"), Some(Some(Wheel::Down)));
        assert_eq!(parse_mouse_sequence("\x1b[<0;10;5M"), Some(None));
        assert_eq!(parse_mouse_sequence("\x1b[<0;10;5m"), Some(None));
        assert_eq!(parse_mouse_sequence("\x1b[A"), None);
        assert_eq!(parse_mouse_sequence("n"), None);
    }

    #[test]
    fn extract_focus_events_strips_focus_and_reports_the_last_one() {
        assert_eq!(extract_focus_events("\x1b[I"), (Some(true), String::new()));
        assert_eq!(extract_focus_events("\x1b[O"), (Some(false), String::new()));
        assert_eq!(extract_focus_events("n"), (None, "n".into()));
        assert_eq!(
            extract_focus_events("a\x1b[Ob\x1b[Ic"),
            (Some(true), "abc".into())
        );
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    #[test]
    fn encode_event_produces_the_ts_key_names() {
        let none = KeyModifiers::NONE;
        assert_eq!(encode_event(&key(KeyCode::Char('j'), none)).unwrap(), "j");
        assert_eq!(
            encode_event(&key(KeyCode::Char('N'), KeyModifiers::SHIFT)).unwrap(),
            "N"
        );
        assert_eq!(
            encode_event(&key(KeyCode::Char('c'), KeyModifiers::CONTROL)).unwrap(),
            "\x03"
        );
        assert_eq!(
            encode_event(&key(KeyCode::Char('q'), KeyModifiers::CONTROL)).unwrap(),
            "\x11"
        );
        assert_eq!(encode_event(&key(KeyCode::Up, none)).unwrap(), "\x1b[A");
        assert_eq!(
            encode_event(&key(KeyCode::Down, KeyModifiers::SHIFT)).unwrap(),
            "\x1b[1;2B"
        );
        assert_eq!(encode_event(&key(KeyCode::F(2), none)).unwrap(), "\x1bOQ");
        assert_eq!(encode_event(&key(KeyCode::F(3), none)).unwrap(), "\x1bOR");
        assert_eq!(encode_event(&key(KeyCode::F(5), none)).unwrap(), "\x1b[15~");
        assert_eq!(encode_event(&key(KeyCode::PageUp, none)).unwrap(), "\x1b[5~");
        assert_eq!(encode_event(&key(KeyCode::Home, none)).unwrap(), "\x1b[H");
        assert_eq!(encode_event(&key(KeyCode::Enter, none)).unwrap(), "\r");
        assert_eq!(encode_event(&key(KeyCode::Esc, none)).unwrap(), "\x1b");
        assert_eq!(
            encode_event(&key(KeyCode::Char('x'), KeyModifiers::ALT)).unwrap(),
            "\x1bx"
        );
    }

    #[test]
    fn encode_event_ignores_key_release() {
        let ev = Event::Key(KeyEvent {
            code: KeyCode::Char('j'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Release,
            state: KeyEventState::NONE,
        });
        assert_eq!(encode_event(&ev), None);
    }

    #[test]
    fn encode_event_round_trips_mouse_and_focus_through_the_ts_parsers() {
        let wheel = |kind, modifiers| {
            Event::Mouse(MouseEvent {
                kind,
                column: 9,
                row: 4,
                modifiers,
            })
        };
        let up = encode_event(&wheel(MouseEventKind::ScrollUp, KeyModifiers::NONE)).unwrap();
        assert_eq!(up, "\x1b[<64;10;5M");
        assert_eq!(parse_mouse_sequence(&up), Some(Some(Wheel::Up)));
        let down = encode_event(&wheel(MouseEventKind::ScrollDown, KeyModifiers::SHIFT)).unwrap();
        assert_eq!(parse_mouse_sequence(&down), Some(Some(Wheel::Down)));
        let click = encode_event(&wheel(MouseEventKind::Up(MouseButton::Left), KeyModifiers::NONE)).unwrap();
        assert_eq!(click, "\x1b[<0;10;5m");
        assert_eq!(
            encode_event(&wheel(MouseEventKind::Moved, KeyModifiers::NONE)),
            None
        );
        assert_eq!(
            extract_focus_events(&encode_event(&Event::FocusLost).unwrap()).0,
            Some(false)
        );
    }
}
