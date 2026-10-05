//! The popups opened by `?`, `C`, `w`, `v`, `a`, `/` and `:` — port of `helpOverlay`,
//! `configOverlay`, `skillsOverlay`, `traceOverlay`, `alertsOverlay`, `searchOverlay` and
//! `commandPaletteOverlay` in `view.ts`. Scrollable ones clamp their offset to the content while
//! drawing, so the scroll position lives in a [`Cell`].

use std::cell::Cell;

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::Frame;
use sdeck_core::agent_catalog::agent_display_name;
use sdeck_core::discovery::claude_trace::{TraceKind, TraceStep};
use sdeck_core::format::humanize_since;
use sdeck_core::status::alert_log::AlertEntry;
use sdeck_core::status::session_status::SessionStatus as AlertStatus;
use sdeck_core::status::usage_display::{
    render_usage_bar, usage_severity, DailySpend, UsageMetric, UsageSeverity,
};
use sdeck_core::store::deck_config::DeckConfig;

use super::list_panel::glyph;
use super::overlay::draw_box;
use super::SessionView;
use crate::agents::LocalAgent;
use crate::ansi::{fit, fit_tail, text_width, wrap};
use crate::config_fields::{ConfigFieldKind, CONFIG_FIELDS};
use crate::sessions::SessionStatus;
use crate::skills::{LocalSkill, SkillState};
use crate::theme::{Role, Theme};

const KEY_COLUMN: usize = 14;
const POPUP_NAME_COLUMN: usize = 32;
/// A `command` override can be a long absolute path; past this width it is cut (keeping the tail).
const CONFIG_VALUE_COLUMN_MAX: usize = 40;

type Hint = (&'static str, &'static str);

const HELP_SECTIONS: &[(&str, &[Hint])] = &[
    (
        "QUICK START",
        &[
            ("Enter", "Attach full-screen (starts it if needed)"),
            ("Ctrl+Q", "Detach back here; the session keeps running"),
            ("Ctrl+K q", "Detach and stop the session, same as x"),
            ("Ctrl+K n", "New session in the same project, without detaching first"),
            ("i", "Interact — type into it right here, list and preview still showing"),
            ("Ctrl+K T", "Swap Attached ⇄ Interacting, without detaching to the list first"),
        ],
    ),
    (
        "NAVIGATION",
        &[
            ("j k", "Select (preview follows)"),
            ("↑ ↓", "Select, or scroll the preview if it has a live session in it"),
            ("PgUp/Dn  Home/End", "Scroll the selected session's preview"),
            ("← →  Tab", "Collapse / expand; ← also goes to the parent"),
            ("1-9", "Jump to a top-level folder or project"),
            ("`", "Back to the previously selected session"),
            ("[ ]", "Previous / next started session (running, waiting or idle), wrapping around"),
            ("/", "Search every session's prompts and replies"),
            (":", "Command palette: run any action by typed name"),
        ],
    ),
    (
        "SESSIONS",
        &[
            ("s", "Start in the background"),
            ("R", "Restart (a fresh process, same conversation)"),
            ("n", "New session in the project, with n's default agent"),
            ("N", "New session in the project, choosing the agent just this once (doesn't change n's default)"),
            ("F3", "Pick n's default agent"),
            ("F4", "Switch account: new sessions launch as it"),
            ("p", "Add a project: new session in any folder"),
            ("o", "Send a one-line prompt without attaching"),
            ("c", "Copy the last response"),
            ("v", "Trajectory: structured trace of messages and tool calls (Claude sessions only)"),
            ("e  F2", "Rename (same as Claude's /rename)"),
            ("Ctrl+L", "Clear context (same as Claude's /clear), without attaching — idle, done, or error only"),
            ("x", "Stop the selected session"),
            ("u", "Mark as unread (finished, not seen)"),
            ("U", "Mark as read"),
            ("A", "Archive / unarchive (^ shows archived)"),
            ("d", "Delete: move to the trash"),
            ("Ctrl+Z  Z", "Undo the delete / open the trash"),
            (",", "Pin: top · bottom · off"),
            ("r", "Refresh the list"),
        ],
    ),
    (
        "MULTI-SELECT",
        &[
            ("Space", "Check the session for a batch action, then move down"),
            ("Esc", "Clear the checked sessions"),
            ("A x d M L", "Archive / stop / delete / move to folder / tag — applied to every checked session"),
        ],
    ),
    (
        "FOLDERS & ORDER",
        &[
            ("g", "New folder"),
            ("M", "Move the project to a folder"),
            ("K J", "Move the folder / project up or down"),
            ("e  d", "Rename / delete the selected folder"),
            ("d", "Remove the selected project from the list (p to add it back)"),
            ("S", "Sort sessions: recent · actionable"),
            ("t", "View: normal · active on top"),
        ],
    ),
    (
        "TAGS",
        &[
            ("L", "Add / edit tags on the session (checked batch: add to all)"),
            ("Enter", "On a tag (bottom of the list): filter to it, again to clear"),
            ("d", "On a tag: remove it from every session"),
        ],
    ),
    (
        "FILTER",
        &[
            ("!  @  #", "Running · waiting · idle"),
            ("&  ~", "Error · stopped"),
            ("*", "Time: all · today · 3 days · 7 days"),
            ("0", "Clear filters"),
        ],
    ),
    (
        "VIEW",
        &[
            ("< >", "Narrow / widen the sessions panel"),
            ("b  Ctrl+K b", "Hide / show the sessions panel"),
            (":  Choose theme", "Pick a palette (Tokyo Night, Catppuccin, Gruvbox, Nord, Dracula, Rose Pine, Solarized), or follow the system"),
            ("m  Ctrl+K m", "Mouse scrolling: off by default (so click-drag selects text), on to scroll the preview with the wheel"),
        ],
    ),
    (
        "OTHER",
        &[
            ("?", "This help"),
            ("C", "Show the config file in use"),
            ("w", "Show local skills / agents / account usage (← → switches tabs)"),
            ("a", "Alert history: every status change sdeck has noticed"),
            ("q  Ctrl+C", "Quit (stops background sessions)"),
        ],
    ),
];

fn surface(t: Theme) -> Style {
    t.bg(Role::Surface)
}

fn text(t: Theme, role: Role) -> Style {
    surface(t).patch(t.fg(role))
}

fn bold(style: Style) -> Style {
    style.add_modifier(Modifier::BOLD)
}

fn selected(t: Theme) -> Style {
    Style::new()
        .bg(t.color(Role::Accent))
        .fg(t.color(Role::Bg))
        .add_modifier(Modifier::BOLD)
}

fn blank(t: Theme, width: usize) -> Line<'static> {
    Line::styled(" ".repeat(width), surface(t))
}

/// A content line with two spaces of padding on each side, ready for [`draw_box`].
fn padded(t: Theme, content: Line<'static>) -> Line<'static> {
    let pad = || Span::styled("  ", surface(t));
    let mut spans = vec![pad()];
    spans.extend(content.spans);
    spans.push(pad());
    Line::from(spans)
}

fn more_row(t: Theme, width: usize, offset: usize, max_scroll: usize) -> Line<'static> {
    let note = if offset < max_scroll {
        "  ▼ more below"
    } else if offset > 0 {
        "  ▲ more above"
    } else {
        ""
    };
    Line::styled(fit(note, width - 2), text(t, Role::Yellow))
}

fn dim(t: Theme, note: &str, inner: usize) -> Line<'static> {
    Line::styled(fit(note, inner), text(t, Role::TextDim))
}

/// Draws `head` rows, the visible slice of `content` from `offset`, then the "more" row.
#[allow(clippy::too_many_arguments)]
fn draw_scrolled(
    frame: &mut Frame,
    t: Theme,
    title: &str,
    width: usize,
    head: Vec<Line<'static>>,
    content: Vec<Line<'static>>,
    offset: usize,
    max_body: usize,
) {
    let max_scroll = content.len().saturating_sub(max_body);
    let mut rows = head;
    rows.extend(
        content
            .into_iter()
            .skip(offset)
            .take(max_body)
            .map(|line| padded(t, line)),
    );
    rows.push(more_row(t, width, offset, max_scroll));
    draw_box(frame, t, title, width, rows);
}

/// The popup's width and inner (padded) width, or `None` when the screen is too narrow.
fn popup_width(frame: &Frame, widest: usize) -> Option<(usize, usize)> {
    let width = usize::from(frame.area().width).saturating_sub(4).min(widest);
    (width >= 8).then(|| (width, width - 6))
}

/// `?`: the key reference. `scroll` is clamped here to what the content allows.
pub fn render_help(
    frame: &mut Frame,
    t: Theme,
    scroll: &Cell<usize>,
    version: &str,
    theme: &str,
    detach_key: &str,
) {
    let Some((width, inner)) = popup_width(frame, 66) else {
        return;
    };
    let mut content = Vec::new();
    for (title, keys) in HELP_SECTIONS {
        content.push(Line::styled(fit(title, inner), bold(text(t, Role::Cyan))));
        for (key, label) in *keys {
            let key = if *key == "Ctrl+Q" { detach_key } else { key };
            content.push(Line::from(vec![
                Span::styled(fit(key, KEY_COLUMN), bold(text(t, Role::Purple))),
                Span::styled(fit(label, inner.saturating_sub(KEY_COLUMN)), text(t, Role::Text)),
            ]));
        }
        content.push(blank(t, inner));
    }
    content.push(dim(
        t,
        &format!("sdeck v{version} · theme {theme} · Esc or ? to close"),
        inner,
    ));

    let max_body = usize::from(frame.area().height).saturating_sub(6).max(3);
    let offset = scroll.get().min(content.len().saturating_sub(max_body));
    scroll.set(offset);
    draw_scrolled(
        frame,
        t,
        " KEYBOARD SHORTCUTS ",
        width,
        vec![blank(t, width - 2)],
        content,
        offset,
        max_body,
    );
}

/// `C`: every setting, `selected` indexing [`CONFIG_FIELDS`]; scrolls to keep the selected row in view.
pub fn render_config(frame: &mut Frame, t: Theme, config: &DeckConfig, path: &str, selected_index: usize) {
    let fields = &*CONFIG_FIELDS;
    let footer = "↑↓ select · Enter toggle/edit · Esc or C to close";
    let label_column = fields
        .iter()
        .map(|f| f.pretty_name().chars().count())
        .max()
        .unwrap_or(0)
        + 2;
    let max_value = fields
        .iter()
        .map(|f| text_width(&f.display(config)))
        .max()
        .unwrap_or(0)
        .min(CONFIG_VALUE_COLUMN_MAX);
    let needed = (label_column + max_value).max(text_width(footer));
    let width = usize::from(frame.area().width).saturating_sub(4).min(needed + 6);
    if width < 8 {
        return;
    }
    let inner = width - 6;
    let value_width = inner.saturating_sub(label_column);

    let mut content = vec![dim(t, path, inner), blank(t, inner)];
    let mut selected_line = 2;
    for (index, field) in fields.iter().enumerate() {
        let group = field.group();
        if index == 0 || group != fields[index - 1].group() {
            if index > 0 {
                content.push(blank(t, inner));
            }
            content.push(Line::styled(
                fit(&group_title(group), inner),
                bold(text(t, Role::Cyan)),
            ));
        }
        if index == selected_index {
            selected_line = content.len();
        }
        let label = fit(&field.pretty_name(), label_column);
        let value = field.display(config);
        // A path's meaningful part is its end: keep that visible, and pad so the border stays aligned.
        let value = if field.kind == ConfigFieldKind::Text {
            let cut = fit_tail(&value, value_width);
            let pad = value_width.saturating_sub(text_width(&cut));
            format!("{cut}{}", " ".repeat(pad))
        } else {
            fit(&value, value_width)
        };
        content.push(if index == selected_index {
            Line::styled(format!("{label}{value}"), selected(t))
        } else {
            Line::from(vec![
                Span::styled(label, bold(text(t, Role::Purple))),
                Span::styled(value, text(t, Role::Text)),
            ])
        });
    }
    content.push(blank(t, inner));
    if let Some(hint) = fields.get(selected_index).and_then(|f| f.hint) {
        for line in wrap(hint, inner) {
            content.push(dim(t, &line, inner));
        }
        content.push(blank(t, inner));
    }
    content.push(dim(t, footer, inner));

    let max_body = usize::from(frame.area().height).saturating_sub(6).max(3);
    let max_scroll = content.len().saturating_sub(max_body);
    let offset = selected_line.saturating_sub(max_body / 2).min(max_scroll);
    draw_scrolled(
        frame,
        t,
        " CONFIG ",
        width,
        vec![blank(t, width - 2)],
        content,
        offset,
        max_body,
    );
}

fn group_title(group: &str) -> String {
    match group {
        "ui" => "General".to_string(),
        "trash" => "Trash".to_string(),
        "accounts" => "Accounts".to_string(),
        _ => match group.strip_prefix("tools.") {
            Some(agent) => agent_display_name(agent),
            None => group.to_string(),
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillsTab {
    Skills,
    Agents,
    Usage,
}

impl SkillsTab {
    pub fn next(self) -> Self {
        match self {
            Self::Skills => Self::Agents,
            Self::Agents => Self::Usage,
            Self::Usage => Self::Skills,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Skills => Self::Usage,
            Self::Agents => Self::Skills,
            Self::Usage => Self::Agents,
        }
    }
}

/// One account's rate-limit readings and this week's day-by-day spend, for the Usage tab.
#[derive(Debug, Clone, PartialEq)]
pub struct AccountUsage {
    pub name: String,
    pub active: bool,
    pub five_hour_percent: Option<f64>,
    pub five_hour_reset_label: Option<String>,
    pub seven_day_percent: Option<f64>,
    pub seven_day_reset_label: Option<String>,
    /// The readings are old: the account's sessions have been idle.
    pub stale: bool,
    pub days: Vec<DailySpend>,
}

const USAGE_TAB_BAR_WIDTH: usize = 24;
const USAGE_TAB_LABEL_WIDTH: usize = 6;

fn usage_tab_row(
    t: Theme,
    inner: usize,
    label: &str,
    metric: UsageMetric,
    percent: Option<f64>,
    reset: Option<&str>,
    stale: bool,
) -> Line<'static> {
    let body = if stale { Role::TextDim } else { Role::Text };
    let head = format!("  {} ", fit(label, USAGE_TAB_LABEL_WIDTH));
    let Some(percent) = percent else {
        return Line::styled(fit(&format!("{head}—"), inner), text(t, Role::TextDim));
    };
    let bar_role = if stale {
        Role::TextDim
    } else {
        match usage_severity(metric, percent) {
            UsageSeverity::Ok => Role::Green,
            UsageSeverity::Warning => Role::Yellow,
            UsageSeverity::Critical => Role::Red,
        }
    };
    let percent_text = format!(" {:>3}%", percent.round());
    let reset = reset
        .filter(|r| !r.is_empty())
        .map(|r| format!("  resets {r}"))
        .unwrap_or_default();
    let used = text_width(&head) + USAGE_TAB_BAR_WIDTH + text_width(&percent_text) + text_width(&reset);
    Line::from(vec![
        Span::styled(head, text(t, body)),
        Span::styled(render_usage_bar(percent, USAGE_TAB_BAR_WIDTH), text(t, bar_role)),
        Span::styled(percent_text, text(t, body)),
        Span::styled(reset, text(t, Role::TextDim)),
        Span::styled(" ".repeat(inner.saturating_sub(used)), surface(t)),
    ])
}

fn usage_lines(t: Theme, inner: usize, accounts: &[AccountUsage], agent_id: &str) -> Vec<Line<'static>> {
    if agent_id != "claude" {
        return vec![dim(t, NOT_CLAUDE, inner)];
    }
    if accounts.is_empty() {
        return vec![dim(t, "No Claude accounts found.", inner)];
    }
    let mut content = Vec::new();
    for a in accounts {
        let suffix = if a.active { " (active)" } else { "" };
        content.push(Line::styled(
            fit(&format!("{}{suffix}", a.name), inner),
            bold(text(t, Role::Cyan)),
        ));
        content.push(usage_tab_row(
            t,
            inner,
            UsageMetric::FiveHour.label(),
            UsageMetric::FiveHour,
            a.five_hour_percent,
            a.five_hour_reset_label.as_deref(),
            a.stale,
        ));
        content.push(usage_tab_row(
            t,
            inner,
            UsageMetric::SevenDay.label(),
            UsageMetric::SevenDay,
            a.seven_day_percent,
            a.seven_day_reset_label.as_deref(),
            a.stale,
        ));
        content.push(dim(
            t,
            "  Share of the 7d quota spent per day, latest first",
            inner,
        ));
        for d in &a.days {
            content.push(usage_tab_row(
                t,
                inner,
                &format!("  {}", d.label),
                UsageMetric::SevenDay,
                d.percent,
                None,
                true,
            ));
        }
        content.push(blank(t, inner));
    }
    content
}

const NOT_CLAUDE: &str =
    "Skills, subagents and usage are a Claude Code concept — press F3 and switch the Session Deck Agent to Claude to see them.";

fn name_row(t: Theme, inner: usize, name: &str, description: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(fit(name, POPUP_NAME_COLUMN), bold(text(t, Role::Purple))),
        Span::styled(
            fit(description, inner.saturating_sub(POPUP_NAME_COLUMN)),
            text(t, Role::Text),
        ),
    ])
}

fn skills_lines(t: Theme, inner: usize, skills: &[LocalSkill], agent_id: &str) -> Vec<Line<'static>> {
    if agent_id != "claude" {
        return vec![dim(t, NOT_CLAUDE, inner)];
    }
    let mut content = Vec::new();
    if skills.is_empty() {
        content.push(dim(
            t,
            "No local skills found in the active account's skills folder.",
            inner,
        ));
    }
    for state in SkillState::ALL {
        let group: Vec<&LocalSkill> = skills.iter().filter(|s| s.state == state).collect();
        if group.is_empty() {
            continue;
        }
        content.push(Line::styled(
            fit(&format!("{} ({})", state.label(), group.len()), inner),
            bold(text(t, Role::Cyan)),
        ));
        content.extend(group.iter().map(|s| name_row(t, inner, &s.name, &s.description)));
        content.push(blank(t, inner));
    }
    content
}

fn agents_lines(t: Theme, inner: usize, agents: &[LocalAgent], agent_id: &str) -> Vec<Line<'static>> {
    if agent_id != "claude" {
        return vec![dim(t, NOT_CLAUDE, inner)];
    }
    if agents.is_empty() {
        return vec![dim(
            t,
            "No local agents found in the active account's agents folder.",
            inner,
        )];
    }
    agents
        .iter()
        .map(|a| name_row(t, inner, &a.name, &a.description))
        .collect()
}

fn tabs_row(t: Theme, active: SkillsTab, inner: usize) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, (tab, label)) in [
        (SkillsTab::Skills, " Skills "),
        (SkillsTab::Agents, " Agents "),
        (SkillsTab::Usage, " Usage "),
    ]
    .into_iter()
    .enumerate()
    {
        if i > 0 {
            spans.push(Span::styled("  ", surface(t)));
        }
        spans.push(Span::styled(
            label,
            if tab == active {
                selected(t)
            } else {
                text(t, Role::TextDim)
            },
        ));
    }
    let used: usize = spans.iter().map(|s| text_width(&s.content)).sum();
    spans.push(Span::styled(" ".repeat(inner.saturating_sub(used)), surface(t)));
    Line::from(spans)
}

/// `w`: local skills (grouped by state), subagents, or per-account usage. All are a Claude Code concept: with another
/// Session Deck Agent selected, the tabs say so instead.
#[allow(clippy::too_many_arguments)]
pub fn render_skills(
    frame: &mut Frame,
    t: Theme,
    tab: SkillsTab,
    skills: &[LocalSkill],
    agents: &[LocalAgent],
    usage: &[AccountUsage],
    scroll: &Cell<usize>,
    agent_id: &str,
) {
    let Some((width, inner)) = popup_width(frame, 88) else {
        return;
    };
    let mut content = match tab {
        SkillsTab::Skills => skills_lines(t, inner, skills, agent_id),
        SkillsTab::Agents => agents_lines(t, inner, agents, agent_id),
        SkillsTab::Usage => usage_lines(t, inner, usage, agent_id),
    };
    content.push(blank(t, inner));
    content.push(dim(t, "← → switch tabs · Esc or w to close", inner));

    let max_body = usize::from(frame.area().height).saturating_sub(7).max(3);
    let offset = scroll.get().min(content.len().saturating_sub(max_body));
    scroll.set(offset);
    let title = format!(
        " SKILLS & AGENTS · {} ",
        agent_display_name(agent_id).to_uppercase()
    );
    draw_scrolled(
        frame,
        t,
        &title,
        width,
        vec![padded(t, tabs_row(t, tab, inner)), blank(t, width - 2)],
        content,
        offset,
        max_body,
    );
}

fn trace_prefix(kind: TraceKind) -> &'static str {
    match kind {
        TraceKind::User => "You",
        TraceKind::Assistant => "Claude",
        TraceKind::Tool => "Tool",
    }
}

fn trace_role(kind: TraceKind) -> Role {
    match kind {
        TraceKind::User => Role::Cyan,
        TraceKind::Assistant => Role::Orange,
        TraceKind::Tool => Role::Purple,
    }
}

/// `v`: one Claude session's prompts, replies and tool calls. Left, the step list; right, the
/// selected step's full detail, which scrolls on its own.
pub fn render_trace(
    frame: &mut Frame,
    t: Theme,
    steps: &[TraceStep],
    selected_index: usize,
    detail_scroll: &Cell<usize>,
) {
    let Some((width, inner)) = popup_width(frame, 110) else {
        return;
    };
    let left_width = (inner * 34 / 100).clamp(16, 36).min(inner);
    let right_width = inner.saturating_sub(left_width + 1).max(10);
    let body_height = usize::from(frame.area().height).saturating_sub(7).max(3);

    let start = selected_index
        .saturating_sub(body_height / 2)
        .min(steps.len().saturating_sub(body_height));
    let mut left_rows: Vec<Span<'static>> = Vec::new();
    for i in 0..body_height {
        let span = match steps.get(start + i) {
            Some(step) => {
                let marker = if step.is_error { "✕ " } else { "" };
                let line = fit(
                    &format!(" {}: {marker}{}", trace_prefix(step.kind), step.label),
                    left_width,
                );
                if start + i == selected_index {
                    Span::styled(line, selected(t))
                } else {
                    let role = if step.is_error {
                        Role::Red
                    } else {
                        trace_role(step.kind)
                    };
                    Span::styled(line, text(t, role))
                }
            }
            None if i == 0 && steps.is_empty() => Span::styled(
                fit("No messages or tool calls yet.", left_width),
                text(t, Role::TextDim),
            ),
            None => Span::styled(" ".repeat(left_width), surface(t)),
        };
        left_rows.push(span);
    }

    let detail = steps
        .get(selected_index)
        .map(|s| wrap(&s.detail, right_width.saturating_sub(1).max(10)))
        .unwrap_or_default();
    let max_detail_scroll = detail.len().saturating_sub(body_height);
    let offset = detail_scroll.get().min(max_detail_scroll);
    detail_scroll.set(offset);

    let mut rows = vec![blank(t, width - 2)];
    for (i, left_span) in left_rows.into_iter().enumerate() {
        let right = detail.get(offset + i).map_or("", String::as_str);
        rows.push(padded(
            t,
            Line::from(vec![
                left_span,
                Span::styled("│", text(t, Role::Border)),
                Span::styled(fit(right, right_width), text(t, Role::Text)),
            ]),
        ));
    }
    let note = match max_detail_scroll {
        0 => "",
        m if offset < m => " ▼ more below",
        _ if offset > 0 => " ▲ more above",
        _ => "",
    };
    rows.push(padded(
        t,
        dim(
            t,
            &format!("↑↓ select step · PgUp/PgDn scroll detail{note} · Esc or v to close"),
            inner,
        ),
    ));
    draw_box(frame, t, " TRAJECTORY ", width, rows);
}

fn alert_status(status: AlertStatus) -> (SessionStatus, &'static str) {
    match status {
        AlertStatus::Running => (SessionStatus::Running, "Running"),
        AlertStatus::Waiting => (SessionStatus::Waiting, "Waiting"),
        AlertStatus::Done => (SessionStatus::Done, "Done"),
        AlertStatus::Error => (SessionStatus::Error, "Error"),
    }
}

/// `a`: every status change sdeck has noticed, newest first. Read-only.
pub fn render_alerts(frame: &mut Frame, t: Theme, alerts: &[AlertEntry], scroll: &Cell<usize>, now_ms: i64) {
    let Some((width, inner)) = popup_width(frame, 88) else {
        return;
    };
    let mut content: Vec<Line<'static>> = if alerts.is_empty() {
        vec![dim(
            t,
            "Nothing yet — status changes show up here as they happen.",
            inner,
        )]
    } else {
        alerts
            .iter()
            .map(|a| {
                let (status, verb) = alert_status(a.status);
                let time = humanize_since(a.at, now_ms);
                let project = a.project.as_ref().map(|p| format!(" · {p}")).unwrap_or_default();
                let available = inner.saturating_sub(3 + text_width(&time)).max(1);
                let mut dot = glyph(t, status);
                dot.style = surface(t).patch(dot.style);
                Line::from(vec![
                    dot,
                    Span::styled(" ", surface(t)),
                    Span::styled(
                        fit(&format!("{verb}: {}{project}", a.label), available),
                        text(t, Role::Text),
                    ),
                    Span::styled(format!(" {time}"), text(t, Role::TextDim)),
                ])
            })
            .collect()
    };
    content.push(blank(t, inner));
    content.push(dim(t, "Esc or a to close", inner));

    let max_body = usize::from(frame.area().height).saturating_sub(6).max(3);
    let offset = scroll.get().min(content.len().saturating_sub(max_body));
    scroll.set(offset);
    draw_scrolled(
        frame,
        t,
        " ALERTS ",
        width,
        vec![blank(t, width - 2)],
        content,
        offset,
        max_body,
    );
}

/// A row of the search results: the session and its project's label.
pub struct SearchResultRow {
    pub view: SessionView,
    pub project_label: String,
}

/// An input line, a list below it and a hint line: what the search and palette popups share.
fn draw_list_popup(
    frame: &mut Frame,
    t: Theme,
    title: &str,
    width: usize,
    prompt: String,
    body: Vec<Line<'static>>,
    footer: &str,
) {
    let inner = width - 4;
    let edge = |line: Line<'static>| {
        let mut spans = vec![Span::styled(" ", surface(t))];
        let style = line.style;
        spans.extend(line.spans.into_iter().map(|s| s.patch_style(style)));
        spans.push(Span::styled(" ", surface(t)));
        Line::from(spans)
    };
    let mut rows = vec![
        edge(Line::styled(fit(&prompt, inner), bold(text(t, Role::Accent)))),
        edge(blank(t, inner)),
    ];
    rows.extend(body.into_iter().map(edge));
    rows.push(edge(dim(t, footer, inner)));
    draw_box(frame, t, title, width, rows);
}

/// First visible index so `index` stays centered where the list is longer than `max_items`.
fn first_visible(index: usize, len: usize, max_items: usize) -> usize {
    index
        .saturating_sub(max_items / 2)
        .min(len.saturating_sub(max_items))
}

/// `/`: live-filtered full-text search over every session's prompts and replies.
pub fn render_search(
    frame: &mut Frame,
    t: Theme,
    query: &str,
    loading: bool,
    results: &[SearchResultRow],
    index: usize,
) {
    let width = usize::from(frame.area().width).saturating_sub(4).min(84);
    if width < 8 {
        return;
    }
    let inner = width - 4;
    let max_items = usize::from(frame.area().height).saturating_sub(9).max(1);
    let first = first_visible(index, results.len(), max_items);
    let body: Vec<Line<'static>> = if loading {
        vec![dim(t, " Reading session content…", inner)]
    } else if query.trim().is_empty() {
        vec![dim(
            t,
            " Type to search every session's prompts and replies.",
            inner,
        )]
    } else if results.is_empty() {
        vec![dim(t, " No matches.", inner)]
    } else {
        results
            .iter()
            .enumerate()
            .skip(first)
            .take(max_items)
            .map(|(i, row)| result_line(t, inner, row, i == index))
            .collect()
    };
    draw_list_popup(
        frame,
        t,
        " SEARCH ",
        width,
        format!(" /{query}█"),
        body,
        " ↑↓ select · Enter jump · !@#&~ status filter · Esc cancel",
    );
}

fn result_line(t: Theme, inner: usize, row: &SearchResultRow, is_selected: bool) -> Line<'static> {
    let v = &row.view;
    let mut dot = glyph(t, v.status);
    let glyph_part = format!(" {} ", dot.content);
    let agent_part = format!(" {} ", v.agent);
    let project_part = format!(" {} ", row.project_label);
    let title_width = inner
        .saturating_sub(text_width(&glyph_part) + text_width(&agent_part) + text_width(&project_part))
        .max(1);
    let title = fit(&v.title, title_width);
    if is_selected {
        let sel = selected(t);
        return Line::from(vec![
            Span::styled(glyph_part, sel),
            Span::styled(title, sel),
            Span::styled(project_part, sel),
            Span::styled(agent_part, sel),
        ]);
    }
    dot.style = surface(t).patch(dot.style);
    let agent_role = match v.agent.as_str() {
        "claude" => Role::Orange,
        "copilot" => Role::Accent,
        _ => Role::Text,
    };
    Line::from(vec![
        Span::styled(" ", surface(t)),
        dot,
        Span::styled(" ", surface(t)),
        Span::styled(title, text(t, Role::Text)),
        Span::styled(project_part, text(t, Role::TextDim)),
        Span::styled(agent_part, text(t, agent_role)),
    ])
}

/// `:`: the command palette; `labels` is already the filtered, match-ordered list.
pub fn render_palette(frame: &mut Frame, t: Theme, query: &str, labels: &[&str], index: usize) {
    let width = usize::from(frame.area().width).saturating_sub(4).min(72);
    if width < 8 {
        return;
    }
    let inner = width - 4;
    let max_items = usize::from(frame.area().height).saturating_sub(7).max(1);
    let first = first_visible(index, labels.len(), max_items);
    let body: Vec<Line<'static>> = if query.trim().is_empty() {
        vec![dim(t, " Type to filter commands.", inner)]
    } else if labels.is_empty() {
        vec![dim(t, " No matching command.", inner)]
    } else {
        labels
            .iter()
            .enumerate()
            .skip(first)
            .take(max_items)
            .map(|(i, label)| {
                let style = if i == index {
                    selected(t)
                } else {
                    text(t, Role::Text)
                };
                Line::styled(fit(&format!(" {label}"), inner), style)
            })
            .collect()
    };
    draw_list_popup(
        frame,
        t,
        " COMMANDS ",
        width,
        format!(" :{query}█"),
        body,
        " ↑↓ select · Enter run · Esc cancel",
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn draw(cols: u16, rows: u16, f: impl FnOnce(&mut Frame)) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(cols, rows)).unwrap();
        terminal.draw(f).unwrap();
        let buf = terminal.backend().buffer();
        (0..buf.area.height)
            .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect())
            .collect()
    }

    fn dark() -> Theme {
        Theme::new(crate::theme::ThemeName::Dark)
    }

    #[test]
    fn help_scrolls_and_clamps_its_offset() {
        let scroll = Cell::new(1000);
        let screen = draw(80, 20, |f| {
            render_help(f, dark(), &scroll, "9.9.9", "dark", "Ctrl+Q")
        });
        assert!(scroll.get() > 0 && scroll.get() < 1000);
        assert!(screen.iter().any(|l| l.contains("Esc or ? to close")));
        assert!(screen
            .iter()
            .any(|l| l.contains("▲ more above") || l.contains("Esc or ?")));

        scroll.set(0);
        let screen = draw(80, 20, |f| {
            render_help(f, dark(), &scroll, "9.9.9", "dark", "Ctrl+Q")
        });
        assert!(screen.iter().any(|l| l.contains("KEYBOARD SHORTCUTS")));
        assert!(screen.iter().any(|l| l.contains("▼ more below")));
        assert!(screen.iter().all(|l| !l.contains("Esc or ? to close")));
    }

    #[test]
    fn help_shows_the_version_theme_and_filter_keys() {
        let scroll = Cell::new(0);
        let screen = draw(80, 120, |f| {
            render_help(f, dark(), &scroll, "9.9.9", "system (dark)", "Ctrl+Q")
        });
        let all = screen.join(
            "
",
        );
        assert!(all.contains("sdeck v9.9.9 · theme system (dark)"), "{all}");
        assert!(all.contains("Time: all"));
        assert!(all.contains("Clear filters"));
    }

    #[test]
    fn help_lists_the_account_switch_key() {
        let scroll = Cell::new(0);
        let screen = draw(80, 60, |f| render_help(f, dark(), &scroll, "1", "dark", "Ctrl+Q"));
        assert!(screen
            .iter()
            .any(|l| l.contains("F4") && l.contains("Switch account")));
    }

    #[test]
    fn config_lists_groups_and_shows_the_selected_rows_hint() {
        let config = DeckConfig::default();
        let screen = draw(100, 60, |f| render_config(f, dark(), &config, "C:/cfg.json", 0));
        let all = screen.join("\n");
        for needle in [
            "CONFIG",
            "General",
            "Max sessions listed",
            "Claude",
            "Trash",
            "Accounts",
            "Share projects",
        ] {
            assert!(all.contains(needle), "{needle}\n{all}");
        }
        assert!(all.contains("How many of the most recent sessions"));
        assert!(all.contains("C:/cfg.json"));
    }

    #[test]
    fn skills_explain_themselves_for_a_non_claude_agent() {
        let scroll = Cell::new(0);
        let screen = draw(120, 20, |f| {
            render_skills(f, dark(), SkillsTab::Skills, &[], &[], &[], &scroll, "copilot")
        });
        assert!(screen.iter().any(|l| l.contains("SKILLS & AGENTS · COPILOT")));
        assert!(screen.iter().any(|l| l.contains("are a Claude Code concept")));
    }

    #[test]
    fn usage_tab_shows_each_account_with_bars_and_daily_spend() {
        let scroll = Cell::new(0);
        let accounts = vec![AccountUsage {
            name: "me@x.com".into(),
            active: true,
            five_hour_percent: Some(73.0),
            five_hour_reset_label: Some("8:30 PM".into()),
            seven_day_percent: None,
            seven_day_reset_label: None,
            stale: false,
            days: vec![
                DailySpend {
                    label: "We",
                    percent: None,
                },
                DailySpend {
                    label: "Tu",
                    percent: Some(12.0),
                },
            ],
        }];
        let screen = draw(120, 30, |f| {
            render_skills(
                f,
                dark(),
                SkillsTab::Usage,
                &[],
                &[],
                &accounts,
                &scroll,
                "claude",
            )
        });
        let all = screen.join("\n");
        assert!(all.contains("me@x.com (active)"), "{all}");
        assert!(all.contains("73% ") && all.contains("resets 8:30 PM"), "{all}");
        assert!(all.contains("Tu") && all.contains("12%"), "{all}");
        assert!(all.contains("We") && all.contains("—"), "{all}");
    }

    #[test]
    fn skills_group_by_state_with_counts() {
        let scroll = Cell::new(0);
        let skills = vec![
            LocalSkill {
                name: "a".into(),
                description: "does a".into(),
                state: SkillState::On,
            },
            LocalSkill {
                name: "b".into(),
                description: "does b".into(),
                state: SkillState::Off,
            },
        ];
        let screen = draw(120, 30, |f| {
            render_skills(f, dark(), SkillsTab::Skills, &skills, &[], &[], &scroll, "claude")
        });
        let all = screen.join("\n");
        assert!(all.contains("ON — visible + auto-triggerable (1)"), "{all}");
        assert!(all.contains("OFF — removed entirely, even from / (1)"), "{all}");
        assert!(all.contains("does b"));
    }

    #[test]
    fn trace_shows_steps_and_the_selected_ones_detail() {
        let steps = vec![
            TraceStep {
                kind: TraceKind::User,
                label: "fix it".into(),
                detail: "fix it please".into(),
                is_error: false,
            },
            TraceStep {
                kind: TraceKind::Tool,
                label: "Read(a.rs)".into(),
                detail: "Read\nboom".into(),
                is_error: true,
            },
        ];
        let scroll = Cell::new(0);
        let screen = draw(100, 20, |f| render_trace(f, dark(), &steps, 1, &scroll));
        let all = screen.join("\n");
        assert!(all.contains("You: fix it"), "{all}");
        assert!(all.contains("Tool: ✕ Read(a.rs)"), "{all}");
        assert!(all.contains("boom"), "{all}");

        let empty = draw(100, 20, |f| render_trace(f, dark(), &[], 0, &scroll)).join("\n");
        assert!(empty.contains("No messages or tool calls yet."));
    }

    #[test]
    fn alerts_list_entries_in_the_given_order() {
        let alerts = vec![
            AlertEntry {
                session_id: "1".into(),
                status: AlertStatus::Waiting,
                label: "newer".into(),
                project: Some("proj".into()),
                at: 2_000,
            },
            AlertEntry {
                session_id: "2".into(),
                status: AlertStatus::Done,
                label: "older".into(),
                project: None,
                at: 1_000,
            },
        ];
        let scroll = Cell::new(0);
        let screen = draw(100, 20, |f| render_alerts(f, dark(), &alerts, &scroll, 5_000));
        let newer = screen
            .iter()
            .position(|l| l.contains("Waiting: newer · proj"))
            .unwrap();
        let older = screen.iter().position(|l| l.contains("Done: older")).unwrap();
        assert!(newer < older);
        let none = draw(100, 20, |f| render_alerts(f, dark(), &[], &scroll, 0)).join("\n");
        assert!(none.contains("Nothing yet"));
    }

    #[test]
    fn search_and_palette_show_their_states_inside_an_aligned_box() {
        let screen = draw(100, 20, |f| render_search(f, dark(), "", false, &[], 0));
        assert!(screen
            .iter()
            .any(|l| l.contains("Type to search every session's prompts and replies.")));
        let widths: Vec<usize> = screen
            .iter()
            .filter(|l| l.contains('│') || l.contains('╭') || l.contains('╰'))
            .map(|l| l.trim().chars().count())
            .collect();
        assert!(
            widths.len() > 3 && widths.iter().all(|w| *w == widths[0]),
            "{widths:?}"
        );

        let loading = draw(100, 20, |f| render_search(f, dark(), "x", true, &[], 0)).join("\n");
        assert!(loading.contains("Reading session content…"));
        let none = draw(100, 20, |f| render_search(f, dark(), "x", false, &[], 0)).join("\n");
        assert!(none.contains("No matches."));

        let blank_palette = draw(100, 20, |f| render_palette(f, dark(), "", &[], 0)).join("\n");
        assert!(blank_palette.contains("Type to filter commands."));
        let listed = draw(100, 20, |f| render_palette(f, dark(), "n", &["New session"], 0)).join("\n");
        assert!(listed.contains(":n█") && listed.contains("New session"));
        let missing = draw(100, 20, |f| render_palette(f, dark(), "zz", &[], 0)).join("\n");
        assert!(missing.contains("No matching command."));
    }
}
