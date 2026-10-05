//! The preview panel, port of `renderPreviewPanel` / `renderGroupPreviewPanel` in `view.ts`: a
//! title and meta line, then the live screen, a stopped session's summary, or a group's sessions.

use std::borrow::Cow;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use sdeck_core::status::claude_transcript_tailer::{TailedTurn, TurnRole};

use super::list_panel::glyph;
use super::{panel_header, GroupCounts, SessionView};
use crate::ansi::{fit, text_width, wrap};
use crate::layout::PANEL_HEADER_ROWS;
use crate::sessions::{LastResponse, SessionStatus};
use crate::term_widget::TermWidget;
use crate::theme::{Role, Theme};

/// What the preview shows when a folder or project row is selected.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupPreview {
    pub is_folder: bool,
    pub name: String,
    /// Project path, or "3 projects" for a folder.
    pub detail: String,
    pub counts: GroupCounts,
    pub sessions: Vec<SessionView>,
}

/// What the preview shows for a session.
pub struct PreviewContent<'a> {
    pub view: SessionView,
    /// The live agent's screen when it runs here, already scrolled back by `scroll_offset`.
    pub screen: Option<Cow<'a, vt100::Screen>>,
    /// Keystrokes go into this session (`i`), as opposed to it only being previewed.
    pub interacting: bool,
    pub exit_code: Option<u32>,
    /// `None`: nothing to load (no transcript).
    pub last_response: Option<&'a LastResponse>,
    /// Tailed turns of a session open elsewhere, newest last; empty while backfilling.
    pub live_turns: Option<&'a [TailedTurn]>,
    /// Lines scrolled back from the live bottom.
    pub scroll_offset: usize,
    /// Cells of `screen` highlighted by click-drag, first and last in reading order.
    pub selection: Option<((u16, u16), (u16, u16))>,
}

/// A muted grey for highlighted text: the theme's text color mixed a quarter of the way into its
/// background, so it reads as a soft block in light and dark themes alike.
fn selection_color(t: Theme) -> Color {
    match (t.color(Role::Bg), t.color(Role::Text)) {
        (Color::Rgb(br, bg, bb), Color::Rgb(tr, tg, tb)) => {
            let mix = |b: u8, f: u8| (u16::from(b) * 3 / 4 + u16::from(f) / 4) as u8;
            Color::Rgb(mix(br, tr), mix(bg, tg), mix(bb, tb))
        }
        _ => Color::DarkGray,
    }
}

fn bold(style: Style) -> Style {
    style.add_modifier(Modifier::BOLD)
}

fn status_text(status: SessionStatus) -> &'static str {
    match status {
        SessionStatus::Running => "running",
        SessionStatus::Waiting => "waiting for you",
        SessionStatus::Done => "finished · not seen yet",
        SessionStatus::Idle => "idle",
        SessionStatus::Starting => "starting",
        SessionStatus::Error => "error",
        SessionStatus::Exited => "exited",
        SessionStatus::Stopped => "not running",
    }
}

/// `─ meta ───…`: the meta text between border dashes, filling the width.
fn meta_line(t: Theme, width: usize, meta: &str, role: Role) -> Line<'static> {
    let fitted = fit(meta, width.saturating_sub(2)).trim_end().to_string();
    let rest = width.saturating_sub(1 + text_width(&fitted));
    Line::from(vec![
        Span::styled("─", t.fg(Role::Border)),
        Span::styled(fitted, t.fg(role)),
        Span::styled("─".repeat(rest), t.fg(Role::Border)),
    ])
}

/// A bold accent key name for hint lines.
fn key(t: Theme, k: &str) -> Span<'static> {
    Span::styled(k.to_string(), bold(t.fg(Role::Accent)))
}

fn dim(t: Theme, text: &str) -> Span<'static> {
    Span::styled(text.to_string(), t.fg(Role::TextDim))
}

fn place(buf: &mut Buffer, rect: Rect, lines: Vec<Line<'static>>) {
    for (y, line) in (rect.y..rect.y + rect.height).zip(lines) {
        line.render(Rect::new(rect.x, y, rect.width, 1).intersection(buf.area), buf);
    }
}

pub fn group_preview_lines(
    t: Theme,
    width: usize,
    height: usize,
    group: &GroupPreview,
) -> Vec<Line<'static>> {
    let (kind, name_role) = if group.is_folder {
        ("Folder", Role::Purple)
    } else {
        ("Project", Role::Cyan)
    };
    let title = Line::styled(
        fit(&format!("{kind} · {}", group.name), width),
        bold(t.fg(name_role)),
    );
    let GroupCounts {
        count,
        running,
        waiting,
    } = group.counts;
    let mut meta = format!(" {count} session{}", if count == 1 { "" } else { "s" });
    if running > 0 {
        meta.push_str(&format!(" · {running} running"));
    }
    if waiting > 0 {
        meta.push_str(&format!(" · {waiting} waiting"));
    }
    meta.push_str(&format!(" · {} ", group.detail));
    let hints: &[(&str, &str)] = if group.is_folder {
        &[
            ("Enter", " collapse/expand · "),
            ("K J", " reorder · "),
            ("e", " rename · "),
            ("d", " delete"),
        ]
    } else {
        &[
            ("Enter", " collapse/expand · "),
            ("M", " move to folder · "),
            ("K J", " reorder · "),
            ("n", " new session · "),
            ("d", " remove"),
        ]
    };
    let mut hint_spans = vec![Span::raw("  ")];
    for (k, label) in hints {
        hint_spans.push(key(t, k));
        hint_spans.push(dim(t, label));
    }
    let mut lines = vec![
        title,
        meta_line(t, width, &meta, Role::TextDim),
        Line::default(),
        Line::from(hint_spans),
        Line::default(),
    ];
    for s in &group.sessions {
        let right = format!(" {} ", s.time_label);
        let title = fit(&s.title, width.saturating_sub(4 + text_width(&right)).max(1));
        lines.push(Line::from(vec![
            Span::raw("  "),
            glyph(t, s.status),
            Span::raw(" "),
            Span::styled(title, t.fg(Role::Text)),
            dim(t, &right),
        ]));
    }
    lines.truncate(height);
    lines
}

/// Tailed turns as a read-only mini-transcript, keeping the last `height` lines.
fn live_turn_lines(t: Theme, turns: &[TailedTurn], width: usize, height: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for turn in turns {
        let label = match turn.role {
            TurnRole::User => Span::styled("You", bold(t.fg(Role::Cyan))),
            TurnRole::Assistant => Span::styled("Claude", bold(t.fg(Role::Orange))),
        };
        lines.push(Line::from(vec![Span::raw("  "), label]));
        for line in wrap(&turn.text, width.saturating_sub(4).max(10)) {
            lines.push(Line::styled(format!("  {line}"), t.fg(Role::Text)));
        }
        lines.push(Line::default());
    }
    let skip = lines.len().saturating_sub(height);
    lines.split_off(skip)
}

/// The preview panel for a session (or the empty panel for none), drawn into `rect`.
pub fn render_preview_panel(t: Theme, rect: Rect, buf: &mut Buffer, content: Option<&PreviewContent<'_>>) {
    let width = usize::from(rect.width);
    let Some(content) = content else {
        place(buf, rect, panel_header(t, width, "PREVIEW", "").to_vec());
        return;
    };
    let v = &content.view;
    let base = v.detail.as_deref().unwrap_or(status_text(v.status));
    let status = if v.elsewhere {
        format!("{base} in another terminal")
    } else {
        base.to_string()
    };
    let title = Line::from(vec![
        glyph(t, v.status),
        Span::raw(" "),
        Span::styled(
            fit(&v.title, width.saturating_sub(2).max(1)),
            bold(t.fg(Role::Accent)),
        ),
    ]);
    let git = v.git.as_ref().map_or(String::new(), |g| {
        let mut text = format!(" · ⎇{}", g.branch.as_deref().unwrap_or("(detached)"));
        if g.ahead > 0 || g.behind > 0 {
            text.push_str(&format!(" ⇡{} ⇣{}", g.ahead, g.behind));
        }
        if g.dirty > 0 {
            text.push_str(&format!(" ✱{}", g.dirty));
        }
        text
    });
    let scrolled = content.scroll_offset > 0;
    let meta = format!(
        " {status} · {} · {} · {}{git}{} ",
        v.agent,
        v.time_label,
        v.cwd,
        if scrolled {
            " · ↑ scrolled · End to jump to latest"
        } else {
            ""
        }
    );
    let meta_role = if scrolled { Role::Yellow } else { Role::TextDim };
    place(buf, rect, vec![title, meta_line(t, width, &meta, meta_role)]);

    let body = Rect {
        y: rect.y + PANEL_HEADER_ROWS,
        height: rect.height.saturating_sub(PANEL_HEADER_ROWS),
        ..rect
    };
    let body_height = usize::from(body.height);
    if let Some(screen) = &content.screen {
        TermWidget {
            screen,
            show_cursor: content.interacting,
            selection: content.selection,
            selection_bg: selection_color(t),
            default_fg: t.color(Role::Text),
            default_bg: t.color(Role::Bg),
        }
        .render(body, buf);
        return;
    }

    if v.elsewhere {
        let header = Line::from(vec![
            Span::raw("  "),
            Span::styled("Open in another terminal.", t.fg(Role::Yellow)),
            dim(t, " Live preview, read-only · close it there to open it here."),
        ]);
        let turns_height = body_height.saturating_sub(1);
        let turns = match content.live_turns {
            Some(turns) if !turns.is_empty() => live_turn_lines(t, turns, width, turns_height),
            _ => vec![Line::from(vec![Span::raw("  "), dim(t, "loading…")])],
        };
        // Pinned header, then the turns anchored to the bottom like a live terminal.
        let mut lines = vec![header];
        lines.extend(std::iter::repeat_n(
            Line::default(),
            turns_height.saturating_sub(turns.len()),
        ));
        lines.extend(turns);
        place(buf, body, lines);
        return;
    }

    let action = if v.status == SessionStatus::Exited {
        let code = content.exit_code.map_or("?".to_string(), |c| c.to_string());
        vec![
            Span::styled(format!("Exited (code {code})."), t.fg(Role::Red)),
            Span::raw(" "),
            key(t, "Enter"),
            dim(t, " restart · "),
            key(t, "x"),
            dim(t, " clear"),
        ]
    } else {
        vec![
            key(t, "Enter"),
            dim(t, " start + attach · "),
            key(t, "s"),
            dim(t, " start in background"),
        ]
    };
    let mut lines = vec![
        Line::default(),
        Line::from([vec![Span::raw("  ")], action].concat()),
        Line::from(vec![
            Span::raw("  "),
            dim(t, v.id.as_deref().unwrap_or("(new session)")),
        ]),
        Line::default(),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("LAST RESPONSE", bold(t.fg(Role::Cyan))),
        ]),
    ];
    match content.last_response {
        Some(LastResponse::Loading) => lines.push(Line::from(vec![Span::raw("  "), dim(t, "loading…")])),
        Some(LastResponse::Ready(text)) if !text.is_empty() => {
            for line in wrap(text, width.saturating_sub(4).max(10)) {
                lines.push(Line::styled(fit(&format!("  {line}"), width), t.fg(Role::Text)));
            }
        }
        _ => {}
    }
    lines.truncate(body_height);
    place(buf, body, lines);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeName;
    use sdeck_core::discovery::git_status::GitStatus;

    const T: Theme = Theme::new(ThemeName::Dark);

    fn view(status: SessionStatus) -> SessionView {
        SessionView {
            title: "Fix the login bug".into(),
            status,
            elsewhere: false,
            agent: "claude".into(),
            show_agent: false,
            time_label: "5m ago".into(),
            cwd: "~/repos/api".into(),
            id: Some("abc-123".into()),
            detail: None,
            git: None,
            account_tag: None,
            other_account: false,
        }
    }

    fn content(view: SessionView) -> PreviewContent<'static> {
        PreviewContent {
            view,
            screen: None,
            interacting: false,
            exit_code: None,
            last_response: None,
            live_turns: None,
            scroll_offset: 0,
            selection: None,
        }
    }

    fn draw(width: u16, height: u16, content: Option<&PreviewContent<'_>>) -> Vec<String> {
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        render_preview_panel(T, buf.area, &mut buf, content);
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn a_live_screen_fills_the_body_under_the_title_and_meta() {
        let mut parser = vt100::Parser::new(4, 30, 0);
        parser.process(b"CLAUDE_CONFIG_DIR=unset\r\n> hello");
        let mut c = content(view(SessionStatus::Running));
        c.view.git = Some(GitStatus {
            branch: Some("main".into()),
            ahead: 2,
            behind: 0,
            dirty: 3,
        });
        c.screen = Some(Cow::Borrowed(parser.screen()));
        let rows = draw(60, 6, Some(&c));
        assert_eq!(
            rows,
            [
                "● Fix the login bug",
                "─ running · claude · 5m ago · ~/repos/api · ⎇main ⇡2 ⇣0 ✱3──",
                "CLAUDE_CONFIG_DIR=unset",
                "> hello",
                "",
                "",
            ]
        );
    }

    #[test]
    fn a_scrolled_preview_says_so_in_the_meta_line() {
        let mut c = content(view(SessionStatus::Running));
        c.scroll_offset = 3;
        let rows = draw(90, 4, Some(&c));
        assert!(
            rows[1].contains("↑ scrolled · End to jump to latest"),
            "{}",
            rows[1]
        );
    }

    #[test]
    fn a_stopped_session_shows_its_actions_id_and_last_response() {
        let response = LastResponse::Ready("All tests pass now.".into());
        let mut c = content(view(SessionStatus::Stopped));
        c.last_response = Some(&response);
        let rows = draw(50, 10, Some(&c));
        assert_eq!(rows[1], "─ not running · claude · 5m ago · ~/repos/api─────");
        assert_eq!(rows[3], "  Enter start + attach · s start in background");
        assert_eq!(rows[4], "  abc-123");
        assert_eq!(rows[6], "  LAST RESPONSE");
        assert_eq!(rows[7], "  All tests pass now.");

        let loading = LastResponse::Loading;
        c.last_response = Some(&loading);
        assert_eq!(draw(50, 10, Some(&c))[7], "  loading…");
    }

    #[test]
    fn an_exited_session_offers_restart_with_its_exit_code() {
        let mut c = content(view(SessionStatus::Exited));
        c.exit_code = Some(3);
        let rows = draw(60, 6, Some(&c));
        assert_eq!(rows[3], "  Exited (code 3). Enter restart · x clear");
    }

    #[test]
    fn an_elsewhere_session_shows_its_tailed_turns_bottom_anchored() {
        let turns = vec![
            TailedTurn {
                role: TurnRole::User,
                text: "hi".into(),
            },
            TailedTurn {
                role: TurnRole::Assistant,
                text: "hello".into(),
            },
        ];
        let mut c = content(view(SessionStatus::Running));
        c.view.elsewhere = true;
        c.live_turns = Some(&turns);
        let rows = draw(80, 10, Some(&c));
        assert!(
            rows[1].starts_with("─ running in another terminal"),
            "{}",
            rows[1]
        );
        assert!(rows[2].starts_with("  Open in another terminal."), "{}", rows[2]);
        assert_eq!(&rows[4..], ["  You", "  hi", "", "  Claude", "  hello", ""]);
    }

    #[test]
    fn no_session_leaves_only_the_panel_header() {
        assert_eq!(draw(10, 3, None), ["PREVIEW", "──────────", ""]);
    }

    #[test]
    fn a_group_lists_its_sessions_newest_first_with_hints() {
        let group = GroupPreview {
            is_folder: false,
            name: "api".into(),
            detail: "~/repos/api".into(),
            counts: GroupCounts {
                count: 2,
                running: 1,
                waiting: 0,
            },
            sessions: vec![view(SessionStatus::Running), view(SessionStatus::Stopped)],
        };
        let text: Vec<String> = group_preview_lines(T, 70, 8, &group)
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect();
        assert_eq!(text[0], "Project · api");
        assert_eq!(
            text[1],
            "─ 2 sessions · 1 running · ~/repos/api────────────────────────────────"
        );
        assert!(
            text[3].starts_with("  Enter collapse/expand · M move to folder"),
            "{}",
            text[3]
        );
        assert!(text[5].starts_with("  ● Fix the login bug"), "{}", text[5]);
        assert!(text[5].ends_with("5m ago"), "{}", text[5]);
        assert_eq!(text.len(), 7);
    }
}
