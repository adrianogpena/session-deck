//! Port of `theme.ts`: Tokyo Night palettes, the system-theme probes, and the `T` cycle.

use std::sync::LazyLock;

use ratatui::style::{Color, Style};
use regex::{Captures, Regex};
use sdeck_core::store::deck_store::ThemePreference;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeName {
    Dark,
    Light,
}

impl ThemeName {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Bg,
    Surface,
    Border,
    Text,
    TextDim,
    Accent,
    Purple,
    Cyan,
    Green,
    Yellow,
    Orange,
    Red,
}

/// Tokyo Night (dark) and Tokyo Night Light, in [`Role`] order.
const DARK: [u32; 12] = [
    0x1a1b26, 0x24283b, 0x414868, 0xc0caf5, 0x787fa0, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0x9ece6a, 0xe0af68,
    0xff9e64, 0xf7768e,
];
const LIGHT: [u32; 12] = [
    0xd5d6db, 0xe9e9ec, 0x9699a3, 0x343b58, 0x6a6d7c, 0x34548a, 0x7847bd, 0x166775, 0x485e30, 0x8f5e15,
    0x965027, 0x8c4351,
];

/// Truecolor styles for one palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub name: ThemeName,
}

impl Theme {
    pub fn new(name: ThemeName) -> Self {
        Self { name }
    }

    pub fn color(self, role: Role) -> Color {
        let hex = match self.name {
            ThemeName::Dark => DARK,
            ThemeName::Light => LIGHT,
        }[role as usize];
        Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
    }

    pub fn fg(self, role: Role) -> Style {
        Style::new().fg(self.color(role))
    }

    pub fn bg(self, role: Role) -> Style {
        Style::new().bg(self.color(role))
    }
}

/// `T` cycles dark → light → system.
pub fn next_theme_preference(current: ThemePreference) -> ThemePreference {
    match current {
        ThemePreference::Dark => ThemePreference::Light,
        ThemePreference::Light => ThemePreference::System,
        ThemePreference::System => ThemePreference::Dark,
    }
}

pub fn preference_label(preference: ThemePreference) -> &'static str {
    match preference {
        ThemePreference::Dark => "dark",
        ThemePreference::Light => "light",
        ThemePreference::System => "system",
    }
}

/// The header's theme text: `system (dark)` when following the system, else the theme itself.
pub fn theme_label(preference: ThemePreference, resolved: ThemeName) -> String {
    match preference {
        ThemePreference::System => format!("system ({})", resolved.as_str()),
        _ => resolved.as_str().to_string(),
    }
}

/// Asks the terminal for its background color; answered via input (see [`extract_background_reply`]).
pub const OSC11_QUERY: &str = "\x1b]11;?\x07";

/// Pulls OSC 11 replies (`ESC]11;rgb:RRRR/GGGG/BBBB` ended by BEL or ST) out of an input chunk: the
/// theme the last one implies, and the input without them.
pub fn extract_background_reply(data: &str) -> (Option<ThemeName>, String) {
    static OSC11_REPLY: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\x1b\]11;rgb:([0-9a-f]+)/([0-9a-f]+)/([0-9a-f]+)(?:\x07|\x1b\\)").unwrap()
    });
    let mut theme = None;
    let rest = OSC11_REPLY.replace_all(data, |c: &Captures| {
        let channel = |i: usize| {
            let hex = &c[i];
            let max = 16f64.powi(hex.len() as i32) - 1.0;
            u64::from_str_radix(hex, 16).map_or(0.0, |v| v as f64 / max)
        };
        let luminance = 0.2126 * channel(1) + 0.7152 * channel(2) + 0.0722 * channel(3);
        theme = Some(if luminance < 0.5 {
            ThemeName::Dark
        } else {
            ThemeName::Light
        });
        ""
    });
    (theme, rest.into_owned())
}

/// Parses `reg query … /v AppsUseLightTheme` output.
fn parse_reg_theme(stdout: &str) -> Option<ThemeName> {
    static VALUE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)AppsUseLightTheme\s+REG_DWORD\s+0x([0-9a-f]+)").unwrap());
    let v = u32::from_str_radix(&VALUE.captures(stdout)?[1], 16).ok()?;
    Some(if v == 0 { ThemeName::Dark } else { ThemeName::Light })
}

/// The OS app theme (Windows registry, macOS defaults), or `None` where there's no such setting.
/// Blocking: spawns a process, so callers run it off the main thread.
pub fn read_os_theme() -> Option<ThemeName> {
    use std::process::{Command, Stdio};
    if cfg!(windows) {
        let out = Command::new("reg")
            .args([
                "query",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
                "/v",
                "AppsUseLightTheme",
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| parse_reg_theme(&String::from_utf8_lossy(&out.stdout)))?
    } else if cfg!(target_os = "macos") {
        // Prints "Dark" in dark mode; fails (no such key) in light mode.
        let out = Command::new("defaults")
            .args(["read", "-g", "AppleInterfaceStyle"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok();
        let dark = out.is_some_and(|o| {
            o.status.success() && String::from_utf8_lossy(&o.stdout).to_lowercase().contains("dark")
        });
        Some(if dark { ThemeName::Dark } else { ThemeName::Light })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palettes_resolve_roles_to_truecolor() {
        assert_eq!(
            Theme::new(ThemeName::Dark).color(Role::Bg),
            Color::Rgb(0x1a, 0x1b, 0x26)
        );
        assert_eq!(
            Theme::new(ThemeName::Light).color(Role::Red),
            Color::Rgb(0x8c, 0x43, 0x51)
        );
    }

    #[test]
    fn cycle_goes_dark_light_system() {
        let mut p = ThemePreference::Dark;
        let mut seen = vec![];
        for _ in 0..3 {
            p = next_theme_preference(p);
            seen.push(preference_label(p));
        }
        assert_eq!(seen, ["light", "system", "dark"]);
    }

    #[test]
    fn theme_label_names_the_resolved_theme_under_system() {
        assert_eq!(
            theme_label(ThemePreference::System, ThemeName::Light),
            "system (light)"
        );
        assert_eq!(theme_label(ThemePreference::Dark, ThemeName::Dark), "dark");
    }

    #[test]
    fn extract_background_reply_reads_luminance_and_strips_the_reply() {
        assert_eq!(
            extract_background_reply("a\x1b]11;rgb:1a1a/1b1b/2626\x07b"),
            (Some(ThemeName::Dark), "ab".into())
        );
        assert_eq!(
            extract_background_reply("\x1b]11;rgb:ffff/ffff/ffff\x1b\\"),
            (Some(ThemeName::Light), String::new())
        );
        assert_eq!(extract_background_reply("j"), (None, "j".into()));
    }

    #[test]
    fn parse_reg_theme_reads_the_dword() {
        let out = "\r\nHKEY_CURRENT_USER\\...\\Personalize\r\n    AppsUseLightTheme    REG_DWORD    0x0\r\n";
        assert_eq!(parse_reg_theme(out), Some(ThemeName::Dark));
        assert_eq!(
            parse_reg_theme(&out.replace("0x0", "0x1")),
            Some(ThemeName::Light)
        );
        assert_eq!(parse_reg_theme("nothing"), None);
    }
}
