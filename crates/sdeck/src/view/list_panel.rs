//! The sessions panel, port of `renderListPanel` and its row renderers in `view.ts`. Every line is
//! exactly `width` columns wide.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use sdeck_core::discovery::git_status::GitStatus;
use sdeck_core::status::usage_display::{
    render_usage_bar, usage_severity, DailySpend, UsageMetric, UsageSeverity, USAGE_BAR_WIDTH,
};
use sdeck_core::store::tree_prefs::SessionPin;

use super::{panel_header, GroupCounts, SessionView};
use crate::ansi::{fit, text_width};
use crate::sessions::SessionStatus;
use crate::theme::{Role, Theme};

/// What the panel draws: `TreeRow`s with their sessions resolved into [`SessionView`]s.
#[derive(Debug, Clone, PartialEq)]
pub enum ListRow {
    Folder {
        name: String,
        counts: GroupCounts,
        collapsed: bool,
        hotkey: Option<u8>,
    },
    Project {
        label: String,
        counts: GroupCounts,
        collapsed: bool,
        depth: usize,
        hotkey: Option<u8>,
        git: Option<GitStatus>,
    },
    Session {
        view: Box<SessionView>,
        is_last: bool,
        depth: usize,
        pin: Option<SessionPin>,
        checked: bool,
    },
    Divider {
        label: String,
    },
    Tag {
        name: String,
        count: usize,
        active: bool,
    },
    Usage {
        metric: UsageMetric,
        percent: Option<f64>,
        reset_label: Option<String>,
    },
    UsageBudget {
        days: Vec<DailySpend>,
    },
}

/// `(glyph, role, bold)` of a session's status dot.
fn status_glyph(status: SessionStatus) -> (char, Role, bool) {
    match status {
        SessionStatus::Running => ('●', Role::Red, true),
        SessionStatus::Waiting => ('◐', Role::Yellow, true),
        // Finished, not seen yet: waiting for a look. Same green as the extension's "Done" decoration.
        SessionStatus::Done => ('●', Role::Green, true),
        SessionStatus::Idle => ('○', Role::TextDim, false),
        SessionStatus::Starting => ('⟳', Role::Yellow, false),
        SessionStatus::Error | SessionStatus::Exited => ('✕', Role::Red, true),
        SessionStatus::Stopped => ('■', Role::TextDim, false),
    }
}

/// A session's styled status dot.
pub(crate) fn glyph(t: Theme, status: SessionStatus) -> Span<'static> {
    let (glyph, role, is_bold) = status_glyph(status);
    let style = t.fg(role);
    Span::styled(glyph.to_string(), if is_bold { bold(style) } else { style })
}

fn bold(style: Style) -> Style {
    style.add_modifier(Modifier::BOLD)
}

fn blank(width: usize) -> Span<'static> {
    Span::raw(" ".repeat(width))
}

fn selected_style(t: Theme) -> Style {
    t.bg(Role::Accent).fg(t.color(Role::Bg))
}

/// ⇡/⇣ ahead/behind upstream, ✱ dirty: one badge for the whole project.
pub fn git_glyphs(g: &GitStatus) -> String {
    format!(
        "{}{}{}",
        if g.ahead > 0 { "⇡" } else { "" },
        if g.behind > 0 { "⇣" } else { "" },
        if g.dirty > 0 { "✱" } else { "" }
    )
}

struct GroupRow<'a> {
    is_folder: bool,
    name: &'a str,
    counts: &'a GroupCounts,
    collapsed: bool,
    depth: usize,
    hotkey: Option<u8>,
    git: Option<&'a GitStatus>,
}

/// Folder and project rows: `1▾ name (n) ●r ◐w ⇡⇣✱`. The leading column shows the 1–9 jump key on
/// top-level rows.
fn group_row(t: Theme, width: usize, g: GroupRow, selected: bool) -> Line<'static> {
    let hotkey = g.hotkey.map_or(" ".to_string(), |h| h.to_string());
    let arrow = if g.collapsed { "▸" } else { "▾" };
    let mid = format!("{}{arrow} ", "  ".repeat(g.depth));
    let lead_width = text_width(&hotkey) + text_width(&mid);
    let git_text = g.git.map(git_glyphs).unwrap_or_default();
    let mut tail = String::new();
    if g.counts.running > 0 {
        tail.push_str(&format!(" ●{}", g.counts.running));
    }
    if g.counts.waiting > 0 {
        tail.push_str(&format!(" ◐{}", g.counts.waiting));
    }
    if !git_text.is_empty() {
        tail.push_str(&format!(" {git_text}"));
    }
    let suffix = format!(" ({}){tail}", g.counts.count);
    let label = fit(
        g.name,
        width.saturating_sub(lead_width + text_width(&suffix)).max(1),
    );
    let label = label.trim_end().to_string();
    let pad = " ".repeat(width.saturating_sub(lead_width + text_width(&label) + text_width(&suffix)));
    if selected {
        let sel = selected_style(t);
        return Line::from(vec![
            Span::styled(format!("{hotkey}{mid}"), sel),
            Span::styled(label, bold(sel)),
            Span::styled(format!("{suffix}{pad}"), sel),
        ]);
    }
    let name_role = if g.is_folder { Role::Purple } else { Role::Cyan };
    let mut spans = vec![
        Span::styled(hotkey, t.fg(Role::TextDim)),
        Span::styled(mid, t.fg(Role::Text)),
        Span::styled(label, bold(t.fg(name_role))),
        Span::styled(format!(" ({})", g.counts.count), t.fg(Role::Text)),
    ];
    if g.counts.running > 0 {
        spans.push(Span::styled(format!(" ●{}", g.counts.running), t.fg(Role::Red)));
    }
    if g.counts.waiting > 0 {
        spans.push(Span::styled(
            format!(" ◐{}", g.counts.waiting),
            t.fg(Role::Yellow),
        ));
    }
    if !git_text.is_empty() {
        spans.push(Span::styled(format!(" {git_text}"), t.fg(Role::Yellow)));
    }
    spans.push(Span::raw(pad));
    Line::from(spans)
}

struct SessionRow<'a> {
    view: &'a SessionView,
    is_last: bool,
    depth: usize,
    pin: Option<SessionPin>,
    checked: bool,
    show_checkbox: bool,
}

fn session_row(t: Theme, width: usize, r: SessionRow, selected: bool) -> Line<'static> {
    let v = r.view;
    let indent = "  ".repeat(r.depth);
    let connector = if r.is_last { "└─" } else { "├─" };
    let checkbox_plain = match (r.show_checkbox, r.checked) {
        (true, true) => "✔ ",
        (true, false) => "· ",
        _ => "",
    };
    // Narrow glyphs only: emoji are one column wide in some terminals and two in others.
    let pin_mark = match r.pin {
        Some(SessionPin::Top) => "↑ ",
        Some(SessionPin::Bottom) => "↓ ",
        None => "",
    };
    let elsewhere_mark = if v.elsewhere { "↗ " } else { "" };
    let (glyph, glyph_role, glyph_bold) = status_glyph(v.status);
    let left_plain = format!("{checkbox_plain}{indent}{connector} {glyph} {pin_mark}{elsewhere_mark}");
    let agent_text = format!(" {}", v.agent);
    // The other account's tag sits right after the title, before the agent name.
    let tag_text = v
        .account_tag
        .as_ref()
        .map(|tag| format!(" {tag}"))
        .unwrap_or_default();
    let avail = width.saturating_sub(text_width(&left_plain)).max(1);
    let max_title = avail
        .saturating_sub(text_width(&agent_text) + text_width(&tag_text))
        .max(1);
    // Truncated to fit, but not padded: the agent name sits right after the title.
    let title = fit(&v.title, text_width(&v.title).min(max_title));
    let used = text_width(&left_plain) + text_width(&title) + text_width(&tag_text) + text_width(&agent_text);
    let pad = " ".repeat(width.saturating_sub(used));
    let active = matches!(v.status, SessionStatus::Running | SessionStatus::Waiting);

    if selected {
        let sel = selected_style(t);
        return Line::from(vec![
            Span::styled(left_plain, sel),
            Span::styled(title, bold(sel)),
            Span::styled(format!("{tag_text}{agent_text}{pad}"), sel),
        ]);
    }
    let mut spans = Vec::new();
    if r.show_checkbox {
        spans.push(if r.checked {
            Span::styled("✔ ", bold(t.fg(Role::Accent)))
        } else {
            Span::styled("· ", t.fg(Role::TextDim))
        });
    }
    spans.push(Span::raw(indent));
    spans.push(Span::styled(connector, t.fg(Role::Border)));
    spans.push(Span::raw(" "));
    let glyph_style = t.fg(glyph_role);
    spans.push(Span::styled(
        glyph.to_string(),
        if glyph_bold {
            bold(glyph_style)
        } else {
            glyph_style
        },
    ));
    spans.push(Span::raw(" "));
    if !pin_mark.is_empty() {
        spans.push(Span::styled(pin_mark, t.fg(Role::Accent)));
    }
    if v.elsewhere {
        spans.push(Span::styled("↗ ", t.fg(Role::Purple)));
    }
    let mut title_style = t.fg(Role::Text);
    if active {
        title_style = bold(title_style);
    }
    if v.status == SessionStatus::Exited {
        title_style = title_style.add_modifier(Modifier::UNDERLINED);
    }
    spans.push(Span::styled(title, title_style));
    spans.push(Span::styled(tag_text, t.fg(Role::TextDim)));
    spans.push(Span::styled(agent_text, t.fg(Role::TextDim)));
    spans.push(Span::raw(pad));
    Line::from(spans)
}

fn divider_row(t: Theme, width: usize, label: &str) -> Line<'static> {
    let text = fit(&format!(" {label} "), width.saturating_sub(2));
    let rest = "─".repeat(width.saturating_sub(2 + text_width(&text)));
    Line::from(vec![
        Span::styled("──", t.fg(Role::Border)),
        Span::styled(text, t.fg(Role::TextDim)),
        Span::styled(rest, t.fg(Role::Border)),
    ])
}

/// `# name (n)`, at the bottom of the list. Enter filters to it (again clears); d removes it from
/// every project.
fn tag_row(t: Theme, width: usize, name: &str, count: usize, active: bool, selected: bool) -> Line<'static> {
    let lead = "  ";
    let suffix = format!(" ({count})");
    let text = fit(
        &format!("# {name}"),
        width
            .saturating_sub(text_width(lead) + text_width(&suffix))
            .max(1),
    );
    let text = text.trim_end().to_string();
    let pad = " ".repeat(width.saturating_sub(text_width(lead) + text_width(&text) + text_width(&suffix)));
    if selected {
        let sel = selected_style(t);
        return Line::from(vec![
            Span::styled(lead, sel),
            Span::styled(text, bold(sel)),
            Span::styled(format!("{suffix}{pad}"), sel),
        ]);
    }
    let style = if active {
        bold(t.fg(Role::Cyan))
    } else {
        t.fg(Role::Cyan)
    };
    Line::from(vec![
        Span::styled(lead, t.fg(Role::TextDim)),
        Span::styled(text, style),
        Span::styled(suffix, t.fg(Role::TextDim)),
        Span::raw(pad),
    ])
}

/// Fixed-width so Context/5h/7d bars line up with each other and with the "Week" budget row.
const USAGE_LABEL_WIDTH: usize = 7;

fn severity_role(severity: UsageSeverity) -> Role {
    match severity {
        UsageSeverity::Ok => Role::Green,
        UsageSeverity::Warning => Role::Yellow,
        UsageSeverity::Critical => Role::Red,
    }
}

/// `5h  ███████░░░ 73% (8:30 PM)`, or `—` when this session has no usage data recorded yet.
fn usage_row(
    t: Theme,
    width: usize,
    metric: UsageMetric,
    percent: Option<f64>,
    reset_label: Option<&str>,
) -> Line<'static> {
    let lead = "  ";
    let label = fit(metric.label(), USAGE_LABEL_WIDTH);
    let reset = reset_label
        .filter(|l| !l.is_empty())
        .map(|l| format!(" ({l})"))
        .unwrap_or_default();
    let avail = width.saturating_sub(text_width(lead)).max(1);
    let Some(percent) = percent else {
        let fitted = fit(&format!("{label} —"), avail);
        let pad = " ".repeat(width.saturating_sub(text_width(lead) + text_width(&fitted)));
        return Line::styled(format!("{lead}{fitted}{pad}"), t.fg(Role::TextDim));
    };
    let bar = render_usage_bar(percent, USAGE_BAR_WIDTH);
    let plain = format!("{label} {bar} {percent}%{reset}");
    if text_width(&plain) > avail {
        let fitted = fit(&plain, avail);
        let pad = " ".repeat(width.saturating_sub(text_width(lead) + text_width(&fitted)));
        return Line::styled(format!("{lead}{fitted}{pad}"), t.fg(Role::Text));
    }
    let pad = " ".repeat(width.saturating_sub(text_width(lead) + text_width(&plain)));
    Line::from(vec![
        Span::styled(lead, t.fg(Role::TextDim)),
        Span::styled(format!("{label} "), t.fg(Role::Text)),
        Span::styled(bar, t.fg(severity_role(usage_severity(metric, percent)))),
        Span::styled(format!(" {percent}%"), t.fg(Role::Text)),
        Span::styled(reset, t.fg(Role::TextDim)),
        Span::raw(pad),
    ])
}

/// This week's day-by-day spend against the 7d quota, below the 7d row.
fn usage_budget_row(t: Theme, width: usize, days: &[DailySpend]) -> Line<'static> {
    let lead = "  ";
    let label = fit("Week", USAGE_LABEL_WIDTH);
    let text = if days.is_empty() {
        format!("{label} —")
    } else {
        let spend: Vec<String> = days
            .iter()
            .map(|d| format!("{}{}%", d.label, d.percent.round()))
            .collect();
        format!("{label} {}", spend.join(" "))
    };
    let fitted = fit(&text, width.saturating_sub(text_width(lead)).max(1));
    let pad = " ".repeat(width.saturating_sub(text_width(lead) + text_width(&fitted)));
    Line::from(vec![
        Span::styled(lead, t.fg(Role::TextDim)),
        Span::styled(fitted, t.fg(Role::Text)),
        Span::raw(pad),
    ])
}

/// Lines for the sessions panel: `height` lines, `width` columns each.
pub fn render_list_panel(
    t: Theme,
    width: usize,
    height: usize,
    rows: &[ListRow],
    selected: usize,
    note: &str,
    empty_message: &str,
) -> Vec<Line<'static>> {
    let mut lines = panel_header(t, width, "SESSIONS", note).to_vec();
    let body = height.saturating_sub(2);
    if rows.is_empty() {
        lines.push(Line::from(blank(width)));
        lines.push(Line::styled(
            fit(&format!("  {empty_message}"), width),
            t.fg(Role::TextDim),
        ));
    }
    // Once anything is checked, every session row reserves the checkbox column so they stay aligned.
    let show_checkbox = rows
        .iter()
        .any(|r| matches!(r, ListRow::Session { checked: true, .. }));
    // Keep the selection in view, roughly centered.
    let start = (selected as isize - (body / 2) as isize)
        .min(rows.len() as isize - body as isize)
        .max(0) as usize;
    for i in 0..body {
        if lines.len() >= height {
            break;
        }
        let is_selected = start + i == selected;
        lines.push(match rows.get(start + i) {
            None => Line::from(blank(width)),
            Some(ListRow::Session {
                view,
                is_last,
                depth,
                pin,
                checked,
            }) => session_row(
                t,
                width,
                SessionRow {
                    view,
                    is_last: *is_last,
                    depth: *depth,
                    pin: *pin,
                    checked: *checked,
                    show_checkbox,
                },
                is_selected,
            ),
            Some(ListRow::Divider { label }) => divider_row(t, width, label),
            Some(ListRow::Tag { name, count, active }) => {
                tag_row(t, width, name, *count, *active, is_selected)
            }
            Some(ListRow::Usage {
                metric,
                percent,
                reset_label,
            }) => usage_row(t, width, *metric, *percent, reset_label.as_deref()),
            Some(ListRow::UsageBudget { days }) => usage_budget_row(t, width, days),
            Some(ListRow::Folder {
                name,
                counts,
                collapsed,
                hotkey,
            }) => group_row(
                t,
                width,
                GroupRow {
                    is_folder: true,
                    name,
                    counts,
                    collapsed: *collapsed,
                    depth: 0,
                    hotkey: *hotkey,
                    git: None,
                },
                is_selected,
            ),
            Some(ListRow::Project {
                label,
                counts,
                collapsed,
                depth,
                hotkey,
                git,
            }) => group_row(
                t,
                width,
                GroupRow {
                    is_folder: false,
                    name: label,
                    counts,
                    collapsed: *collapsed,
                    depth: *depth,
                    hotkey: *hotkey,
                    git: git.as_ref(),
                },
                is_selected,
            ),
        });
    }
    while lines.len() < height {
        lines.push(Line::from(blank(width)));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeName;

    fn text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn view(title: &str, status: SessionStatus) -> SessionView {
        SessionView {
            title: title.into(),
            status,
            elsewhere: false,
            agent: "claude".into(),
            time_label: "now".into(),
            cwd: "~/repos/api".into(),
            id: Some("id".into()),
            detail: None,
            git: None,
            account_tag: None,
        }
    }

    fn session(view: SessionView, is_last: bool, pin: Option<SessionPin>) -> ListRow {
        ListRow::Session {
            view: Box::new(view),
            is_last,
            depth: 1,
            pin,
            checked: false,
        }
    }

    fn counts(count: usize, running: usize, waiting: usize) -> GroupCounts {
        GroupCounts {
            count,
            running,
            waiting,
        }
    }

    fn fixture() -> Vec<ListRow> {
        let mut other = view("Other account", SessionStatus::Stopped);
        other.account_tag = Some("adrianogpena".into());
        let mut away = view("Open in another terminal", SessionStatus::Idle);
        away.elsewhere = true;
        vec![
            ListRow::Folder {
                name: "Work".into(),
                counts: counts(3, 1, 0),
                collapsed: false,
                hotkey: Some(1),
            },
            ListRow::Project {
                label: "api".into(),
                counts: counts(3, 1, 1),
                collapsed: false,
                depth: 1,
                hotkey: None,
                git: Some(GitStatus {
                    branch: None,
                    ahead: 1,
                    behind: 0,
                    dirty: 2,
                }),
            },
            session(
                view("Fix the build", SessionStatus::Running),
                false,
                Some(SessionPin::Top),
            ),
            session(away, false, None),
            session(other, true, Some(SessionPin::Bottom)),
            ListRow::Project {
                label: "web".into(),
                counts: counts(1, 0, 0),
                collapsed: true,
                depth: 0,
                hotkey: Some(2),
                git: None,
            },
            ListRow::Divider { label: "TAGS".into() },
            ListRow::Tag {
                name: "urgent".into(),
                count: 2,
                active: false,
            },
            ListRow::Divider {
                label: "USAGE".into(),
            },
            ListRow::Usage {
                metric: UsageMetric::Context,
                percent: Some(42.0),
                reset_label: None,
            },
            ListRow::Usage {
                metric: UsageMetric::FiveHour,
                percent: Some(73.0),
                reset_label: Some("8:30 PM".into()),
            },
            ListRow::Usage {
                metric: UsageMetric::SevenDay,
                percent: None,
                reset_label: None,
            },
            ListRow::UsageBudget {
                days: vec![
                    DailySpend {
                        label: "Mo",
                        percent: 12.4,
                    },
                    DailySpend {
                        label: "Tu",
                        percent: 30.0,
                    },
                ],
            },
        ]
    }

    #[test]
    fn a_fixture_tree_with_every_row_kind_renders_as_expected() {
        let t = Theme::new(ThemeName::Dark);
        let lines = render_list_panel(t, 44, 18, &fixture(), 2, "· filtered", "empty");
        let rendered: Vec<String> = lines.iter().map(text).collect();
        let expected = [
            "SESSIONS · filtered",
            "────────────────────────────────────────────",
            "1▾ Work (3) ●1",
            "   ▾ api (3) ●1 ◐1 ⇡✱",
            "  ├─ ● ↑ Fix the build claude",
            "  ├─ ○ ↗ Open in another terminal claude",
            "  └─ ■ ↓ Other account adrianogpena claude",
            "2▸ web (1)",
            "── TAGS",
            "  # urgent (2)",
            "── USAGE",
            "  Context ████░░░░░░ 42%",
            "  5h      ███████░░░ 73% (8:30 PM)",
            "  7d      —",
            "  Week    Mo12% Tu30%",
            "",
            "",
            "",
        ];
        for (got, want) in rendered.iter().zip(expected) {
            assert_eq!(got.trim_end(), want.trim_end());
        }
        assert!(lines.iter().all(|l| text(l).chars().count() == 44));
    }

    #[test]
    fn a_session_row_of_another_account_dims_its_tag_and_keeps_the_status_color() {
        let t = Theme::new(ThemeName::Dark);
        let mut other = view("Other", SessionStatus::Waiting);
        other.account_tag = Some("me".into());
        let lines = render_list_panel(t, 40, 5, &[session(other, true, None)], 9, "", "empty");
        let row = &lines[2];
        let tag = row.spans.iter().find(|s| s.content == " me").unwrap();
        assert_eq!(tag.style, t.fg(Role::TextDim));
        let dot = row.spans.iter().find(|s| s.content == "◐").unwrap();
        assert_eq!(dot.style.fg, Some(t.color(Role::Yellow)));
    }

    #[test]
    fn the_selected_row_is_highlighted_across_its_full_width() {
        let t = Theme::new(ThemeName::Dark);
        let lines = render_list_panel(t, 30, 6, &fixture()[..3], 2, "", "empty");
        assert!(lines[4]
            .spans
            .iter()
            .all(|s| s.style.bg == Some(t.color(Role::Accent))));
        assert_eq!(text(&lines[4]).chars().count(), 30);
        assert!(lines[3].spans.iter().all(|s| s.style.bg.is_none()));
    }

    #[test]
    fn the_selection_stays_in_view_and_an_empty_list_shows_its_message() {
        let t = Theme::new(ThemeName::Dark);
        let rows: Vec<ListRow> = (0..30)
            .map(|i| session(view(&format!("session {i}"), SessionStatus::Idle), i == 29, None))
            .collect();
        let lines = render_list_panel(t, 30, 12, &rows, 20, "", "empty");
        let shown: Vec<String> = lines.iter().skip(2).map(text).collect();
        assert!(shown.iter().any(|l| l.contains("session 20")), "{shown:?}");
        let empty = render_list_panel(t, 30, 6, &[], 0, "", "Nothing here.");
        assert_eq!(text(&empty[3]).trim_end(), "  Nothing here.");
    }

    #[test]
    fn checked_sessions_reserve_a_checkbox_column_on_every_session_row() {
        let t = Theme::new(ThemeName::Dark);
        let mut rows = vec![
            session(view("one", SessionStatus::Idle), false, None),
            session(view("two", SessionStatus::Idle), true, None),
        ];
        if let ListRow::Session { checked, .. } = &mut rows[1] {
            *checked = true;
        }
        let lines = render_list_panel(t, 30, 6, &rows, 9, "", "empty");
        assert!(text(&lines[2]).starts_with("·   ├─ ○ one"), "{}", text(&lines[2]));
        assert!(text(&lines[3]).starts_with("✔   └─ ○ two"), "{}", text(&lines[3]));
    }
}
