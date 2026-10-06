//! The rows of the config popup (`C`), port of `configFields.ts`: how each setting is shown,
//! pre-filled for editing, and applied. Adds the two `accounts.*` rows the TS version lacks.

use std::sync::LazyLock;

use sdeck_core::agent_catalog::all_agent_ids;
use sdeck_core::discovery::path_utils::expand_input_path;
use sdeck_core::status::session_status::SessionStatus;
use sdeck_core::store::deck_config::{
    DeckConfig, PanelLayout, RestoreSessions, ToolConfig, UsagePosition, DEFAULT_DETACH_KEY,
};

use crate::keybindings::{self, display_spec, effective_spec, KeyAction, Target, KEY_ACTIONS};

/// `Toggle` applies immediately on Enter; the others open a text prompt first, pre-filled with
/// [`ConfigField::edit_value`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFieldKind {
    Toggle,
    Number,
    Text,
    Args,
    StatusList,
    /// Enter then press the new key; Backspace restores the default.
    Key,
}

/// The popup's tabs: settings grouped by what they are about, so no tab is long.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigTab {
    General,
    Display,
    Agents,
    Accounts,
    Keybindings,
}

impl ConfigTab {
    pub const ALL: [ConfigTab; 5] = [
        Self::General,
        Self::Display,
        Self::Agents,
        Self::Accounts,
        Self::Keybindings,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Display => "Display",
            Self::Agents => "Agents",
            Self::Accounts => "Accounts",
            Self::Keybindings => "Keybindings",
        }
    }

    pub fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|&t| t == self).unwrap()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Setting {
    MaxSessionsListed,
    Notifications,
    NotifyStatuses,
    RecentProjectsFirst,
    RecentSessionsFirst,
    NewSessionFullScreen,
    PromptProjectFolder,
    GitStatus,
    ExpandCollapsedOnActiveJump,
    ShowUsage,
    UsagePosition,
    Layout,
    CompactUsage,
    Use24HourClock,
    ProjectSearchRoot,
    DetachKey,
    Binding,
    Ctl,
    RestoreSessions,
    ToolEnabled,
    ToolCommand,
    ToolArgs,
    TrashRetentionDays,
    ShareProjects,
    ShowAllSessions,
    ShowOwner,
}

pub struct ConfigField {
    /// `<group>.<name>`, e.g. `tools.claude.command`.
    pub label: String,
    pub tab: ConfigTab,
    /// The section header the row sits under within its tab.
    group: String,
    pub kind: ConfigFieldKind,
    /// One-line explanation shown under the popup while this row is selected.
    pub hint: Option<&'static str>,
    setting: Setting,
    /// The agent id of a `tools.<agent>.*` row.
    agent: String,
    /// The action of a Keybindings row (`None` for the detach key).
    binding: Option<&'static KeyAction>,
}

fn split_args(input: &str) -> Vec<String> {
    input.split_whitespace().map(str::to_string).collect()
}

fn on_off(value: bool) -> String {
    if value { "on" } else { "off" }.to_string()
}

/// A positive whole number, as `Number.isInteger(n) && n > 0` accepts it.
fn positive_int(input: &str) -> Option<u64> {
    let n: f64 = input.trim().parse().ok()?;
    (n.fract() == 0.0 && n > 0.0 && n <= u64::MAX as f64).then_some(n as u64)
}

impl ConfigField {
    fn new(
        label: &str,
        tab: ConfigTab,
        group: &str,
        kind: ConfigFieldKind,
        setting: Setting,
        hint: Option<&'static str>,
    ) -> Self {
        Self {
            label: label.to_string(),
            tab,
            group: group.to_string(),
            kind,
            hint,
            setting,
            agent: String::new(),
            binding: None,
        }
    }

    fn tool(agent: &str, name: &str, kind: ConfigFieldKind, setting: Setting) -> Self {
        Self {
            label: format!("tools.{agent}.{name}"),
            tab: ConfigTab::Agents,
            group: format!("tools.{agent}"),
            kind,
            hint: None,
            setting,
            agent: agent.to_string(),
            binding: None,
        }
    }

    fn binding(action: &'static KeyAction) -> Self {
        Self {
            label: format!("keys.{}.{}", action.group, action.id),
            tab: ConfigTab::Keybindings,
            group: format!("keys.{}", action.group),
            kind: ConfigFieldKind::Key,
            hint: Some(action.hint),
            setting: Setting::Binding,
            agent: String::new(),
            binding: Some(action),
        }
    }

    fn target(&self) -> Option<Target> {
        match self.setting {
            Setting::DetachKey => Some(Target::Detach),
            Setting::Binding => self.binding.map(Target::Action),
            _ => None,
        }
    }

    /// Why a Keybindings row refuses `input`; `None` for any other row or an acceptable key.
    pub fn key_error(&self, config: &DeckConfig, input: &str) -> Option<String> {
        keybindings::validate(config, self.target()?, input).err()
    }

    /// The default key of a Keybindings row, as shown.
    pub fn default_display(&self) -> Option<String> {
        match self.setting {
            Setting::DetachKey => Some(display_spec(DEFAULT_DETACH_KEY)),
            Setting::Binding => self.binding.map(|a| display_spec(a.default)),
            _ => None,
        }
    }

    /// Whether the row sits on a key other than its default.
    pub fn is_customized(&self, c: &DeckConfig) -> bool {
        match self.setting {
            Setting::DetachKey => c.ui.detach_key != DEFAULT_DETACH_KEY,
            Setting::Binding => self.binding.is_some_and(|a| effective_spec(c, a) != a.default),
            _ => false,
        }
    }

    /// `Some(include_files)` when the value is a path the prompt suggests completions for: a folder
    /// for the search root, a file or folder for a tool command.
    pub fn path_completion(&self) -> Option<bool> {
        match self.setting {
            Setting::ProjectSearchRoot => Some(false),
            Setting::ToolCommand => Some(true),
            _ => None,
        }
    }

    /// The part of the label before the last dot: the popup's group header.
    pub fn group(&self) -> &str {
        &self.group
    }

    fn name(&self) -> &str {
        self.label
            .rsplit_once('.')
            .map_or(self.label.as_str(), |(_, name)| name)
    }

    /// The name as words (`recentProjectsFirst` → `Recent projects first`).
    pub fn pretty_name(&self) -> String {
        if self.setting == Setting::Ctl {
            return "Control server".to_string();
        }
        let mut spaced = String::new();
        let chars: Vec<char> = self.name().chars().collect();
        for (i, &c) in chars.iter().enumerate() {
            if let Some(&prev) = i.checked_sub(1).map(|p| &chars[p]) {
                let boundary = (prev.is_ascii_lowercase() && c.is_ascii_uppercase())
                    || (prev.is_ascii_alphabetic() && c.is_ascii_digit())
                    || (prev.is_ascii_digit() && c.is_ascii_uppercase());
                if boundary {
                    spaced.push(' ');
                }
            }
            spaced.push(c.to_ascii_lowercase());
        }
        let mut out = spaced.chars();
        out.next()
            .map(|first| first.to_uppercase().chain(out).collect())
            .unwrap_or_default()
    }

    fn tool_of<'a>(&self, config: &'a DeckConfig) -> Option<&'a ToolConfig> {
        config.tools.get(&self.agent)
    }

    /// Friendly rendering for the popup row, e.g. `(default: claude)` when unset.
    pub fn display(&self, c: &DeckConfig) -> String {
        match self.setting {
            Setting::MaxSessionsListed => c.ui.max_sessions_listed.to_string(),
            Setting::Notifications => on_off(c.ui.notifications),
            Setting::NotifyStatuses => {
                let joined = self.edit_value(c);
                if joined.is_empty() {
                    "(none)".into()
                } else {
                    joined
                }
            }
            Setting::RecentProjectsFirst => on_off(c.ui.recent_projects_first),
            Setting::RecentSessionsFirst => on_off(c.ui.recent_sessions_first),
            Setting::NewSessionFullScreen => if c.ui.new_session_full_screen {
                "Attached"
            } else {
                "Interacting"
            }
            .into(),
            Setting::PromptProjectFolder => on_off(c.ui.prompt_project_folder),
            Setting::GitStatus => on_off(c.ui.git_status),
            Setting::ExpandCollapsedOnActiveJump => on_off(c.ui.expand_collapsed_on_active_jump),
            Setting::ShowUsage => on_off(c.ui.show_usage),
            Setting::CompactUsage => on_off(c.ui.compact_usage),
            Setting::UsagePosition => match c.ui.usage_position {
                UsagePosition::Top => "Top",
                UsagePosition::Bottom => "Bottom",
                UsagePosition::Float => "Float",
            }
            .into(),
            Setting::Layout => match c.ui.layout {
                PanelLayout::Auto => "Auto",
                PanelLayout::Side => "Side",
                PanelLayout::Stacked => "Stacked",
            }
            .into(),
            Setting::Use24HourClock => if c.ui.use_24_hour_clock {
                "24-hour"
            } else {
                "12-hour"
            }
            .into(),
            Setting::ProjectSearchRoot => {
                c.ui.project_search_root
                    .clone()
                    .unwrap_or_else(|| "(none)".into())
            }
            Setting::DetachKey => display_spec(&c.ui.detach_key),
            Setting::Binding => self
                .binding
                .map(|a| display_spec(&effective_spec(c, a)))
                .unwrap_or_default(),
            Setting::Ctl => on_off(c.ui.ctl),
            Setting::RestoreSessions => match c.ui.restore_sessions {
                RestoreSessions::Ask => "Ask",
                RestoreSessions::Always => "Always",
                RestoreSessions::Never => "Never",
            }
            .into(),
            Setting::ToolEnabled => on_off(self.tool_of(c).and_then(|t| t.enabled) != Some(false)),
            Setting::ToolCommand => self
                .tool_of(c)
                .and_then(|t| t.command.clone())
                .unwrap_or_else(|| format!("(default: {})", self.agent)),
            Setting::ToolArgs => {
                let joined = self.edit_value(c);
                if joined.is_empty() {
                    "(none)".into()
                } else {
                    joined
                }
            }
            Setting::TrashRetentionDays => c.trash.retention_days.to_string(),
            Setting::ShareProjects => on_off(c.accounts.share_projects),
            Setting::ShowAllSessions => on_off(c.accounts.show_all_sessions),
            Setting::ShowOwner => on_off(c.accounts.show_owner),
        }
    }

    /// The prompt's starting value when editing: the raw stored value, empty when unset.
    pub fn edit_value(&self, c: &DeckConfig) -> String {
        match self.setting {
            Setting::NotifyStatuses => {
                c.ui.notify_statuses
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            }
            Setting::ProjectSearchRoot => c.ui.project_search_root.clone().unwrap_or_default(),
            Setting::ToolCommand => self
                .tool_of(c)
                .and_then(|t| t.command.clone())
                .unwrap_or_default(),
            Setting::ToolArgs => self
                .tool_of(c)
                .and_then(|t| t.args.as_ref())
                .map(|a| a.join(" "))
                .unwrap_or_default(),
            Setting::NewSessionFullScreen | Setting::Use24HourClock => on_off(match self.setting {
                Setting::NewSessionFullScreen => c.ui.new_session_full_screen,
                _ => c.ui.use_24_hour_clock,
            }),
            _ => self.display(c),
        }
    }

    /// `Toggle` fields ignore `input` and flip themselves. The others parse the prompt's text;
    /// `None` means invalid input, so the caller keeps the config unchanged.
    pub fn apply(&self, config: &DeckConfig, input: &str) -> Option<DeckConfig> {
        let mut c = config.clone();
        match self.setting {
            Setting::MaxSessionsListed => c.ui.max_sessions_listed = positive_int(input)?,
            Setting::Notifications => c.ui.notifications = !c.ui.notifications,
            Setting::NotifyStatuses => {
                c.ui.notify_statuses = split_args(input)
                    .iter()
                    .map(|v| SessionStatus::parse(v))
                    .collect::<Option<Vec<_>>>()?;
            }
            Setting::RecentProjectsFirst => c.ui.recent_projects_first = !c.ui.recent_projects_first,
            Setting::RecentSessionsFirst => c.ui.recent_sessions_first = !c.ui.recent_sessions_first,
            Setting::NewSessionFullScreen => c.ui.new_session_full_screen = !c.ui.new_session_full_screen,
            Setting::PromptProjectFolder => c.ui.prompt_project_folder = !c.ui.prompt_project_folder,
            Setting::GitStatus => c.ui.git_status = !c.ui.git_status,
            Setting::ExpandCollapsedOnActiveJump => {
                c.ui.expand_collapsed_on_active_jump = !c.ui.expand_collapsed_on_active_jump
            }
            Setting::ShowUsage => c.ui.show_usage = !c.ui.show_usage,
            Setting::CompactUsage => c.ui.compact_usage = !c.ui.compact_usage,
            Setting::UsagePosition => c.ui.usage_position = c.ui.usage_position.next(),
            Setting::Layout => c.ui.layout = c.ui.layout.next(),
            Setting::Use24HourClock => c.ui.use_24_hour_clock = !c.ui.use_24_hour_clock,
            Setting::ProjectSearchRoot => {
                let root = input.trim();
                let root = root
                    .strip_prefix('"')
                    .and_then(|r| r.strip_suffix('"'))
                    .unwrap_or(root);
                if !root.is_empty() && !std::path::Path::new(&expand_input_path(root)).is_dir() {
                    return None;
                }
                c.ui.project_search_root = (!root.is_empty()).then(|| root.to_string());
            }
            Setting::DetachKey => {
                c.ui.detach_key = keybindings::validate(config, Target::Detach, input).ok()?
            }
            Setting::Binding => {
                let action = self.binding?;
                let spec = keybindings::validate(config, Target::Action(action), input).ok()?;
                if spec == action.default {
                    c.keybindings.shift_remove(action.id);
                } else {
                    c.keybindings.insert(action.id.to_string(), spec);
                }
            }
            Setting::Ctl => c.ui.ctl = !c.ui.ctl,
            Setting::RestoreSessions => c.ui.restore_sessions = c.ui.restore_sessions.next(),
            Setting::ToolEnabled => {
                let tool = c.tools.entry(self.agent.clone()).or_default();
                tool.enabled = Some(tool.enabled == Some(false));
            }
            Setting::ToolCommand => {
                let command = input.trim();
                c.tools.entry(self.agent.clone()).or_default().command =
                    (!command.is_empty()).then(|| command.to_string());
            }
            Setting::ToolArgs => {
                let args = split_args(input);
                c.tools.entry(self.agent.clone()).or_default().args = (!args.is_empty()).then_some(args);
            }
            Setting::TrashRetentionDays => c.trash.retention_days = positive_int(input)?,
            Setting::ShareProjects => c.accounts.share_projects = !c.accounts.share_projects,
            Setting::ShowAllSessions => c.accounts.show_all_sessions = !c.accounts.show_all_sessions,
            Setting::ShowOwner => c.accounts.show_owner = !c.accounts.show_owner,
        }
        Some(c)
    }
}

fn build_fields() -> Vec<ConfigField> {
    use ConfigFieldKind::*;
    use ConfigTab::*;
    let ui = |name: &str, tab, group: &str, kind, setting, hint| {
        ConfigField::new(&format!("ui.{name}"), tab, group, kind, setting, Some(hint))
    };
    let mut fields = vec![
        ui(
            "maxSessionsListed",
            General,
            "sessions",
            Number,
            Setting::MaxSessionsListed,
            "How many of the most recent sessions load into the list from disk.",
        ),
        ui(
            "newSessionFullScreen",
            General,
            "sessions",
            Toggle,
            Setting::NewSessionFullScreen,
            "Which mode n/N opens a new session in: Attached — full-screen, same as pressing Enter — or Interacting — typed into in place, list and preview still showing, same as pressing i.",
        ),
        ui(
            "restoreSessions",
            General,
            "sessions",
            Toggle,
            Setting::RestoreSessions,
            "Reopens the sessions that were running when sdeck last exited (resumes their conversations) — Enter cycles: Ask (a prompt at startup), Always, Never.",
        ),
        ui(
            "projectSearchRoot",
            General,
            "sessions",
            Text,
            Setting::ProjectSearchRoot,
            "Folder the project prompt (p) opens in, with its subfolders suggested as you type. Empty: no suggestions.",
        ),
        ui(
            "notifications",
            General,
            "notifications",
            Toggle,
            Setting::Notifications,
            "Desktop notifications for sessions that need you — see notifyStatuses below for which ones.",
        ),
        ui(
            "notifyStatuses",
            General,
            "notifications",
            StatusList,
            Setting::NotifyStatuses,
            "Which session statuses trigger a desktop notification (space-separated: running waiting done error).",
        ),
        ui(
            "ctl",
            General,
            "advanced",
            Toggle,
            Setting::Ctl,
            "Lets the sdeck ctl command (list, status, screen, wait) talk to this running sdeck, read-only. Takes effect after a restart.",
        ),
        ConfigField::new(
            "trash.retentionDays",
            General,
            "advanced",
            Number,
            Setting::TrashRetentionDays,
            Some("Deleted sessions older than this are purged from ~/.session-deck/trash/ at startup."),
        ),
        ui(
            "promptProjectFolder",
            General,
            "sessions",
            Toggle,
            Setting::PromptProjectFolder,
            "When p adds a new project and folders exist, asks which folder to move it to (or to leave it at the top level).",
        ),
        ui(
            "layout",
            Display,
            "layout",
            Toggle,
            Setting::Layout,
            "How the panels are arranged — Enter cycles: Auto (side by side from 80 columns, stacked from 50), Side (side by side from 50), Stacked (list above preview from 50). < and > resize the list in either.",
        ),
        ui(
            "recentProjectsFirst",
            Display,
            "list",
            Toggle,
            Setting::RecentProjectsFirst,
            "On: top-level projects reorder by most-recent activity. Off: a fixed, alphabetical order until you move one with K/J.",
        ),
        ui(
            "recentSessionsFirst",
            Display,
            "list",
            Toggle,
            Setting::RecentSessionsFirst,
            "On: sessions in a project reorder by most-recent activity. Off: a fixed order until you move one with K/J.",
        ),
        ui(
            "expandCollapsedOnActiveJump",
            Display,
            "list",
            Toggle,
            Setting::ExpandCollapsedOnActiveJump,
            "On: [ and ] can expand a collapsed folder/project to reach a session inside. Off: they only jump between sessions already shown.",
        ),
        ui(
            "gitStatus",
            Display,
            "list",
            Toggle,
            Setting::GitStatus,
            "Shows ⇡/⇣/✱ git badges on rows and the branch name in the preview panel.",
        ),
        ui(
            "showUsage",
            Display,
            "usage",
            Toggle,
            Setting::ShowUsage,
            "Shows a Context/5h/7d usage section at the bottom of the list for the selected Claude session.",
        ),
        ui(
            "usagePosition",
            Display,
            "usage",
            Toggle,
            Setting::UsagePosition,
            "Where the usage section sits — Enter cycles: Float (right after the last row), Top (pinned above the tree), Bottom (pinned to the bottom of the panel).",
        ),
        ui(
            "compactUsage",
            Display,
            "usage",
            Toggle,
            Setting::CompactUsage,
            "One-line usage summary instead of the Model/Cache/Context/5h/7d rows.",
        ),
        ui(
            "use24HourClock",
            Display,
            "usage",
            Toggle,
            Setting::Use24HourClock,
            "The 5h usage row's reset time shows as 20:30 instead of 8:30 PM.",
        ),
    ];
    for agent in all_agent_ids() {
        fields.push(ConfigField::tool(agent, "enabled", Toggle, Setting::ToolEnabled));
        fields.push(ConfigField::tool(agent, "command", Text, Setting::ToolCommand));
        fields.push(ConfigField::tool(agent, "args", Args, Setting::ToolArgs));
    }
    let account = |name: &str, setting, hint| {
        ConfigField::new(
            &format!("accounts.{name}"),
            Accounts,
            "accounts",
            Toggle,
            setting,
            Some(hint),
        )
    };
    fields.push(account(
        "shareProjects",
        Setting::ShareProjects,
        "On: a project used by several accounts shows as one row. Off: one row per account.",
    ));
    fields.push(account(
        "showAllSessions",
        Setting::ShowAllSessions,
        "On: every logged-in account's sessions are listed. Off: only the active account's (F4 switches it).",
    ));
    fields.push(account(
        "showOwner",
        Setting::ShowOwner,
        "On: another account's session line shows its owner's name. Off: the title is only dimmed.",
    ));
    fields.push(ConfigField {
        label: "keys.chord.detachKey".into(),
        tab: Keybindings,
        group: "keys.chord".into(),
        kind: Key,
        hint: Some(
            "Detaches from a full-screen session, or stops interacting. ctrl+<letter>, not c h i j k m.",
        ),
        setting: Setting::DetachKey,
        agent: String::new(),
        binding: None,
    });
    fields.extend(KEY_ACTIONS.iter().map(ConfigField::binding));
    fields
}

/// Indexes into [`CONFIG_FIELDS`] of the rows on `tab`, in display order.
pub fn tab_fields(tab: ConfigTab) -> Vec<usize> {
    CONFIG_FIELDS
        .iter()
        .enumerate()
        .filter(|(_, f)| f.tab == tab)
        .map(|(i, _)| i)
        .collect()
}

pub static CONFIG_FIELDS: LazyLock<Vec<ConfigField>> = LazyLock::new(build_fields);

#[cfg(test)]
mod tests {
    use super::*;
    use sdeck_core::store::deck_config::parse_deck_config;

    fn field(label: &str) -> &'static ConfigField {
        CONFIG_FIELDS
            .iter()
            .find(|f| f.label == label)
            .expect("field exists")
    }

    fn base() -> DeckConfig {
        parse_deck_config("{}")
    }

    #[test]
    fn max_sessions_listed_accepts_a_positive_integer_and_rejects_everything_else() {
        let f = field("ui.maxSessionsListed");
        assert_eq!(f.apply(&base(), "10").unwrap().ui.max_sessions_listed, 10);
        for bad in ["0", "-5", "3.5", "abc"] {
            assert!(f.apply(&base(), bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn notifications_toggles_regardless_of_input() {
        let f = field("ui.notifications");
        let off = f.apply(&base(), "").unwrap();
        assert!(!off.ui.notifications);
        assert!(f.apply(&off, "").unwrap().ui.notifications);
    }

    #[test]
    fn ctl_toggles_and_says_it_needs_a_restart() {
        let f = field("ui.ctl");
        let off = f.apply(&base(), "").unwrap();
        assert!(!off.ui.ctl);
        assert_eq!(f.display(&off), "off");
        assert!(f.apply(&off, "").unwrap().ui.ctl);
        assert!(f.hint.unwrap().contains("restart"));
    }

    #[test]
    fn tool_command_trims_and_clears_back_to_the_default_on_blank_input() {
        let f = field("tools.claude.command");
        let set = f.apply(&base(), "  claude-nightly  ").unwrap();
        assert_eq!(set.tools["claude"].command.as_deref(), Some("claude-nightly"));
        assert_eq!(f.apply(&base(), "   ").unwrap().tools["claude"].command, None);
        assert_eq!(f.display(&base()), "(default: claude)");
        assert_eq!(f.display(&set), "claude-nightly");
    }

    #[test]
    fn tool_args_split_on_whitespace_and_clear_on_blank_input() {
        let f = field("tools.claude.args");
        let set = f.apply(&base(), "--model  opus").unwrap();
        assert_eq!(
            set.tools["claude"].args,
            Some(vec!["--model".into(), "opus".into()])
        );
        assert_eq!(f.apply(&base(), "   ").unwrap().tools["claude"].args, None);
        assert_eq!(f.display(&base()), "(none)");
        assert_eq!(
            f.display(&f.apply(&base(), "--allow-all").unwrap()),
            "--allow-all"
        );
    }

    #[test]
    fn edit_value_is_the_raw_stored_value_not_the_friendly_default() {
        let f = field("tools.copilot.command");
        assert_eq!(f.edit_value(&base()), "");
        assert_eq!(f.display(&base()), "(default: copilot)");
    }

    #[test]
    fn notify_statuses_takes_known_statuses_and_rejects_an_unknown_one() {
        let f = field("ui.notifyStatuses");
        let set = f.apply(&base(), "waiting error").unwrap();
        assert_eq!(
            set.ui.notify_statuses,
            [SessionStatus::Waiting, SessionStatus::Error]
        );
        assert!(f.apply(&base(), "waiting bogus").is_none());
        assert_eq!(f.display(&base()), "waiting done error");
        assert_eq!(f.display(&f.apply(&base(), "").unwrap()), "(none)");
    }

    #[test]
    fn toggles_flip_from_their_defaults() {
        let cases: [(&str, &str, &str); 6] = [
            ("ui.recentProjectsFirst", "off", "on"),
            ("ui.recentSessionsFirst", "on", "off"),
            ("ui.gitStatus", "on", "off"),
            ("ui.expandCollapsedOnActiveJump", "on", "off"),
            ("ui.compactUsage", "off", "on"),
            ("accounts.shareProjects", "on", "off"),
        ];
        for (label, before, after) in cases {
            let f = field(label);
            assert_eq!(f.display(&base()), before, "{label}");
            let flipped = f.apply(&base(), "").unwrap();
            assert_eq!(f.display(&flipped), after, "{label}");
            assert_eq!(f.display(&f.apply(&flipped, "").unwrap()), before, "{label}");
        }
    }

    #[test]
    fn usage_position_cycles_float_top_bottom() {
        let f = field("ui.usagePosition");
        assert_eq!(f.display(&base()), "Float");
        let top = f.apply(&base(), "").unwrap();
        assert_eq!(f.display(&top), "Top");
        let bottom = f.apply(&top, "").unwrap();
        assert_eq!(f.display(&bottom), "Bottom");
        assert_eq!(f.display(&f.apply(&bottom, "").unwrap()), "Float");
    }

    #[test]
    fn new_session_mode_shows_attached_or_interacting() {
        let f = field("ui.newSessionFullScreen");
        assert_eq!(f.display(&base()), "Attached");
        let flipped = f.apply(&base(), "").unwrap();
        assert!(!flipped.ui.new_session_full_screen);
        assert_eq!(f.display(&flipped), "Interacting");
    }

    #[test]
    fn show_all_sessions_toggles_the_accounts_setting() {
        let f = field("accounts.showAllSessions");
        assert!(!f.apply(&base(), "").unwrap().accounts.show_all_sessions);
    }

    #[test]
    fn tool_enabled_toggles_per_agent() {
        let claude = field("tools.claude.enabled");
        let copilot = field("tools.copilot.enabled");
        assert_eq!(claude.display(&base()), "on");
        let off = claude.apply(&base(), "").unwrap();
        assert_eq!(off.tools["claude"].enabled, Some(false));
        assert_eq!(claude.display(&off), "off");
        assert_eq!(copilot.display(&off), "on");
        assert_eq!(claude.display(&claude.apply(&off, "").unwrap()), "on");
    }

    #[test]
    fn every_catalog_agent_gets_tool_rows() {
        let f = field("tools.codex.enabled");
        assert_eq!(f.display(&base()), "on");
        assert_eq!(f.apply(&base(), "").unwrap().tools["codex"].enabled, Some(false));
    }

    #[test]
    fn trash_retention_days_accepts_a_positive_integer_and_rejects_everything_else() {
        let f = field("trash.retentionDays");
        assert_eq!(f.apply(&base(), "7").unwrap().trash.retention_days, 7);
        assert!(f.apply(&base(), "0").is_none());
        assert_eq!(f.display(&base()), "30");
    }

    #[test]
    fn names_are_shown_as_words() {
        assert_eq!(
            field("ui.expandCollapsedOnActiveJump").pretty_name(),
            "Expand collapsed on active jump"
        );
        assert_eq!(field("ui.use24HourClock").pretty_name(), "Use 24 hour clock");
        assert_eq!(field("tools.claude.command").pretty_name(), "Command");
        assert_eq!(field("tools.claude.command").group(), "tools.claude");
    }
}
