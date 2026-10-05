use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use serde_json::{Map, Value};

use super::atomic::write_atomic;
use crate::agent_catalog::all_agent_ids;
use crate::paths::deck_home;
use crate::status::session_status::SessionStatus;

/// Overrides for one agent's launch command.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToolConfig {
    /// `false` skips discovering and starting this agent's sessions entirely.
    pub enabled: Option<bool>,
    /// Replaces the executable sdeck spawns (a bare name resolved on PATH, or a full path).
    pub command: Option<String>,
    /// Extra arguments appended after the ones sdeck builds itself.
    pub args: Option<Vec<String>>,
    /// Fields this version doesn't know, kept so a write never drops them.
    pub extra: Map<String, Value>,
}

/// Where the USAGE section sits in the list panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsagePosition {
    /// Pinned above the tree.
    Top,
    /// Pinned to the bottom of the panel.
    Bottom,
    /// Right after the last row, so it follows the tree's size.
    Float,
}

impl UsagePosition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Bottom => "bottom",
            Self::Float => "float",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "top" => Some(Self::Top),
            "bottom" => Some(Self::Bottom),
            "float" => Some(Self::Float),
            _ => None,
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Float => Self::Top,
            Self::Top => Self::Bottom,
            Self::Bottom => Self::Float,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct UiConfig {
    /// How many of the most recent sessions of each project are loaded from disk. Default: 10.
    pub max_sessions_listed: u64,
    /// Desktop notifications for waiting/finished/error sessions. Default: on.
    pub notifications: bool,
    /// Which statuses `notifications` fires for. Default: waiting, done, error.
    pub notify_statuses: Vec<SessionStatus>,
    /// Unordered top-level projects sort by recent activity (true) or alphabetically (false). Default: false.
    pub recent_projects_first: bool,
    /// Sessions inside a project sort by recent activity (true) or keep a fixed order (false). Default: true.
    pub recent_sessions_first: bool,
    /// `n`/`N` opens Attached (true) or Interacting (false). Default: true.
    pub new_session_full_screen: bool,
    /// Git dirty/ahead/behind markers on rows and in the preview. Default: true.
    pub git_status: bool,
    /// `[`/`]` expand a collapsed folder or project to reach a started session. Default: true.
    pub expand_collapsed_on_active_jump: bool,
    /// Context / 5h / 7d usage section at the bottom of the list. Default: true.
    pub show_usage: bool,
    /// Where the usage section sits: top, bottom or float. Default: float.
    pub usage_position: UsagePosition,
    /// One-line usage summary instead of the Model/Cache/Context/5h/7d rows. Default: false.
    pub compact_usage: bool,
    /// The 5h reset time shows as `20:30` (true) or `8:30 PM` (false). Default: false.
    pub use_24_hour_clock: bool,
    /// Config dir of the account new sessions launch as; unset = the default account.
    pub active_account_config_dir: Option<String>,
    /// Key that detaches from a full attach or stops interacting, as `ctrl+<letter>`. Default: `ctrl+q`.
    pub detach_key: String,
    /// The `sdeck ctl` control server (read-only). Default: on. Read at startup only.
    pub ctl: bool,
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrashConfig {
    /// Deleted sessions older than this are purged from the trash at startup. Default: 30.
    pub retention_days: u64,
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AccountsConfig {
    /// A project used by several accounts shows as one row (true) or one row per account (false). Default: true.
    pub share_projects: bool,
    /// Show every account's sessions (true) or only the active account's (false). Default: true.
    pub show_all_sessions: bool,
    /// Show the owning account's name on the session line of another account's session. Default: false.
    pub show_owner: bool,
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeckConfig {
    pub ui: UiConfig,
    /// Keyed by every id `all_agent_ids()` lists, plus any other id found in the file.
    pub tools: IndexMap<String, ToolConfig>,
    pub trash: TrashConfig,
    pub accounts: AccountsConfig,
    /// Shell command lines run on a status change, keyed `onWaiting | onDone | onError | onRunning`.
    /// An empty line is off; other keys are kept so a write never drops them.
    pub hooks: IndexMap<String, String>,
    pub extra: Map<String, Value>,
}

const DEFAULT_MAX_SESSIONS_LISTED: u64 = 10;
const DEFAULT_NOTIFY_STATUSES: [SessionStatus; 3] =
    [SessionStatus::Waiting, SessionStatus::Done, SessionStatus::Error];
const DEFAULT_TRASH_RETENTION_DAYS: u64 = 30;
pub const DEFAULT_DETACH_KEY: &str = "ctrl+q";

/// The letter of a `ctrl+<a-z>` detach key (case-insensitive). `c h i j m` are control bytes with
/// other meanings (Ctrl+C, Backspace, Tab, Enter) and `k` is the chord key, so they are refused.
pub fn parse_detach_letter(s: &str) -> Option<char> {
    let lower = s.trim().to_ascii_lowercase();
    let letter = lower.strip_prefix("ctrl+")?;
    let mut chars = letter.chars();
    let c = chars.next()?;
    if chars.next().is_some() || !c.is_ascii_lowercase() || "chijmk".contains(c) {
        return None;
    }
    Some(c)
}

impl UiConfig {
    /// The configured detach letter; `q` if the stored value is not valid.
    pub fn detach_letter(&self) -> char {
        parse_detach_letter(&self.detach_key).unwrap_or('q')
    }
}

/// Whether `main` starts the control server.
pub fn ctl_enabled(config: &DeckConfig) -> bool {
    config.ui.ctl
}

impl Default for DeckConfig {
    fn default() -> Self {
        DeckConfig {
            ui: UiConfig {
                max_sessions_listed: DEFAULT_MAX_SESSIONS_LISTED,
                notifications: true,
                notify_statuses: DEFAULT_NOTIFY_STATUSES.to_vec(),
                recent_projects_first: false,
                recent_sessions_first: true,
                new_session_full_screen: true,
                git_status: true,
                expand_collapsed_on_active_jump: true,
                show_usage: false,
                usage_position: UsagePosition::Float,
                compact_usage: false,
                use_24_hour_clock: false,
                active_account_config_dir: None,
                detach_key: DEFAULT_DETACH_KEY.to_string(),
                ctl: true,
                extra: Map::new(),
            },
            tools: all_agent_ids()
                .into_iter()
                .map(|id| (id.to_string(), ToolConfig::default()))
                .collect(),
            trash: TrashConfig {
                retention_days: DEFAULT_TRASH_RETENTION_DAYS,
                extra: Map::new(),
            },
            accounts: AccountsConfig {
                share_projects: true,
                show_all_sessions: true,
                show_owner: false,
                extra: Map::new(),
            },
            hooks: IndexMap::new(),
            extra: Map::new(),
        }
    }
}

/// `~/.session-deck/config.json` — editable by hand, or from the config popup.
pub fn deck_config_path() -> PathBuf {
    deck_home().join("config.json")
}

pub(crate) fn extras(obj: &Map<String, Value>, known: &[&str]) -> Map<String, Value> {
    obj.iter()
        .filter(|(k, _)| !known.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// A JSON number that is a whole number greater than zero (`30` and `30.0` both count, as in JS).
fn positive_int(v: &Value) -> Option<u64> {
    let n = v.as_f64()?;
    (n.fract() == 0.0 && n > 0.0 && n <= u64::MAX as f64).then_some(n as u64)
}

fn parse_tool_config(raw: Option<&Value>) -> ToolConfig {
    let Some(obj) = raw.and_then(Value::as_object) else {
        return ToolConfig::default();
    };
    let mut config = ToolConfig {
        extra: extras(obj, &["enabled", "command", "args"]),
        ..ToolConfig::default()
    };
    if let Some(b) = obj.get("enabled").and_then(Value::as_bool) {
        config.enabled = Some(b);
    }
    if let Some(s) = obj.get("command").and_then(Value::as_str) {
        if !s.trim().is_empty() {
            config.command = Some(s.trim().to_string());
        }
    }
    if let Some(items) = obj.get("args").and_then(Value::as_array) {
        let args: Vec<String> = items
            .iter()
            .filter_map(|a| a.as_str().map(str::to_string))
            .collect();
        if args.len() == items.len() {
            config.args = Some(args);
        }
    }
    config
}

/// Tolerant: a missing, malformed or partial file behaves like an empty one — every setting falls back to its default.
pub fn parse_deck_config(raw: &str) -> DeckConfig {
    let mut config = DeckConfig::default();
    let Ok(Value::Object(root)) = serde_json::from_str::<Value>(raw) else {
        return config;
    };
    config.extra = extras(&root, &["ui", "tools", "trash", "accounts", "hooks"]);

    if let Some(ui) = root.get("ui").and_then(Value::as_object) {
        let c = &mut config.ui;
        c.extra = extras(
            ui,
            &[
                "maxSessionsListed",
                "notifications",
                "notifyStatuses",
                "recentProjectsFirst",
                "recentSessionsFirst",
                "newSessionFullScreen",
                "gitStatus",
                "expandCollapsedOnActiveJump",
                "showUsage",
                "usagePosition",
                "compactUsage",
                "use24HourClock",
                "activeAccountConfigDir",
                "detachKey",
                "ctl",
            ],
        );
        if let Some(n) = ui.get("maxSessionsListed").and_then(positive_int) {
            c.max_sessions_listed = n;
        }
        let flag = |key: &str, slot: &mut bool| {
            if let Some(b) = ui.get(key).and_then(Value::as_bool) {
                *slot = b;
            }
        };
        flag("notifications", &mut c.notifications);
        flag("recentProjectsFirst", &mut c.recent_projects_first);
        flag("recentSessionsFirst", &mut c.recent_sessions_first);
        flag("newSessionFullScreen", &mut c.new_session_full_screen);
        flag("gitStatus", &mut c.git_status);
        flag(
            "expandCollapsedOnActiveJump",
            &mut c.expand_collapsed_on_active_jump,
        );
        flag("showUsage", &mut c.show_usage);
        flag("compactUsage", &mut c.compact_usage);
        flag("use24HourClock", &mut c.use_24_hour_clock);
        flag("ctl", &mut c.ctl);
        if let Some(p) = ui
            .get("usagePosition")
            .and_then(Value::as_str)
            .and_then(UsagePosition::parse)
        {
            c.usage_position = p;
        }
        if let Some(items) = ui.get("notifyStatuses").and_then(Value::as_array) {
            let parsed: Vec<SessionStatus> = items
                .iter()
                .filter_map(|s| s.as_str().and_then(SessionStatus::parse))
                .collect();
            if parsed.len() == items.len() {
                c.notify_statuses = parsed;
            }
        }
        if let Some(l) = ui
            .get("detachKey")
            .and_then(Value::as_str)
            .and_then(parse_detach_letter)
        {
            c.detach_key = format!("ctrl+{l}");
        }
        if let Some(s) = ui.get("activeAccountConfigDir").and_then(Value::as_str) {
            if !s.trim().is_empty() {
                c.active_account_config_dir = Some(s.to_string());
            }
        }
    }

    if let Some(raw_tools) = root.get("tools").and_then(Value::as_object) {
        // Known ids keep their `{}` default when absent; ids only in the file survive too.
        let mut ids: Vec<String> = config.tools.keys().cloned().collect();
        ids.extend(
            raw_tools
                .keys()
                .filter(|k| !config.tools.contains_key(*k))
                .cloned(),
        );
        for id in ids {
            let parsed = parse_tool_config(raw_tools.get(&id));
            config.tools.insert(id, parsed);
        }
    }

    if let Some(trash) = root.get("trash").and_then(Value::as_object) {
        config.trash.extra = extras(trash, &["retentionDays"]);
        if let Some(n) = trash.get("retentionDays").and_then(positive_int) {
            config.trash.retention_days = n;
        }
    }

    if let Some(accounts) = root.get("accounts").and_then(Value::as_object) {
        config.accounts.extra = extras(accounts, &["shareProjects", "showAllSessions", "showOwner"]);
        if let Some(b) = accounts.get("shareProjects").and_then(Value::as_bool) {
            config.accounts.share_projects = b;
        }
        if let Some(b) = accounts.get("showAllSessions").and_then(Value::as_bool) {
            config.accounts.show_all_sessions = b;
        }
        if let Some(b) = accounts.get("showOwner").and_then(Value::as_bool) {
            config.accounts.show_owner = b;
        }
    }
    if let Some(hooks) = root.get("hooks").and_then(Value::as_object) {
        config.hooks = hooks
            .iter()
            .filter_map(|(key, line)| {
                let line = line.as_str()?.trim();
                (!line.is_empty()).then(|| (key.clone(), line.to_string()))
            })
            .collect();
    }
    config
}

fn tool_to_value(tool: &ToolConfig) -> Value {
    let mut obj = Map::new();
    if let Some(b) = tool.enabled {
        obj.insert("enabled".into(), Value::Bool(b));
    }
    if let Some(c) = &tool.command {
        obj.insert("command".into(), Value::String(c.clone()));
    }
    if let Some(a) = &tool.args {
        obj.insert("args".into(), a.iter().cloned().map(Value::String).collect());
    }
    obj.extend(tool.extra.clone());
    Value::Object(obj)
}

/// Every field's resolved value, defaults included, so the file always shows what's in effect.
pub fn deck_config_to_json(config: &DeckConfig) -> String {
    let ui = &config.ui;
    let mut ui_obj = Map::new();
    ui_obj.insert("maxSessionsListed".into(), ui.max_sessions_listed.into());
    ui_obj.insert("notifications".into(), ui.notifications.into());
    ui_obj.insert(
        "notifyStatuses".into(),
        ui.notify_statuses
            .iter()
            .map(|s| Value::from(s.as_str()))
            .collect(),
    );
    ui_obj.insert("recentProjectsFirst".into(), ui.recent_projects_first.into());
    ui_obj.insert("recentSessionsFirst".into(), ui.recent_sessions_first.into());
    ui_obj.insert("newSessionFullScreen".into(), ui.new_session_full_screen.into());
    ui_obj.insert("gitStatus".into(), ui.git_status.into());
    ui_obj.insert(
        "expandCollapsedOnActiveJump".into(),
        ui.expand_collapsed_on_active_jump.into(),
    );
    ui_obj.insert("showUsage".into(), ui.show_usage.into());
    ui_obj.insert("usagePosition".into(), ui.usage_position.as_str().into());
    ui_obj.insert("compactUsage".into(), ui.compact_usage.into());
    ui_obj.insert("use24HourClock".into(), ui.use_24_hour_clock.into());
    ui_obj.insert("detachKey".into(), ui.detach_key.clone().into());
    ui_obj.insert("ctl".into(), ui.ctl.into());
    if let Some(dir) = &ui.active_account_config_dir {
        ui_obj.insert("activeAccountConfigDir".into(), dir.clone().into());
    }
    ui_obj.extend(ui.extra.clone());

    let tools: Map<String, Value> = config
        .tools
        .iter()
        .map(|(id, t)| (id.clone(), tool_to_value(t)))
        .collect();

    let mut trash = Map::new();
    trash.insert("retentionDays".into(), config.trash.retention_days.into());
    trash.extend(config.trash.extra.clone());

    let mut accounts = Map::new();
    accounts.insert("shareProjects".into(), config.accounts.share_projects.into());
    accounts.insert("showAllSessions".into(), config.accounts.show_all_sessions.into());
    accounts.insert("showOwner".into(), config.accounts.show_owner.into());
    accounts.extend(config.accounts.extra.clone());

    let hooks: Map<String, Value> = config
        .hooks
        .iter()
        .map(|(k, v)| (k.clone(), Value::from(v.as_str())))
        .collect();

    let mut root = Map::new();
    root.insert("ui".into(), Value::Object(ui_obj));
    root.insert("tools".into(), Value::Object(tools));
    root.insert("trash".into(), Value::Object(trash));
    root.insert("accounts".into(), Value::Object(accounts));
    if !hooks.is_empty() {
        root.insert("hooks".into(), Value::Object(hooks));
    }
    root.extend(config.extra.clone());
    format!(
        "{}\n",
        serde_json::to_string_pretty(&Value::Object(root)).expect("config serializes")
    )
}

/// Reads and parses the config file. A missing file returns the defaults, same as an empty one.
pub fn read_deck_config(path: &Path) -> DeckConfig {
    match std::fs::read_to_string(path) {
        Ok(raw) => parse_deck_config(&raw),
        Err(_) => DeckConfig::default(),
    }
}

pub fn write_deck_config(config: &DeckConfig, path: &Path) -> std::io::Result<()> {
    write_atomic(path, &deck_config_to_json(config))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(v: Value) -> DeckConfig {
        parse_deck_config(&v.to_string())
    }

    #[test]
    fn returns_every_default_when_the_file_is_empty() {
        let c = parse_deck_config("{}");
        assert_eq!(c.ui.max_sessions_listed, 10);
        assert!(c.ui.notifications);
        assert_eq!(
            c.ui.notify_statuses,
            vec![SessionStatus::Waiting, SessionStatus::Done, SessionStatus::Error]
        );
        assert!(!c.ui.recent_projects_first);
        assert!(c.ui.recent_sessions_first);
        assert!(c.ui.new_session_full_screen);
        assert!(c.ui.git_status);
        assert!(c.ui.expand_collapsed_on_active_jump);
        assert!(!c.ui.show_usage);
        assert_eq!(c.ui.usage_position, UsagePosition::Float);
        assert!(!c.ui.compact_usage);
        assert!(!c.ui.use_24_hour_clock);
        assert_eq!(c.tools["claude"], ToolConfig::default());
        assert_eq!(c.tools["copilot"], ToolConfig::default());
        assert_eq!(c.trash.retention_days, 30);
    }

    #[test]
    fn new_account_fields_default_and_round_trip() {
        let c = parse_deck_config("{}");
        assert!(c.accounts.share_projects);
        assert!(c.accounts.show_all_sessions);
        assert_eq!(c.ui.active_account_config_dir, None);

        let c = parse(json!({
            "accounts": { "shareProjects": false, "showAllSessions": false },
            "ui": { "activeAccountConfigDir": "C:\\Users\\me\\.claude-work" }
        }));
        assert!(!c.accounts.share_projects);
        assert!(!c.accounts.show_all_sessions);
        assert_eq!(
            c.ui.active_account_config_dir.as_deref(),
            Some("C:\\Users\\me\\.claude-work")
        );
        assert_eq!(parse_deck_config(&deck_config_to_json(&c)), c);
    }

    #[test]
    fn ctl_defaults_on_parses_false_and_round_trips() {
        assert!(ctl_enabled(&parse_deck_config("{}")));
        let off = parse(json!({ "ui": { "ctl": false } }));
        assert!(!ctl_enabled(&off));
        assert_eq!(parse_deck_config(&deck_config_to_json(&off)), off);
        assert!(ctl_enabled(&parse(json!({ "ui": { "ctl": "no" } }))));
    }

    #[test]
    fn detach_key_parses_ctrl_letters_and_falls_back_for_the_rest() {
        assert_eq!(parse_deck_config("{}").ui.detach_key, "ctrl+q");
        for (raw, want) in [("ctrl+e", "ctrl+e"), ("Ctrl+E", "ctrl+e")] {
            let c = parse(json!({ "ui": { "detachKey": raw } }));
            assert_eq!(c.ui.detach_key, want);
            assert_eq!(c.ui.detach_letter(), 'e');
            assert_eq!(parse_deck_config(&deck_config_to_json(&c)), c);
        }
        for raw in ["ctrl+c", "ctrl+k", "alt+q", "", "ctrl+qq", "ctrl+1"] {
            let c = parse(json!({ "ui": { "detachKey": raw } }));
            assert_eq!(c.ui.detach_key, "ctrl+q", "{raw}");
        }
        assert_eq!(parse_detach_letter("ctrl+h"), None);
        assert_eq!(parse_detach_letter("ctrl+m"), None);
    }

    #[test]
    fn hooks_parse_trimmed_skip_empty_keep_unknown_and_round_trip() {
        assert!(parse_deck_config("{}").hooks.is_empty());
        let c = parse(json!({ "hooks": {
            "onWaiting": "  echo hi  ", "onDone": "", "onError": 5, "onBogus": "x", "onRunning": "run"
        } }));
        assert_eq!(c.hooks.len(), 3);
        assert_eq!(c.hooks["onBogus"], "x");
        assert_eq!(c.hooks["onWaiting"], "echo hi");
        assert_eq!(c.hooks["onRunning"], "run");
        assert_eq!(parse_deck_config(&deck_config_to_json(&c)), c);
    }

    #[test]
    fn boolean_ui_flags_accept_booleans_only() {
        let c = parse(
            json!({ "ui": { "recentProjectsFirst": true, "recentSessionsFirst": false,
            "newSessionFullScreen": false, "gitStatus": false, "expandCollapsedOnActiveJump": false } }),
        );
        assert!(c.ui.recent_projects_first);
        assert!(!c.ui.recent_sessions_first);
        assert!(!c.ui.new_session_full_screen);
        assert!(!c.ui.git_status);
        assert!(!c.ui.expand_collapsed_on_active_jump);

        let c = parse(
            json!({ "ui": { "recentProjectsFirst": "true", "recentSessionsFirst": "false",
            "newSessionFullScreen": "false", "gitStatus": "false", "expandCollapsedOnActiveJump": "false" } }),
        );
        assert!(!c.ui.recent_projects_first);
        assert!(c.ui.recent_sessions_first);
        assert!(c.ui.new_session_full_screen);
        assert!(c.ui.git_status);
        assert!(c.ui.expand_collapsed_on_active_jump);
    }

    #[test]
    fn usage_position_accepts_known_values_and_falls_back_to_float() {
        let c = parse(json!({ "ui": { "usagePosition": "Bottom" } }));
        assert_eq!(c.ui.usage_position, UsagePosition::Bottom);
        assert_eq!(parse_deck_config(&deck_config_to_json(&c)), c);
        let c = parse(json!({ "ui": { "usagePosition": "middle" } }));
        assert_eq!(c.ui.usage_position, UsagePosition::Float);
    }

    #[test]
    fn compact_usage_round_trips() {
        let c = parse(json!({ "ui": { "compactUsage": true } }));
        assert!(c.ui.compact_usage);
        assert_eq!(parse_deck_config(&deck_config_to_json(&c)), c);
    }

    #[test]
    fn notify_statuses_accepts_valid_and_rejects_an_unknown_status_wholesale() {
        let c = parse(json!({ "ui": { "notifyStatuses": ["waiting"] } }));
        assert_eq!(c.ui.notify_statuses, vec![SessionStatus::Waiting]);
        let c = parse(json!({ "ui": { "notifyStatuses": ["waiting", "bogus"] } }));
        assert_eq!(
            c.ui.notify_statuses,
            vec![SessionStatus::Waiting, SessionStatus::Done, SessionStatus::Error]
        );
    }

    #[test]
    fn accepts_tools_enabled_and_trash_retention_days() {
        let c =
            parse(json!({ "tools": { "copilot": { "enabled": false } }, "trash": { "retentionDays": 7 } }));
        assert_eq!(c.tools["copilot"].enabled, Some(false));
        assert_eq!(c.tools["claude"].enabled, None);
        assert_eq!(c.trash.retention_days, 7);
    }

    #[test]
    fn non_positive_retention_days_falls_back_to_the_default() {
        assert_eq!(
            parse(json!({ "trash": { "retentionDays": 0 } }))
                .trash
                .retention_days,
            30
        );
    }

    #[test]
    fn malformed_json_falls_back_to_defaults() {
        let c = parse_deck_config("{ not json");
        assert_eq!(c.ui.max_sessions_listed, 10);
        assert!(c.ui.notifications);
    }

    #[test]
    fn keeps_only_valid_fields_dropping_the_rest_to_their_defaults() {
        let c = parse(json!({
            "ui": { "maxSessionsListed": 0, "notifications": false },
            "tools": { "claude": { "command": "  claude-nightly  ", "args": ["--model", "opus", 42] } }
        }));
        assert_eq!(c.ui.max_sessions_listed, 10);
        assert!(!c.ui.notifications);
        assert_eq!(c.tools["claude"].command.as_deref(), Some("claude-nightly"));
        assert_eq!(c.tools["claude"].args, None);
        assert_eq!(c.tools["copilot"], ToolConfig::default());
    }

    #[test]
    fn accepts_a_valid_tools_args_array() {
        let c = parse(json!({ "tools": { "copilot": { "args": ["--allow-all"] } } }));
        assert_eq!(c.tools["copilot"].args, Some(vec!["--allow-all".to_string()]));
    }

    #[test]
    fn defaults_every_catalog_agent_and_accepts_settings_for_one() {
        assert_eq!(parse_deck_config("{}").tools["codex"], ToolConfig::default());
        let c = parse(json!({ "tools": { "codex": { "enabled": false, "command": "codex-beta" } } }));
        assert_eq!(c.tools["codex"].enabled, Some(false));
        assert_eq!(c.tools["codex"].command.as_deref(), Some("codex-beta"));
        assert_eq!(c.tools["claude"], ToolConfig::default());
    }

    #[test]
    fn preserves_settings_for_an_agent_id_not_in_the_catalog() {
        let c = parse(json!({ "tools": { "some-future-agent": { "command": "whatever" } } }));
        assert_eq!(c.tools["some-future-agent"].command.as_deref(), Some("whatever"));
    }

    #[test]
    fn unknown_fields_survive_a_rewrite() {
        let c = parse(json!({
            "future": { "a": 1 },
            "ui": { "futureFlag": true },
            "tools": { "claude": { "model": "opus" } },
            "trash": { "x": 2 },
            "accounts": { "y": 3 }
        }));
        let out: Value = serde_json::from_str(&deck_config_to_json(&c)).unwrap();
        assert_eq!(out["future"], json!({ "a": 1 }));
        assert_eq!(out["ui"]["futureFlag"], json!(true));
        assert_eq!(out["tools"]["claude"]["model"], json!("opus"));
        assert_eq!(out["trash"]["x"], json!(2));
        assert_eq!(out["accounts"]["y"], json!(3));
    }

    #[test]
    fn read_returns_defaults_for_a_missing_file_and_parses_a_real_one() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.json");
        assert_eq!(read_deck_config(&missing).ui.max_sessions_listed, 10);
        let file = dir.path().join("config.json");
        std::fs::write(&file, r#"{"ui":{"maxSessionsListed":50}}"#).unwrap();
        assert_eq!(read_deck_config(&file).ui.max_sessions_listed, 50);
    }

    #[test]
    fn write_round_trips_through_read_creating_the_directory() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("nested").join("config.json");
        let mut config = read_deck_config(&file);
        config.ui.max_sessions_listed = 75;
        config.ui.notifications = false;
        config.tools.get_mut("claude").unwrap().command = Some("claude-nightly".into());
        write_deck_config(&config, &file).unwrap();
        assert_eq!(read_deck_config(&file), config);
    }
}
