//! Header, filter pills, and the bottom bar variants (help, message, prompt, confirm) — port of the
//! matching `view.ts` renderers. Each returns one line exactly `cols` wide.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::ansi::{fit, fit_tail, text_width};
use crate::filters::{StatusCategory, StatusCounts, TimeFilter, STATUS_CATEGORIES};
use crate::theme::{Role, Theme};

fn bold(style: Style) -> Style {
    style.add_modifier(Modifier::BOLD)
}

fn blank(width: usize) -> Span<'static> {
    Span::raw(" ".repeat(width))
}

fn category_glyph(category: StatusCategory) -> (char, Role) {
    match category {
        StatusCategory::Running => ('●', Role::Accent),
        StatusCategory::Waiting => ('◐', Role::Yellow),
        StatusCategory::Idle => ('○', Role::TextDim),
        StatusCategory::Error => ('✕', Role::Red),
        StatusCategory::Stopped => ('■', Role::TextDim),
    }
}

/// Top row: the name on the left, `N live` on the right.
pub fn header(t: Theme, cols: usize, live_count: usize) -> Line<'static> {
    let title = bold(t.fg(Role::Accent));
    let right = format!("{live_count} live ");
    let left_width = 1 + text_width("Session Deck");
    let mut spans = vec![Span::raw(" ")];
    match cols
        .checked_sub(left_width + text_width(&right))
        .filter(|&gap| gap >= 1)
    {
        Some(gap) => {
            spans.push(Span::styled("Session Deck", title));
            spans.push(blank(gap));
            spans.push(Span::styled(right, t.fg(Role::TextDim)));
        }
        None => spans.push(Span::styled(fit("Session Deck", cols.saturating_sub(2)), title)),
    }
    Line::from(spans).style(t.bg(Role::Surface))
}

/// Second row: `All N`, one pill per status category (plus the done pill after waiting), the time
/// filter, and the tag filter if any.
#[allow(clippy::too_many_arguments)] // mirrors TS `renderPills`
pub fn pills(
    t: Theme,
    cols: usize,
    total: usize,
    counts: &StatusCounts,
    done_count: usize,
    status_filter: &[StatusCategory],
    time_filter: TimeFilter,
    tag_filter: Option<&str>,
) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    let mut plain_width = 1;
    let mut pill = |spans: &mut Vec<Span<'static>>, label: String, active: bool, label_style: Style| {
        plain_width += text_width(&label) + 2;
        if active {
            spans.push(Span::styled(
                format!(" {label} "),
                bold(t.bg(Role::Accent).fg(t.color(Role::Bg))),
            ));
        } else {
            spans.extend([Span::raw(" "), Span::styled(label, label_style), Span::raw(" ")]);
        }
    };
    pill(
        &mut spans,
        format!("All {total}"),
        status_filter.is_empty(),
        t.fg(Role::Text),
    );
    for category in STATUS_CATEGORIES {
        let (glyph, role) = category_glyph(category);
        pill(
            &mut spans,
            format!("{glyph} {}", counts.get(category)),
            status_filter.contains(&category),
            t.fg(role),
        );
        // "done" (finished, not yet seen) folds into the "waiting" filter category, so its pill rides along right after it.
        if category == StatusCategory::Waiting {
            let active = status_filter.contains(&StatusCategory::Waiting);
            pill(&mut spans, format!("✓ {done_count}"), active, t.fg(Role::Green));
        }
    }
    spans.push(Span::styled("│", t.fg(Role::Border)));
    pill(
        &mut spans,
        time_filter.label().to_string(),
        time_filter != TimeFilter::All,
        t.fg(Role::Purple),
    );
    let mut extra = 1;
    if let Some(tag) = tag_filter {
        spans.push(Span::styled("│", t.fg(Role::Border)));
        pill(&mut spans, format!("# {tag}"), true, t.fg(Role::Cyan));
        extra += 1;
    }
    let plain_width = plain_width + extra;
    spans.push(blank(cols.saturating_sub(plain_width)));
    Line::from(spans)
}

/// Widest first; the first variant that fits is shown.
const HELP_VARIANTS: &[&[(&str, &str)]] = &[
    &[
        ("↑↓", "select"),
        ("⏎", "attach"),
        ("Space", "mark"),
        ("n", "new"),
        ("o", "prompt"),
        ("e", "rename"),
        ("x", "stop"),
        ("A", "archive"),
        ("d", "delete"),
        ("M", "move"),
        ("?", "help"),
        ("q", "quit"),
    ],
    &[
        ("↑↓", "select"),
        ("⏎", "attach"),
        ("n", "new"),
        ("e", "rename"),
        ("x", "stop"),
        ("?", "help"),
        ("q", "quit"),
    ],
    &[("⏎", "attach"), ("?", "help"), ("q", "quit")],
    &[("?", "help")],
];

/// Bottom row when nothing else claims it: key hints, shortened to fit narrow widths.
pub fn help_bar(t: Theme, cols: usize) -> Line<'static> {
    for variant in HELP_VARIANTS {
        let plain = format!(
            " {}",
            variant
                .iter()
                .map(|(k, l)| format!("{k} {l}"))
                .collect::<Vec<_>>()
                .join(" · ")
        );
        let width = text_width(&plain);
        if width > cols {
            continue;
        }
        let mut spans = vec![Span::raw(" ")];
        for (i, (k, l)) in variant.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" · ", t.fg(Role::Border)));
            }
            spans.extend([
                Span::styled(*k, bold(t.fg(Role::Accent))),
                Span::raw(" "),
                Span::styled(*l, t.fg(Role::TextDim)),
            ]);
        }
        spans.push(blank(cols - width));
        return Line::from(spans);
    }
    Line::from(blank(cols))
}

/// A transient message (see `App::flash`) in place of the help bar.
pub fn message_bar(t: Theme, cols: usize, message: &str) -> Line<'static> {
    Line::from(Span::styled(
        fit(&format!(" {message}"), cols),
        bold(t.fg(Role::Yellow)),
    ))
}

/// A yes/no question in place of the help bar.
pub fn confirm_bar(t: Theme, cols: usize, question: &str) -> Line<'static> {
    message_bar(t, cols, &format!("{question} (y/N)"))
}

/// Footer text input: `label: value` with a reverse-video cursor, keeping the value's tail visible.
pub fn prompt_bar(t: Theme, cols: usize, label: &str, value: &str) -> Line<'static> {
    let prefix = format!(" {label}: ");
    let width = cols.saturating_sub(text_width(&prefix)).max(1);
    let cursor_width = 1;
    // Keep the tail visible rather than truncating it away when the value overflows.
    let visible = fit_tail(value, width - cursor_width);
    let visible_width = text_width(&visible).min(width - cursor_width);
    let pad = width - visible_width - cursor_width;
    // A reverse-video space instead of a block glyph: relies only on color swap, not on the terminal
    // having (and correctly rendering) a full-block character.
    Line::from(vec![
        Span::styled(prefix, bold(t.fg(Role::Accent))),
        Span::styled(visible, t.fg(Role::Text)),
        Span::styled(" ", Style::new().add_modifier(Modifier::REVERSED)),
        Span::styled(" ".repeat(pad), t.fg(Role::Text)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeName;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    const DARK: Theme = Theme {
        name: ThemeName::Dark,
    };

    /// Draws `line` on a one-row TestBackend `cols` wide and returns the row's text.
    fn snapshot(line: Line<'static>, cols: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(cols, 1)).unwrap();
        term.draw(|f| f.render_widget(line, f.area())).unwrap();
        let buf = term.backend().buffer();
        (0..cols).map(|x| buf[(x, 0)].symbol().to_string()).collect()
    }

    fn counts() -> StatusCounts {
        let mut c = StatusCounts::default();
        c.set(StatusCategory::Running, 2);
        c.set(StatusCategory::Idle, 5);
        c
    }

    #[test]
    fn header_at_120_60_and_30_columns() {
        let line = header(DARK, 120, 3);
        assert_eq!(line.width(), 120);
        let row = snapshot(line, 120);
        assert!(row.starts_with(" Session Deck "), "{row}");
        assert!(row.ends_with("3 live "), "{row}");

        let narrow = header(DARK, 30, 3);
        assert_eq!(narrow.width(), 30);
        assert_eq!(
            snapshot(narrow, 30),
            format!(" Session Deck{}3 live ", " ".repeat(10))
        );
        let tiny = header(DARK, 10, 3);
        assert_eq!(snapshot(tiny, 10), " Session… ");
    }

    #[test]
    fn pills_at_120_and_60_columns() {
        let row = snapshot(pills(DARK, 120, 7, &counts(), 1, &[], TimeFilter::All, None), 120);
        let left = "  All 7  ● 2  ◐ 0  ✓ 1  ○ 5  ✕ 0  ■ 0 │ all time ";
        assert_eq!(row, format!("{left}{}", " ".repeat(120 - text_width(left))));
        assert!(!row.contains("filter"));
        let line = pills(
            DARK,
            60,
            7,
            &counts(),
            1,
            &[StatusCategory::Running],
            TimeFilter::Today,
            Some("work"),
        );
        assert_eq!(line.width(), 60);
        assert_eq!(
            snapshot(line, 60).trim_end(),
            "  All 7  ● 2  ◐ 0  ✓ 1  ○ 5  ✕ 0  ■ 0 │ today │ # work"
        );
    }

    #[test]
    fn pills_highlight_the_active_filters() {
        let line = pills(
            DARK,
            120,
            7,
            &counts(),
            1,
            &[StatusCategory::Waiting],
            TimeFilter::All,
            None,
        );
        let active: Vec<String> = line
            .spans
            .iter()
            .filter(|s| s.style.bg == Some(DARK.color(Role::Accent)))
            .map(|s| s.content.to_string())
            .collect();
        assert_eq!(active, [" ◐ 0 ", " ✓ 1 "]);
    }

    #[test]
    fn help_bar_shortens_on_narrow_widths() {
        let wide = snapshot(help_bar(DARK, 130), 130);
        assert!(
            wide.starts_with(" ↑↓ select · ⏎ attach · Space mark · n new"),
            "{wide}"
        );
        let at_120 = snapshot(help_bar(DARK, 120), 120);
        assert_eq!(
            at_120.trim_end(),
            " ↑↓ select · ⏎ attach · n new · e rename · x stop · ? help · q quit"
        );
        assert_eq!(
            snapshot(help_bar(DARK, 60), 60).trim_end(),
            " ⏎ attach · ? help · q quit"
        );
        assert_eq!(snapshot(help_bar(DARK, 8), 8), " ? help ");
        assert_eq!(snapshot(help_bar(DARK, 3), 3), "   ");
    }

    #[test]
    fn message_and_confirm_bars_fit_the_width() {
        assert_eq!(
            snapshot(message_bar(DARK, 120, "Theme: light"), 120).trim_end(),
            " Theme: light"
        );
        assert_eq!(snapshot(message_bar(DARK, 10, "Theme: light"), 10), " Theme: l…");
        assert_eq!(
            snapshot(confirm_bar(DARK, 60, "Quit?"), 60).trim_end(),
            " Quit? (y/N)"
        );
    }

    #[test]
    fn prompt_bar_keeps_the_tail_and_draws_a_cursor() {
        let line = prompt_bar(DARK, 120, "Name", "abc");
        assert_eq!(line.width(), 120);
        assert_eq!(snapshot(line, 120).trim_end(), " Name: abc");
        let row = snapshot(prompt_bar(DARK, 20, "Name", "a very long session name"), 20);
        assert_eq!(row, " Name: …ession name ");
    }
}
