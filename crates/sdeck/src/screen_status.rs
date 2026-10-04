//! Port of `screenStatus.ts`: errors only visible on the agent's screen. Claude's status files say
//! nothing about a failed sign-in; the session just sits there.

use std::sync::LazyLock;

use regex::Regex;

/// Checked against the last lines only, so a message that scrolled away (e.g. after a successful
/// `/login`) no longer counts.
pub const SCREEN_ERROR_LINES: usize = 25;

static SCREEN_ERRORS: LazyLock<[(Regex, &str); 3]> = LazyLock::new(|| {
    let re = |p: &str| Regex::new(&format!("(?i){p}")).expect("valid pattern");
    [
        (
            re(r"API Error: 401|authentication_error|OAuth token (has )?expired|Invalid API key"),
            "sign-in failed · run /login",
        ),
        (re(r"Please run /login"), "signed out · run /login"),
        (
            re(r"API Error: (429|5\d\d)|overloaded_error|rate_limit_error"),
            "API error · retrying or rate-limited",
        ),
    ]
});

/// The error the given screen lines show, if any: the first matching pattern wins.
pub fn detect_screen_error(lines: &[String]) -> Option<String> {
    let text = lines[lines.len().saturating_sub(SCREEN_ERROR_LINES)..].join("\n");
    SCREEN_ERRORS
        .iter()
        .find(|(pattern, _)| pattern.is_match(&text))
        .map(|(_, label)| label.to_string())
}

/// The visible screen as plain text lines.
pub fn screen_lines(screen: &vt100::Screen) -> Vec<String> {
    let (_, cols) = screen.size();
    screen.rows(0, cols).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(l: &[&str]) -> Vec<String> {
        l.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn recognizes_sign_in_failures_and_api_errors() {
        assert_eq!(
            detect_screen_error(&lines(&[
                "❯ hi",
                r#"  ⎿  API Error: 401 {"type":"error","error":{"type":"authentication_error"}}"#
            ]))
            .as_deref(),
            Some("sign-in failed · run /login")
        );
        assert_eq!(
            detect_screen_error(&lines(&["Invalid API key · Please run /login"])).as_deref(),
            Some("sign-in failed · run /login")
        );
        assert_eq!(
            detect_screen_error(&lines(&["Not logged in · Please run /login"])).as_deref(),
            Some("signed out · run /login")
        );
        assert_eq!(
            detect_screen_error(&lines(&[r#"API Error: 529 {"type":"overloaded_error"}"#])).as_deref(),
            Some("API error · retrying or rate-limited")
        );
    }

    #[test]
    fn ignores_normal_output_and_errors_that_scrolled_out_of_the_last_lines() {
        assert_eq!(
            detect_screen_error(&lines(&["● Done. The tests pass.", "❯"])),
            None
        );
        let mut scrolled = lines(&["API Error: 401"]);
        scrolled.extend(std::iter::repeat_n(
            "later output".to_string(),
            SCREEN_ERROR_LINES,
        ));
        assert_eq!(detect_screen_error(&scrolled), None);
    }

    #[test]
    fn screen_lines_reads_the_visible_rows() {
        let mut parser = vt100::Parser::new(3, 10, 0);
        parser.process(b"one\r\ntwo");
        assert_eq!(screen_lines(parser.screen()), ["one", "two", ""]);
    }
}
