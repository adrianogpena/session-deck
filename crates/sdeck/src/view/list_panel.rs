//! The sessions panel, port of `renderListPanel` and its row renderers in `view.ts`. Every line is
//! exactly `width` columns wide.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use sdeck_core::discovery::git_status::GitStatus;
use sdeck_core::status::usage_display::{
    render_usage_bar, usage_severity, UsageMetric, UsageSeverity, USAGE_BAR_WIDTH,
};
use sdeck_core::store::deck_config::UsagePosition;
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
        /// The reading is old: drawn dimmed.
        stale: bool,
    },
    UsageModel {
        label: Option<String>,
    },
    /// `(warm, detail)`: the prompt cache is alive, and how long is left or has passed.
    UsageCache {
        cache: Option<(bool, String)>,
    },
    /// `(metric, percent, stale)` of each metric that has data, on one line.
    UsageCompact {
        parts: Vec<(UsageMetric, f64, bool)>,
    },
}

/// `(glyph, role, bold)` of a session's status dot.
fn status_glyph(status: SessionStatus) -> (char, Role, bool) {
    match status {
        SessionStatus::Running => ('●', Role::Accent, true),
        SessionStatus::Waiting => ('◐', Role::Yellow, true),
        // Finished, not seen yet: waiting for a look. Same green as the extension's "Done" decoration.
        SessionStatus::Done => ('✓', Role::Green, true),
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
        spans.push(Span::styled(
            format!(" ●{}", g.counts.running),
            t.fg(Role::Accent),
        ));
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
    let agent_text = if v.show_agent {
        format!(" {}", v.agent)
    } else {
        String::new()
    };
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
    let mut title_style = t.fg(if v.other_account {
        Role::TextDim
    } else {
        Role::Text
    });
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

/// Fixed-width so Context/5h/7d bars line up with each other.
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
    stale: bool,
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
    let body_role = if stale { Role::TextDim } else { Role::Text };
    let bar = render_usage_bar(percent, USAGE_BAR_WIDTH);
    let shown = percent.round();
    let plain = format!("{label} {bar} {shown}%{reset}");
    if text_width(&plain) > avail {
        let fitted = fit(&plain, avail);
        let pad = " ".repeat(width.saturating_sub(text_width(lead) + text_width(&fitted)));
        return Line::styled(format!("{lead}{fitted}{pad}"), t.fg(body_role));
    }
    let pad = " ".repeat(width.saturating_sub(text_width(lead) + text_width(&plain)));
    Line::from(vec![
        Span::styled(lead, t.fg(Role::TextDim)),
        Span::styled(format!("{label} "), t.fg(body_role)),
        Span::styled(
            bar,
            t.fg(if stale {
                Role::TextDim
            } else {
                severity_role(usage_severity(metric, percent))
            }),
        ),
        Span::styled(format!(" {shown}%"), t.fg(body_role)),
        Span::styled(reset, t.fg(Role::TextDim)),
        Span::raw(pad),
    ])
}

/// `Model   Opus 5.5 · high`, or `—` when this session hasn't reported one.
fn usage_model_row(t: Theme, width: usize, label: Option<&str>) -> Line<'static> {
    let lead = "  ";
    let head = fit("Model", USAGE_LABEL_WIDTH);
    let text = format!("{head} {}", label.unwrap_or("—"));
    let fitted = fit(&text, width.saturating_sub(text_width(lead)).max(1));
    let pad = " ".repeat(width.saturating_sub(text_width(lead) + text_width(&fitted)));
    let role = if label.is_some() {
        Role::Text
    } else {
        Role::TextDim
    };
    Line::from(vec![
        Span::styled(lead, t.fg(Role::TextDim)),
        Span::styled(fitted, t.fg(role)),
        Span::raw(pad),
    ])
}

/// `Cache   Cached Session · 4m left` or `Cache   Fetch Session · idle 47m`, or `—` without a transcript.
fn usage_cache_row(t: Theme, width: usize, cache: Option<&(bool, String)>) -> Line<'static> {
    let lead = "  ";
    let head = fit("Cache", USAGE_LABEL_WIDTH);
    let (text, role) = match cache {
        Some((true, detail)) => (format!("{head} Cached Session · {detail}"), Role::Green),
        Some((false, detail)) => (format!("{head} Fetch Session · {detail}"), Role::Yellow),
        None => (format!("{head} —"), Role::TextDim),
    };
    let fitted = fit(&text, width.saturating_sub(text_width(lead)).max(1));
    let pad = " ".repeat(width.saturating_sub(text_width(lead) + text_width(&fitted)));
    Line::from(vec![
        Span::styled(lead, t.fg(Role::TextDim)),
        Span::styled(fitted, t.fg(role)),
        Span::raw(pad),
    ])
}

/// `ctx 42% · 5h 73% · 7d 12%`, or `—` when no metric has data.
fn usage_compact_row(t: Theme, width: usize, parts: &[(UsageMetric, f64, bool)]) -> Line<'static> {
    let lead = "  ";
    let avail = width.saturating_sub(text_width(lead)).max(1);
    let pieces: Vec<(String, String, Role)> = parts
        .iter()
        .map(|&(metric, percent, stale)| {
            let name = match metric {
                UsageMetric::Context => "ctx",
                other => other.label(),
            };
            let role = if stale {
                Role::TextDim
            } else {
                severity_role(usage_severity(metric, percent))
            };
            (format!("{name} "), format!("{}%", percent.round()), role)
        })
        .collect();
    let plain = if pieces.is_empty() {
        "—".to_string()
    } else {
        pieces
            .iter()
            .map(|(name, value, _)| format!("{name}{value}"))
            .collect::<Vec<_>>()
            .join(" · ")
    };
    if pieces.is_empty() || text_width(&plain) > avail {
        let fitted = fit(&plain, avail);
        let pad = " ".repeat(width.saturating_sub(text_width(lead) + text_width(&fitted)));
        return Line::styled(format!("{lead}{fitted}{pad}"), t.fg(Role::TextDim));
    }
    let mut spans = vec![Span::styled(lead, t.fg(Role::TextDim))];
    for (i, (name, value, role)) in pieces.into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", t.fg(Role::TextDim)));
        }
        spans.push(Span::styled(name, t.fg(Role::TextDim)));
        spans.push(Span::styled(value, t.fg(role)));
    }
    spans.push(Span::raw(
        " ".repeat(width.saturating_sub(text_width(lead) + text_width(&plain))),
    ));
    Line::from(spans)
}

fn list_row_line(
    t: Theme,
    width: usize,
    row: &ListRow,
    is_selected: bool,
    show_checkbox: bool,
) -> Line<'static> {
    match row {
        ListRow::Session {
            view,
            is_last,
            depth,
            pin,
            checked,
        } => session_row(
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
        ListRow::Divider { label } => divider_row(t, width, label),
        ListRow::Tag { name, count, active } => tag_row(t, width, name, *count, *active, is_selected),
        ListRow::Usage {
            metric,
            percent,
            reset_label,
            stale,
        } => usage_row(t, width, *metric, *percent, reset_label.as_deref(), *stale),
        ListRow::UsageModel { label } => usage_model_row(t, width, label.as_deref()),
        ListRow::UsageCache { cache } => usage_cache_row(t, width, cache.as_ref()),
        ListRow::UsageCompact { parts } => usage_compact_row(t, width, parts),
        ListRow::Folder {
            name,
            counts,
            collapsed,
            hotkey,
        } => group_row(
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
        ListRow::Project {
            label,
            counts,
            collapsed,
            depth,
            hotkey,
            git,
        } => group_row(
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
    }
}

/// Fewest tree rows a pinned usage block may leave; below that the block scrolls with the rows instead.
const MIN_TREE_ROWS: usize = 3;

/// Lines for the sessions panel: `height` lines, `width` columns each. With `usage_position` `Top` or
/// `Bottom`, the trailing USAGE block is pinned there and only the rows before it scroll.
#[allow(clippy::too_many_arguments)]
pub fn render_list_panel(
    t: Theme,
    width: usize,
    height: usize,
    rows: &[ListRow],
    selected: usize,
    note: &str,
    empty_message: &str,
    usage_position: UsagePosition,
) -> Vec<Line<'static>> {
    let mut lines = panel_header(t, width, "SESSIONS", note).to_vec();
    let usage_start = rows
        .iter()
        .rposition(|r| matches!(r, ListRow::Divider { label } if label == "USAGE"));
    let pinned_rows = usage_start.map_or(0, |at| rows.len() - at + 1);
    let usage_position = if usage_position != UsagePosition::Float
        && height.saturating_sub(lines.len() + pinned_rows) < MIN_TREE_ROWS
    {
        UsagePosition::Float
    } else {
        usage_position
    };
    let split = match usage_position {
        UsagePosition::Float => None,
        _ => usage_start,
    };
    let (tree, block) = rows.split_at(split.unwrap_or(rows.len()));
    let block_lines = |lines: &mut Vec<Line<'static>>| {
        for row in block {
            lines.push(list_row_line(t, width, row, false, false));
        }
    };
    if usage_position == UsagePosition::Top {
        block_lines(&mut lines);
        if !block.is_empty() {
            lines.push(Line::from(blank(width)));
        }
    }
    let reserved = if usage_position == UsagePosition::Bottom && !block.is_empty() {
        block.len() + 1
    } else {
        0
    };
    let tree_end = height.saturating_sub(reserved);
    let body = tree_end.saturating_sub(lines.len());
    if tree.is_empty() {
        lines.push(Line::from(blank(width)));
        lines.push(Line::styled(
            fit(&format!("  {empty_message}"), width),
            t.fg(Role::TextDim),
        ));
    }
    // Once anything is checked, every session row reserves the checkbox column so they stay aligned.
    let show_checkbox = tree
        .iter()
        .any(|r| matches!(r, ListRow::Session { checked: true, .. }));
    // Keep the selection in view, roughly centered.
    let start = (selected as isize - (body / 2) as isize)
        .min(tree.len() as isize - body as isize)
        .max(0) as usize;
    for i in 0..body {
        if lines.len() >= tree_end {
            break;
        }
        let is_selected = start + i == selected;
        lines.push(match tree.get(start + i) {
            None => Line::from(blank(width)),
            Some(row) => list_row_line(t, width, row, is_selected, show_checkbox),
        });
    }
    while lines.len() < tree_end {
        lines.push(Line::from(blank(width)));
    }
    if usage_position == UsagePosition::Bottom {
        block_lines(&mut lines);
    }
    lines.truncate(height);
    while lines.len() < height {
        lines.push(Line::from(blank(width)));
    }
    lines
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_status_has_its_own_glyph_except_error_and_exited() {
        use SessionStatus::*;
        let glyphs: Vec<char> = [Running, Done, Waiting, Idle, Starting, Error, Stopped]
            .map(|s| status_glyph(s).0)
            .to_vec();
        let mut unique = glyphs.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), glyphs.len(), "{glyphs:?}");
        assert_eq!(status_glyph(Error).0, status_glyph(Exited).0);
        assert_eq!(status_glyph(Running), ('●', Role::Accent, true));
        assert_eq!(status_glyph(Done), ('✓', Role::Green, true));
    }

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
            show_agent: false,
            time_label: "now".into(),
            cwd: "~/repos/api".into(),
            id: Some("id".into()),
            detail: None,
            git: None,
            account_tag: None,
            other_account: false,
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
        away.agent = "copilot".into();
        away.show_agent = true;
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
            ListRow::UsageModel {
                label: Some("Opus 5.5 · high".into()),
            },
            ListRow::UsageCache {
                cache: Some((true, "4m left".into())),
            },
            ListRow::Usage {
                metric: UsageMetric::Context,
                percent: Some(42.0),
                reset_label: None,
                stale: false,
            },
            ListRow::Usage {
                metric: UsageMetric::FiveHour,
                percent: Some(73.0),
                reset_label: Some("8:30 PM".into()),
                stale: false,
            },
            ListRow::Usage {
                metric: UsageMetric::SevenDay,
                percent: None,
                reset_label: None,
                stale: false,
            },
        ]
    }

    #[test]
    fn a_fixture_tree_with_every_row_kind_renders_as_expected() {
        let t = Theme::new(ThemeName::Dark);
        let lines = render_list_panel(
            t,
            44,
            19,
            &fixture(),
            2,
            "· filtered",
            "empty",
            UsagePosition::Float,
        );
        let rendered: Vec<String> = lines.iter().map(text).collect();
        let expected = [
            "SESSIONS · filtered",
            "────────────────────────────────────────────",
            "1▾ Work (3) ●1",
            "   ▾ api (3) ●1 ◐1 ⇡✱",
            "  ├─ ● ↑ Fix the build",
            "  ├─ ○ ↗ Open in another terminal copilot",
            "  └─ ■ ↓ Other account adrianogpena",
            "2▸ web (1)",
            "── TAGS",
            "  # urgent (2)",
            "── USAGE",
            "  Model   Opus 5.5 · high",
            "  Cache   Cached Session · 4m left",
            "  Context ████░░░░░░ 42%",
            "  5h      ███████░░░ 73% (8:30 PM)",
            "  7d      —",
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
    fn a_pinned_usage_block_sits_at_the_top_or_bottom_and_the_tree_scrolls_around_it() {
        let t = Theme::new(ThemeName::Dark);
        let rows = fixture();
        for (position, usage_at) in [(UsagePosition::Top, 2), (UsagePosition::Bottom, 7)] {
            let lines = render_list_panel(t, 44, 14, &rows, 2, "", "empty", position);
            let rendered: Vec<String> = lines.iter().map(|l| text(l).trim_end().to_string()).collect();
            assert_eq!(rendered.len(), 14);
            assert_eq!(rendered[usage_at], "── USAGE", "{position:?}");
            assert_eq!(rendered[usage_at + 1], "  Model   Opus 5.5 · high");
            assert_eq!(rendered[usage_at + 5], "  7d      —");
            assert!(rendered.iter().any(|r| r.contains("Fix the build")));
        }
    }

    #[test]
    fn a_compact_usage_block_is_one_line_in_every_position() {
        let t = Theme::new(ThemeName::Dark);
        let mut rows = fixture();
        let at = rows
            .iter()
            .position(|r| matches!(r, ListRow::UsageModel { .. }))
            .unwrap();
        rows.truncate(at);
        rows.push(ListRow::UsageCompact {
            parts: vec![
                (UsageMetric::Context, 42.0, false),
                (UsageMetric::FiveHour, 73.0, false),
                (UsageMetric::SevenDay, 12.0, true),
            ],
        });
        for position in [UsagePosition::Float, UsagePosition::Top, UsagePosition::Bottom] {
            let lines = render_list_panel(t, 44, 14, &rows, 2, "", "empty", position);
            let rendered: Vec<String> = lines.iter().map(|l| text(l).trim_end().to_string()).collect();
            let usage_at = rendered.iter().position(|r| r == "── USAGE").unwrap();
            assert_eq!(
                rendered[usage_at + 1],
                "  ctx 42% · 5h 73% · 7d 12%",
                "{position:?}"
            );
            assert!(lines.iter().all(|l| text(l).chars().count() == 44));
        }
        let narrow = text(&list_row_line(t, 14, rows.last().unwrap(), false, false));
        assert_eq!(narrow.chars().count(), 14);
        let empty = ListRow::UsageCompact { parts: vec![] };
        assert_eq!(
            text(&list_row_line(t, 20, &empty, false, false)).trim_end(),
            "  —"
        );
    }

    #[test]
    fn a_pinned_usage_block_that_would_starve_the_tree_scrolls_with_the_rows_instead() {
        let t = Theme::new(ThemeName::Dark);
        for position in [UsagePosition::Top, UsagePosition::Bottom] {
            let lines = render_list_panel(t, 60, 8, &fixture(), 2, "", "empty", position);
            assert_eq!(lines.len(), 8);
            assert!(
                lines.iter().any(|l| text(l).contains("Fix the build")),
                "{position:?}"
            );
            assert!(lines.iter().all(|l| text(l).chars().count() == 60));
        }
    }

    #[test]
    fn a_session_row_of_another_account_dims_its_tag_and_keeps_the_status_color() {
        let t = Theme::new(ThemeName::Dark);
        let mut other = view("Other", SessionStatus::Waiting);
        other.account_tag = Some("me".into());
        other.other_account = true;
        let lines = render_list_panel(
            t,
            40,
            5,
            &[session(other, true, None)],
            9,
            "",
            "empty",
            UsagePosition::Float,
        );
        let row = &lines[2];
        let tag = row.spans.iter().find(|s| s.content == " me").unwrap();
        assert_eq!(tag.style, t.fg(Role::TextDim));
        let dot = row.spans.iter().find(|s| s.content == "◐").unwrap();
        assert_eq!(dot.style.fg, Some(t.color(Role::Yellow)));
        let title = row.spans.iter().find(|s| s.content == "Other").unwrap();
        assert_eq!(title.style.fg, Some(t.color(Role::TextDim)));
    }

    #[test]
    fn the_selected_row_is_highlighted_across_its_full_width() {
        let t = Theme::new(ThemeName::Dark);
        let lines = render_list_panel(t, 30, 6, &fixture()[..3], 2, "", "empty", UsagePosition::Float);
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
        let lines = render_list_panel(t, 30, 12, &rows, 20, "", "empty", UsagePosition::Float);
        let shown: Vec<String> = lines.iter().skip(2).map(text).collect();
        assert!(shown.iter().any(|l| l.contains("session 20")), "{shown:?}");
        let empty = render_list_panel(t, 30, 6, &[], 0, "", "Nothing here.", UsagePosition::Float);
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
        let lines = render_list_panel(t, 30, 6, &rows, 9, "", "empty", UsagePosition::Float);
        assert!(text(&lines[2]).starts_with("·   ├─ ○ one"), "{}", text(&lines[2]));
        assert!(text(&lines[3]).starts_with("✔   └─ ○ two"), "{}", text(&lines[3]));
    }
}
