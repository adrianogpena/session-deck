//! Plain-text fitting, port of the text helpers in `ansi.ts`. `fitAnsi` isn't ported: views build
//! styled ratatui spans, not SGR strings, and ratatui clips those itself. `renderTerm` becomes the
//! vt100 preview in 08.

use unicode_width::UnicodeWidthChar;

/// Display width of one character in terminal columns; control characters take none.
fn char_width(c: char) -> usize {
    if (c as u32) < 0x20 {
        0
    } else {
        c.width().unwrap_or(0)
    }
}

/// Display width in terminal columns of plain text (no escape sequences).
pub fn text_width(text: &str) -> usize {
    text.chars().map(char_width).sum()
}

/// Truncates (with `…`) or pads plain text to exactly `width` columns.
pub fn fit(text: &str, width: usize) -> String {
    let mut kept: Vec<char> = Vec::new();
    let mut w = 0;
    for c in text.chars() {
        if (c as u32) < 0x20 {
            continue;
        }
        let cw = char_width(c);
        if w + cw > width {
            // Make room for the ellipsis, measuring in columns (a wide character frees two).
            while w + 1 > width {
                match kept.pop() {
                    Some(last) => w -= char_width(last),
                    None => break,
                }
            }
            if w < width {
                kept.push('…');
                w += 1;
            }
            break;
        }
        kept.push(c);
        w += cw;
    }
    let mut result: String = kept.into_iter().collect();
    result.push_str(&" ".repeat(width.saturating_sub(w)));
    result
}

/// Truncates (with a leading `…`) or leaves plain text as-is, keeping the *tail* within `width`
/// columns. Used where the end of the text (e.g. a trailing cursor) must stay visible.
pub fn fit_tail(text: &str, width: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut w = 0;
    let mut start = chars.len();
    for i in (0..chars.len()).rev() {
        let cw = char_width(chars[i]);
        if w + cw > width {
            break;
        }
        w += cw;
        start = i;
    }
    if start == 0 {
        return text.to_string();
    }
    let mut kept = &chars[start..];
    while !kept.is_empty() && w + 1 > width {
        w -= char_width(kept[0]);
        kept = &kept[1..];
    }
    std::iter::once('…').chain(kept.iter().copied()).collect()
}

/// Collapses whitespace runs to single spaces and trims.
pub fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Word-wraps at spaces, keeping paragraphs; a word longer than `width` is cut. Measured in
/// characters, like the TS `length`.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let para = para.strip_suffix('\r').unwrap_or(para);
        let mut line = String::new();
        for word in para.split(' ') {
            if format!("{line} {word}").trim().chars().count() > width {
                if !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                }
                line = word.chars().take(width).collect();
            } else if line.is_empty() {
                line = word.to_string();
            } else {
                line = format!("{line} {word}");
            }
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_truncates_with_an_ellipsis_and_pads_to_the_exact_width() {
        assert_eq!(fit("hello world", 8), "hello w…");
        assert_eq!(fit("hi", 5), "hi   ");
        assert_eq!(text_width(&fit("日本語テキスト", 7)), 7);
    }

    #[test]
    fn fit_handles_zero_width() {
        assert_eq!(fit("hello", 0), "");
    }

    #[test]
    fn fit_tail_keeps_the_end_visible() {
        assert_eq!(fit_tail("abc", 5), "abc");
        assert_eq!(fit_tail("hello world", 6), "…world");
        assert_eq!(text_width(&fit_tail("日本語テキスト", 6)), 5);
    }

    #[test]
    fn one_line_collapses_whitespace() {
        assert_eq!(one_line("  a\n\tb  c "), "a b c");
    }

    #[test]
    fn wrap_breaks_at_spaces_keeps_paragraphs_and_cuts_long_words() {
        assert_eq!(wrap("the quick brown fox", 9), vec!["the quick", "brown fox"]);
        assert_eq!(wrap("a\r\nb", 5), vec!["a", "b"]);
        assert_eq!(wrap("abcdefghij", 4), vec!["abcd"]);
    }
}
