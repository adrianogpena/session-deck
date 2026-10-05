//! Color palettes and the system-theme probes.

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

/// One named color scheme: its light/dark kind and a color per [`Role`], in `Role` order.
#[derive(Debug, PartialEq, Eq)]
pub struct Palette {
    pub id: &'static str,
    pub label: &'static str,
    /// Palettes of one family are light and dark takes on the same scheme.
    pub family: &'static str,
    pub kind: ThemeName,
    colors: [u32; 12],
}

pub const DEFAULT_DARK: &str = "tokyo-night";
pub const DEFAULT_LIGHT: &str = "tokyo-night-light";
/// Index in [`PALETTES`] of the default dark and light palettes.
const DEFAULT_DARK_INDEX: usize = 0;
const DEFAULT_LIGHT_INDEX: usize = 7;

const fn palette(
    id: &'static str,
    label: &'static str,
    family: &'static str,
    kind: ThemeName,
    colors: [u32; 12],
) -> Palette {
    Palette {
        id,
        label,
        family,
        kind,
        colors,
    }
}

use ThemeName::{Dark as D, Light as L};

#[rustfmt::skip]
pub static PALETTES: [Palette; 13] = [
    palette("tokyo-night", "Tokyo Night", "tokyo-night", D, [
        0x1a1b26, 0x24283b, 0x414868, 0xc0caf5, 0x787fa0, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0x9ece6a, 0xe0af68,
        0xff9e64, 0xf7768e,
    ]),
    palette("catppuccin-mocha", "Catppuccin Mocha", "catppuccin", D, [
        0x1e1e2e, 0x313244, 0x585b70, 0xcdd6f4, 0x7f849c, 0x89b4fa, 0xcba6f7, 0x94e2d5, 0xa6e3a1, 0xf9e2af,
        0xfab387, 0xf38ba8,
    ]),
    palette("gruvbox-dark", "Gruvbox Dark", "gruvbox", D, [
        0x282828, 0x3c3836, 0x665c54, 0xebdbb2, 0xa89984, 0x83a598, 0xd3869b, 0x8ec07c, 0xb8bb26, 0xfabd2f,
        0xfe8019, 0xfb4934,
    ]),
    palette("nord", "Nord", "nord", D, [
        0x2e3440, 0x3b4252, 0x4c566a, 0xd8dee9, 0x7b88a1, 0x81a1c1, 0xb48ead, 0x88c0d0, 0xa3be8c, 0xebcb8b,
        0xd08770, 0xbf616a,
    ]),
    palette("dracula", "Dracula", "dracula", D, [
        0x282a36, 0x44475a, 0x6272a4, 0xf8f8f2, 0x8f9bd0, 0xbd93f9, 0xff79c6, 0x8be9fd, 0x50fa7b, 0xf1fa8c,
        0xffb86c, 0xff5555,
    ]),
    palette("rose-pine", "Rose Pine", "rose-pine", D, [
        0x191724, 0x1f1d2e, 0x403d52, 0xe0def4, 0x908caa, 0xc4a7e7, 0xc4a7e7, 0x9ccfd8, 0x31748f, 0xf6c177,
        0xebbcba, 0xeb6f92,
    ]),
    palette("solarized-dark", "Solarized Dark", "solarized", D, [
        0x002b36, 0x073642, 0x586e75, 0x93a1a1, 0x657b83, 0x268bd2, 0x6c71c4, 0x2aa198, 0x859900, 0xb58900,
        0xcb4b16, 0xdc322f,
    ]),
    palette("tokyo-night-light", "Tokyo Night Light", "tokyo-night", L, [
        0xd5d6db, 0xe9e9ec, 0x9699a3, 0x343b58, 0x6a6d7c, 0x34548a, 0x7847bd, 0x166775, 0x485e30, 0x8f5e15,
        0x965027, 0x8c4351,
    ]),
    palette("catppuccin-latte", "Catppuccin Latte", "catppuccin", L, [
        0xeff1f5, 0xccd0da, 0xacb0be, 0x4c4f69, 0x6c6f85, 0x1e66f5, 0x8839ef, 0x179299, 0x358021, 0xa8680f,
        0xcc4a00, 0xd20f39,
    ]),
    palette("gruvbox-light", "Gruvbox Light", "gruvbox", L, [
        0xfbf1c7, 0xebdbb2, 0xa89984, 0x3c3836, 0x7c6f64, 0x076678, 0x8f3f71, 0x427b58, 0x79740e, 0xb57614,
        0xaf3a03, 0x9d0006,
    ]),
    palette("rose-pine-dawn", "Rose Pine Dawn", "rose-pine", L, [
        0xfaf4ed, 0xfffaf3, 0xcecacd, 0x575279, 0x797593, 0x907aa9, 0x907aa9, 0x56949f, 0x286983, 0xb36b00,
        0xb85c58, 0xb4637a,
    ]),
    palette("solarized-light", "Solarized Light", "solarized", L, [
        0xfdf6e3, 0xeee8d5, 0x93a1a1, 0x586e75, 0x78898b, 0x268bd2, 0x6c71c4, 0x1d8a82, 0x6f8000, 0x9d7500,
        0xcb4b16, 0xdc322f,
    ]),
    palette("alucard", "Alucard", "dracula", L, [
        0xfffbeb, 0xece6d0, 0xc8c2a8, 0x1f1f1f, 0x6c664b, 0x644ac9, 0xa3144d, 0x036a96, 0x14710a, 0x846e15,
        0xa34d14, 0xcb3a2a,
    ]),
];

pub fn palette_by_id(id: &str) -> Option<&'static Palette> {
    PALETTES.iter().find(|p| p.id == id)
}

/// The palette a saved `id` names, if it is one of `kind`; else the `kind` take on the family of the
/// other side's saved palette (`Dracula` → `Alucard`); else that kind's default.
pub fn resolve_palette(kind: ThemeName, id: Option<&str>, other_side_id: Option<&str>) -> &'static Palette {
    let of_kind = |p: &&'static Palette| p.kind == kind;
    id.and_then(palette_by_id)
        .filter(of_kind)
        .or_else(|| {
            let sibling = other_side_id.and_then(palette_by_id)?;
            PALETTES
                .iter()
                .find(|p| p.family == sibling.family && p.kind == kind)
        })
        .unwrap_or_else(|| Theme::new(kind).palette)
}

/// Truecolor styles for one palette; `name` is the palette's light/dark kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub name: ThemeName,
    pub palette: &'static Palette,
}

impl Theme {
    /// The default palette of `name`'s kind.
    pub const fn new(name: ThemeName) -> Self {
        Self {
            name,
            palette: &PALETTES[match name {
                ThemeName::Dark => DEFAULT_DARK_INDEX,
                ThemeName::Light => DEFAULT_LIGHT_INDEX,
            }],
        }
    }

    pub fn from_palette(palette: &'static Palette) -> Self {
        Self {
            name: palette.kind,
            palette,
        }
    }

    pub fn color(self, role: Role) -> Color {
        let hex = self.palette.colors[role as usize];
        Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
    }

    pub fn fg(self, role: Role) -> Style {
        Style::new().fg(self.color(role))
    }

    pub fn bg(self, role: Role) -> Style {
        Style::new().bg(self.color(role))
    }
}

/// The theme text: `system (Nord)` when following the system, else the palette's label.
pub fn theme_label(preference: ThemePreference, theme: Theme) -> String {
    match preference {
        ThemePreference::System => format!("system ({})", theme.palette.label),
        _ => theme.palette.label.to_string(),
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
    fn theme_label_names_the_resolved_theme_under_system() {
        assert_eq!(
            theme_label(ThemePreference::System, Theme::new(ThemeName::Light)),
            "system (Tokyo Night Light)"
        );
        let nord = Theme::from_palette(palette_by_id("nord").unwrap());
        assert_eq!(theme_label(ThemePreference::Dark, nord), "Nord");
    }

    fn luminance(hex: u32) -> f64 {
        let lin = |c: u32| {
            let c = c as f64 / 255.0;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(hex >> 16 & 0xff) + 0.7152 * lin(hex >> 8 & 0xff) + 0.0722 * lin(hex & 0xff)
    }

    fn contrast(a: u32, b: u32) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn palette_ids_are_unique_and_defaults_match_their_kind() {
        let mut ids: Vec<_> = PALETTES.iter().map(|p| p.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), PALETTES.len());
        assert_eq!(Theme::new(ThemeName::Dark).palette.id, DEFAULT_DARK);
        assert_eq!(Theme::new(ThemeName::Light).palette.id, DEFAULT_LIGHT);
        assert_eq!(Theme::new(ThemeName::Dark).palette.kind, ThemeName::Dark);
        assert_eq!(Theme::new(ThemeName::Light).palette.kind, ThemeName::Light);
    }

    #[test]
    fn resolve_palette_ignores_unknown_ids_and_the_wrong_kind() {
        assert_eq!(resolve_palette(ThemeName::Dark, Some("nord"), None).id, "nord");
        assert_eq!(
            resolve_palette(ThemeName::Dark, Some("nope"), None).id,
            DEFAULT_DARK
        );
        assert_eq!(
            resolve_palette(ThemeName::Light, Some("nord"), None).id,
            DEFAULT_LIGHT
        );
        assert_eq!(resolve_palette(ThemeName::Light, None, None).id, DEFAULT_LIGHT);
    }

    #[test]
    fn the_unpicked_side_follows_the_picked_palettes_family() {
        let light = |other| resolve_palette(ThemeName::Light, None, other).id;
        assert_eq!(light(Some("dracula")), "alucard");
        assert_eq!(light(Some("catppuccin-mocha")), "catppuccin-latte");
        assert_eq!(light(Some("nord")), DEFAULT_LIGHT);
        assert_eq!(
            resolve_palette(ThemeName::Light, Some("gruvbox-light"), Some("dracula")).id,
            "gruvbox-light"
        );
        assert_eq!(
            resolve_palette(ThemeName::Dark, None, Some("alucard")).id,
            "dracula"
        );
    }

    #[test]
    fn every_palette_is_readable_on_its_background() {
        for p in &PALETTES {
            let bg = p.colors[Role::Bg as usize];
            let check = |role: Role, min: f64| {
                let ratio = contrast(p.colors[role as usize], bg);
                assert!(ratio >= min, "{} {:?}: {ratio:.2} < {min}", p.id, role);
            };
            check(Role::Text, 4.5);
            for role in [
                Role::TextDim,
                Role::Accent,
                Role::Purple,
                Role::Cyan,
                Role::Green,
                Role::Yellow,
                Role::Orange,
                Role::Red,
            ] {
                check(role, 3.0);
            }
        }
    }

    #[test]
    fn status_hues_stay_distinct_in_every_palette() {
        for p in &PALETTES {
            let c = |r: Role| p.colors[r as usize];
            assert_ne!(c(Role::Green), c(Role::Red), "{}", p.id);
            assert_ne!(c(Role::Yellow), c(Role::Red), "{}", p.id);
            assert_ne!(c(Role::Accent), c(Role::Green), "{}", p.id);
        }
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
