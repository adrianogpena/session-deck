//! Rebindable keys. Every list action has a default key; `keybindings` in the config file overrides
//! it by action id. The list handler matches the default keys, so [`translate`] turns an overridden key
//! back into its action's default before dispatch (and swallows a default key that was moved away).
//! Keys are written as specs: a character (`x`, `?`), `space`, `ctrl+<letter>` or `f1`…`f12`.

use std::borrow::Cow;

use sdeck_core::store::deck_config::{parse_detach_letter, DeckConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyContext {
    /// The session list.
    List,
    /// The key after the chord prefix, while attached or interacting.
    Chord,
    /// The chord prefix itself.
    Prefix,
}

#[derive(Debug)]
pub struct KeyAction {
    pub id: &'static str,
    pub group: &'static str,
    pub context: KeyContext,
    pub default: &'static str,
    pub hint: &'static str,
}

const fn list(group: &'static str, id: &'static str, default: &'static str, hint: &'static str) -> KeyAction {
    KeyAction {
        id,
        group,
        context: KeyContext::List,
        default,
        hint,
    }
}

const fn chord(id: &'static str, default: &'static str, hint: &'static str) -> KeyAction {
    KeyAction {
        id,
        group: "chord",
        context: KeyContext::Chord,
        default,
        hint,
    }
}

pub const CHORD_PREFIX_ID: &str = "chordPrefix";
pub const DEFAULT_CHORD_PREFIX: &str = "ctrl+k";

pub static KEY_ACTIONS: &[KeyAction] = &[
    KeyAction {
        id: CHORD_PREFIX_ID,
        group: "chord",
        context: KeyContext::Prefix,
        default: DEFAULT_CHORD_PREFIX,
        hint: "Starts a chord while attached or interacting: this key, then one of the chord keys below. ctrl+<letter>, not c h i j m.",
    },
    chord("chordNewSession", "n", "After the prefix: leave, then a new session in the same project."),
    chord("chordSwitchMode", "t", "After the prefix: swap Attached and Interacting for the same session."),
    chord("chordStopSession", "q", "After the prefix: leave and stop the session."),
    chord("chordToggleMouse", "m", "After the prefix (interacting only): mouse on (wheel scroll, drag to copy) or off (terminal selection)."),
    chord("chordToggleSidebar", "b", "After the prefix (interacting only): show or hide the sessions panel."),
    list("navigation", "moveUp", "k", "Select the previous row; the preview follows."),
    list("navigation", "moveDown", "j", "Select the next row; the preview follows."),
    list("navigation", "collapse", "h", "Collapse the group, or go to the parent."),
    list("navigation", "expand", "l", "Expand the group, or go to its first child."),
    list("navigation", "previousSession", "`", "Back to the previously selected session."),
    list("navigation", "previousActive", "[", "Previous started session, wrapping around."),
    list("navigation", "nextActive", "]", "Next started session, wrapping around."),
    list("navigation", "search", "/", "Search every session's prompts and replies."),
    list("navigation", "commandPalette", ":", "Command palette: run any action by typed name."),
    list("sessions", "interact", "i", "Type into the selected session right here, list and preview still showing."),
    list("sessions", "newSession", "n", "New session in the project, with the default agent."),
    list("sessions", "newSessionChooseAgent", "N", "New session, choosing the agent just this once."),
    list("sessions", "chooseDefaultAgent", "f3", "Pick the default agent for new sessions."),
    list("sessions", "switchAccount", "f4", "Switch account: new sessions launch as it."),
    list("sessions", "addProject", "p", "Add a project: new session in any folder."),
    list("sessions", "sendPrompt", "o", "Send a one-line prompt without attaching."),
    list("sessions", "copyResponse", "c", "Copy the last response."),
    list("sessions", "rename", "e", "Rename the session or folder (F2 always works too)."),
    list("sessions", "clearContext", "ctrl+l", "Clear context without attaching (idle, done or error only)."),
    list("sessions", "start", "s", "Start the session in the background."),
    list("sessions", "stop", "x", "Stop the selected session (or every checked one)."),
    list("sessions", "restart", "R", "Restart: a fresh process, same conversation."),
    list("sessions", "trajectory", "v", "Structured trace of messages and tool calls (Claude sessions only)."),
    list("sessions", "refresh", "r", "Refresh the list."),
    list("organize", "multiSelect", "space", "Check the session for a batch action, then move down."),
    list("organize", "archive", "A", "Archive or unarchive."),
    list("organize", "archivedView", "^", "Show or hide archived sessions."),
    list("organize", "undoDelete", "ctrl+z", "Undo the last delete."),
    list("organize", "trash", "Z", "Open the trash."),
    list("organize", "markUnread", "u", "Mark as unread (finished, not seen)."),
    list("organize", "markRead", "U", "Mark as read."),
    list("organize", "pin", ",", "Pin: top, bottom or off."),
    list("organize", "newFolder", "g", "New folder."),
    list("organize", "moveToFolder", "M", "Move the project to a folder."),
    list("organize", "tags", "L", "Add or edit tags on the session."),
    list("organize", "reorderUp", "K", "Move the folder or project up (Shift+↑ always works too)."),
    list("organize", "reorderDown", "J", "Move the folder or project down (Shift+↓ always works too)."),
    list("organize", "delete", "d", "Delete: move to the trash, or remove the project or folder."),
    list("organize", "sortSessions", "S", "Sort sessions: recent or actionable first."),
    list("organize", "viewMode", "t", "View: normal, or groups with active sessions on top."),
    list("organize", "timeFilter", "*", "Time filter: all, today, 3 days, 7 days."),
    list("view", "toggleMouse", "m", "Mouse on (wheel scroll, drag to copy) or off (terminal selection)."),
    list("view", "toggleSidebar", "b", "Hide or show the sessions panel."),
    list("view", "narrowSidebar", "<", "Narrow the sessions panel."),
    list("view", "widenSidebar", ">", "Widen the sessions panel."),
    list("view", "help", "?", "Opens the config popup on this tab."),
    list("view", "config", "C", "This config popup."),
    list("view", "skills", "w", "Local skills, agents and account usage."),
    list("view", "alerts", "a", "Alert history."),
    list("view", "quit", "q", "Quit (stops background sessions); Ctrl+C always works too."),
];

pub fn action(id: &str) -> Option<&'static KeyAction> {
    KEY_ACTIONS.iter().find(|a| a.id == id)
}

const FKEY_TILDE: [u8; 8] = [15, 17, 18, 19, 20, 21, 23, 24];
/// The list reserves these for its own use (hotkeys, status filters).
const RESERVED_CHARS: &str = "0123456789!@#&~";

/// The normalized form of `spec`, or `None` when it names no usable key.
pub fn parse_spec(spec: &str) -> Option<String> {
    let spec = spec.trim();
    let lower = spec.to_ascii_lowercase();
    if lower == "space" {
        return Some(lower);
    }
    if let Some(letter) = lower.strip_prefix("ctrl+") {
        let mut chars = letter.chars();
        let c = chars.next()?;
        let ok = chars.next().is_none() && c.is_ascii_lowercase() && !"chijm".contains(c);
        return ok.then_some(lower);
    }
    if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
        return (1..=12).contains(&n).then_some(lower);
    }
    let mut chars = spec.chars();
    let c = chars.next()?;
    (chars.next().is_none() && !c.is_control() && !c.is_whitespace()).then(|| spec.to_string())
}

/// The strings the terminal sends for `spec`, as the app's `on_key` sees them.
pub fn encodings(spec: &str) -> Vec<String> {
    let Some(spec) = parse_spec(spec) else {
        return Vec::new();
    };
    if spec == "space" {
        return vec![" ".into()];
    }
    if let Some(letter) = spec.strip_prefix("ctrl+") {
        let byte = letter.as_bytes()[0] - b'a' + 1;
        return vec![char::from(byte).to_string()];
    }
    if let Some(n) = spec.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
        return if n <= 4 {
            let letter = char::from(b'P' + n - 1);
            vec![format!("\x1bO{letter}"), format!("\x1b[{}~", 10 + n)]
        } else {
            vec![format!("\x1b[{}~", FKEY_TILDE[usize::from(n - 5)])]
        };
    }
    vec![spec]
}

/// The spec of the key a terminal sent as `raw`; `None` for keys that can't be bound.
pub fn spec_from_raw(raw: &str) -> Option<String> {
    for n in 1..=12u8 {
        if encodings(&format!("f{n}")).iter().any(|e| e == raw) {
            return Some(format!("f{n}"));
        }
    }
    let mut chars = raw.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    match c {
        ' ' => Some("space".into()),
        '\x01'..='\x1a' => parse_spec(&format!("ctrl+{}", char::from(c as u8 + b'a' - 1))),
        c if c.is_control() => None,
        c => Some(c.to_string()),
    }
}

/// A spec as shown in the popup: `Ctrl+E`, `F3`, `Space`, `x`.
pub fn display_spec(spec: &str) -> String {
    if let Some(letter) = spec.strip_prefix("ctrl+") {
        format!("Ctrl+{}", letter.to_ascii_uppercase())
    } else if spec == "space" {
        "Space".into()
    } else if spec.len() > 1 && spec.starts_with('f') {
        spec.to_ascii_uppercase()
    } else {
        spec.to_string()
    }
}

/// The key `action` is on now: the override when it is a valid spec, else the default.
pub fn effective_spec(config: &DeckConfig, action: &KeyAction) -> String {
    config
        .keybindings
        .get(action.id)
        .and_then(|s| parse_spec(s))
        .unwrap_or_else(|| action.default.to_string())
}

/// The key `key` stands for in the list: its action's default when it is an override, `None` when it
/// is the default of an action that was moved elsewhere.
pub fn translate<'a>(config: &DeckConfig, key: &'a str) -> Option<Cow<'a, str>> {
    if config.keybindings.is_empty() {
        return Some(Cow::Borrowed(key));
    }
    let moved: Vec<(&KeyAction, String)> = KEY_ACTIONS
        .iter()
        .filter(|a| a.context == KeyContext::List)
        .map(|a| (a, effective_spec(config, a)))
        .filter(|(a, spec)| spec != a.default)
        .collect();
    for (a, spec) in &moved {
        if encodings(spec).iter().any(|e| e == key) {
            let canonical = encodings(a.default).swap_remove(0);
            return Some(Cow::Owned(canonical));
        }
    }
    if moved
        .iter()
        .any(|(a, _)| encodings(a.default).iter().any(|e| e == key))
    {
        return None;
    }
    Some(Cow::Borrowed(key))
}

/// The letters of the chord's resolving keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChordKeys {
    pub new_session: char,
    pub switch_mode: char,
    pub stop_session: char,
    pub toggle_mouse: char,
    pub toggle_sidebar: char,
}

fn letter_of(config: &DeckConfig, id: &str) -> char {
    let action = action(id).expect("chord action exists");
    let spec = effective_spec(config, action);
    spec.chars()
        .next()
        .filter(|c| spec.len() == 1 && c.is_ascii_alphabetic())
        .map_or_else(
            || action.default.chars().next().unwrap(),
            |c| c.to_ascii_lowercase(),
        )
}

impl ChordKeys {
    pub fn from_config(config: &DeckConfig) -> Self {
        Self {
            new_session: letter_of(config, "chordNewSession"),
            switch_mode: letter_of(config, "chordSwitchMode"),
            stop_session: letter_of(config, "chordStopSession"),
            toggle_mouse: letter_of(config, "chordToggleMouse"),
            toggle_sidebar: letter_of(config, "chordToggleSidebar"),
        }
    }
}

/// The letter of the chord prefix (`ctrl+k` unless overridden).
pub fn chord_prefix_letter(config: &DeckConfig) -> char {
    let action = action(CHORD_PREFIX_ID).expect("prefix action exists");
    let spec = effective_spec(config, action);
    spec.strip_prefix("ctrl+")
        .and_then(|l| l.chars().next())
        .unwrap_or('k')
}

/// What a row of the Keybindings tab changes.
#[derive(Clone, Copy)]
pub enum Target {
    Action(&'static KeyAction),
    Detach,
}

fn pretty(id: &str) -> String {
    let mut out = String::new();
    for (i, c) in id.chars().enumerate() {
        if c.is_ascii_uppercase() {
            out.push(' ');
            out.push(c.to_ascii_lowercase());
        } else if i == 0 {
            out.extend(c.to_uppercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// The spec to store for `input` on `target`, or why it can't be used. Empty input is the default key.
pub fn validate(config: &DeckConfig, target: Target, input: &str) -> Result<String, String> {
    let default = match target {
        Target::Action(a) => a.default,
        Target::Detach => "ctrl+q",
    };
    let input = if input.trim().is_empty() { default } else { input };
    let context = match target {
        Target::Action(a) => a.context,
        Target::Detach => KeyContext::Prefix,
    };
    let own_id = match target {
        Target::Action(a) => a.id,
        Target::Detach => "",
    };
    match context {
        KeyContext::List => {
            let spec = parse_spec(input).ok_or("Not a key sdeck can use")?;
            if spec.chars().count() == 1 && spec.chars().all(|c| RESERVED_CHARS.contains(c)) {
                return Err(format!("{spec} is reserved (jump hotkeys and status filters)"));
            }
            let taken = KEY_ACTIONS
                .iter()
                .filter(|a| a.context == KeyContext::List && a.id != own_id)
                .find(|a| effective_spec(config, a) == spec);
            match taken {
                Some(a) => Err(format!("{} is already {}", display_spec(&spec), pretty(a.id))),
                None => Ok(spec),
            }
        }
        KeyContext::Chord => {
            let mut chars = input.trim().chars();
            let letter = chars
                .next()
                .filter(|c| chars.next().is_none() && c.is_ascii_alphabetic())
                .ok_or("Chord keys are a single letter")?
                .to_ascii_lowercase();
            let taken = KEY_ACTIONS
                .iter()
                .filter(|a| a.context == KeyContext::Chord && a.id != own_id)
                .find(|a| effective_spec(config, a) == letter.to_string());
            match taken {
                Some(a) => Err(format!("{letter} is already {}", pretty(a.id))),
                None => Ok(letter.to_string()),
            }
        }
        KeyContext::Prefix => {
            let letter = match target {
                Target::Detach => parse_detach_letter(input),
                // The prefix may be Ctrl+K, which a detach key may not.
                Target::Action(_) => {
                    parse_spec(input).and_then(|s| s.strip_prefix("ctrl+").and_then(|l| l.chars().next()))
                }
            }
            .ok_or("Use ctrl+<letter>, not c h i j m")?;
            let other = match target {
                Target::Detach => chord_prefix_letter(config),
                Target::Action(_) => config.ui.detach_letter(),
            };
            if letter == other {
                return Err(format!(
                    "Ctrl+{} is already the other one",
                    letter.to_ascii_uppercase()
                ));
            }
            Ok(format!("ctrl+{letter}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdeck_core::store::deck_config::parse_deck_config;

    fn base() -> DeckConfig {
        parse_deck_config("{}")
    }

    fn with(id: &str, spec: &str) -> DeckConfig {
        let mut c = base();
        c.keybindings.insert(id.into(), spec.into());
        c
    }

    #[test]
    fn default_keys_are_unique_per_context_and_valid() {
        for ctx in [KeyContext::List, KeyContext::Chord] {
            let mut seen = Vec::new();
            for a in KEY_ACTIONS.iter().filter(|a| a.context == ctx) {
                let spec = parse_spec(a.default).unwrap_or_else(|| panic!("{} default", a.id));
                assert!(!seen.contains(&spec), "{} duplicates {spec}", a.id);
                seen.push(spec);
            }
        }
    }

    #[test]
    fn specs_encode_to_what_the_terminal_sends() {
        assert_eq!(encodings("x"), ["x"]);
        assert_eq!(encodings("Ctrl+L"), ["\x0c"]);
        assert_eq!(encodings("space"), [" "]);
        assert_eq!(encodings("f3"), ["\x1bOR", "\x1b[13~"]);
        assert_eq!(encodings("f5"), ["\x1b[15~"]);
        assert!(encodings("ctrl+h").is_empty());
        assert!(encodings("ctrl+c").is_empty());
        assert!(encodings("f13").is_empty());
        assert!(encodings("ab").is_empty());
    }

    #[test]
    fn a_pressed_key_becomes_its_spec_and_back() {
        for raw in ["x", "?", " ", "\x05", "\x1bOQ", "\x1b[15~", "É"] {
            let spec = spec_from_raw(raw).unwrap();
            assert!(encodings(&spec).contains(&raw.to_string()), "{raw:?} -> {spec}");
        }
        for raw in ["\x08", "\t", "\r", "\x03", "\x1b", "\x1b[A", "ab"] {
            assert_eq!(spec_from_raw(raw), None, "{raw:?}");
        }
        assert_eq!(display_spec("ctrl+e"), "Ctrl+E");
        assert_eq!(display_spec("f3"), "F3");
        assert_eq!(display_spec("space"), "Space");
    }

    #[test]
    fn translate_maps_an_override_to_the_default_and_swallows_the_moved_default() {
        let c = with("newSession", "y");
        assert_eq!(translate(&c, "y").as_deref(), Some("n"));
        assert_eq!(translate(&c, "n"), None);
        assert_eq!(translate(&c, "j").as_deref(), Some("j"));
        let f = with("rename", "f6");
        assert_eq!(translate(&f, "\x1b[17~").as_deref(), Some("e"));
        assert_eq!(translate(&base(), "n").as_deref(), Some("n"));
    }

    #[test]
    fn validate_refuses_taken_reserved_and_unusable_keys() {
        let new = Target::Action(action("newSession").unwrap());
        assert_eq!(validate(&base(), new, "y").unwrap(), "y");
        assert_eq!(validate(&base(), new, "").unwrap(), "n");
        assert!(validate(&base(), new, "j").unwrap_err().contains("Move down"));
        assert!(validate(&base(), new, "5").unwrap_err().contains("reserved"));
        assert!(validate(&base(), new, "!").is_err());
        assert!(validate(&base(), new, "nope").is_err());
        assert_eq!(validate(&base(), new, "n").unwrap(), "n");
    }

    #[test]
    fn chord_keys_are_single_letters_unique_among_chords() {
        let stop = Target::Action(action("chordStopSession").unwrap());
        assert_eq!(validate(&base(), stop, "X").unwrap(), "x");
        assert!(validate(&base(), stop, "n").is_err());
        assert!(validate(&base(), stop, "1").is_err());
        let c = with("chordNewSession", "z");
        assert_eq!(ChordKeys::from_config(&c).new_session, 'z');
        assert_eq!(ChordKeys::from_config(&base()).stop_session, 'q');
    }

    #[test]
    fn detach_and_chord_prefix_cannot_share_a_letter() {
        let prefix = Target::Action(action(CHORD_PREFIX_ID).unwrap());
        assert_eq!(validate(&base(), prefix, "ctrl+e").unwrap(), "ctrl+e");
        assert!(validate(&base(), prefix, "ctrl+q").is_err());
        assert!(validate(&base(), Target::Detach, "ctrl+k").is_err());
        assert_eq!(validate(&base(), Target::Detach, "ctrl+e").unwrap(), "ctrl+e");
        let moved = with(CHORD_PREFIX_ID, "ctrl+e");
        assert_eq!(chord_prefix_letter(&moved), 'e');
        assert_eq!(chord_prefix_letter(&base()), 'k');
        assert!(validate(&moved, Target::Detach, "ctrl+e").is_err());
    }
}
