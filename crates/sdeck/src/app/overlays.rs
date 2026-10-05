//! The popups opened by `?`, `C`, `w`, `a` and `:` — one at a time, and while one is open it takes
//! every key (a prompt popup, opened from the config popup, takes them first). Port of the
//! `onHelpKey` / `onConfigKey` / `onSkillsKey` / `onAlertsKey` / `onCommandPaletteKey` handlers.
//! Search (`/`) and the trajectory (`v`) have their own modules.

use std::cell::Cell;
use std::path::PathBuf;
use std::time::Instant;

use ratatui::Frame;
use sdeck_core::format::now_ms;
use sdeck_core::paths::claude_dir;
use sdeck_core::status::account::discover_accounts;
use sdeck_core::status::alert_log::{read_alerts, AlertEntry, ALERT_LOG_CAP};
use sdeck_core::store::deck_config::{deck_config_path, write_deck_config};

use super::search::SearchState;
use super::trace::TraceState;
use super::App;
use crate::agents::{agents_dir, discover_local_agents, LocalAgent};
use crate::commands::{palette_matches, CommandId, PALETTE_COMMANDS};
use crate::config_fields::{tab_fields, ConfigFieldKind, ConfigTab, CONFIG_FIELDS};
use crate::keybindings::spec_from_raw;
use crate::skills::{claude_settings_path, discover_local_skills, skills_dir, LocalSkill};
use crate::theme::Theme;
use crate::view::overlays::AccountUsage;
use crate::view::overlays::{
    render_alerts, render_config, render_palette, render_search, render_skills, render_trace,
    SearchResultRow, SkillsTab,
};

pub(super) struct PaletteState {
    pub query: String,
    pub index: usize,
}

pub(super) enum Overlay {
    Config {
        tab: ConfigTab,
        /// Position within the tab's rows.
        selected: usize,
        /// Waiting for the key to bind to the selected Keybindings row.
        capturing: bool,
    },
    Skills {
        tab: SkillsTab,
        scroll: Cell<usize>,
        skills: Vec<LocalSkill>,
        agents: Vec<LocalAgent>,
        usage: Vec<AccountUsage>,
    },
    Trace(TraceState),
    Alerts {
        alerts: Vec<AlertEntry>,
        scroll: Cell<usize>,
    },
    Search(SearchState),
    Palette(PaletteState),
}

fn is_up(key: &str) -> bool {
    matches!(key, "\x1b[A" | "\x1bOA" | "k")
}

fn is_down(key: &str) -> bool {
    matches!(key, "\x1b[B" | "\x1bOB" | "j")
}

/// A typed or pasted chunk, minus control characters; `None` for an escape sequence.
pub(super) fn typed_text(key: &str) -> Option<String> {
    (!key.starts_with('\x1b')).then(|| key.chars().filter(|c| !c.is_control()).collect())
}

fn step(index: usize, key: &str, len: usize) -> usize {
    if is_up(key) {
        index.saturating_sub(1)
    } else if is_down(key) {
        (index + 1).min(len.saturating_sub(1))
    } else {
        index
    }
}

impl App {
    pub(super) fn open_config(&mut self) {
        self.open_config_tab(ConfigTab::General);
    }

    /// `?`: the config popup on the Keybindings tab.
    pub(super) fn open_keybindings(&mut self) {
        self.open_config_tab(ConfigTab::Keybindings);
    }

    fn open_config_tab(&mut self, tab: ConfigTab) {
        self.overlay = Some(Overlay::Config {
            tab,
            selected: 0,
            capturing: false,
        });
    }

    /// Skills and subagents are a Claude Code concept: shown only while that's the Session Deck
    /// Agent; the popup itself says so otherwise.
    pub(super) fn open_skills(&mut self) {
        let claude = self.active_agent == "claude";
        let config_dir = self
            .active_account()
            .map_or_else(claude_dir, |a| a.config_dir.clone());
        self.overlay = Some(Overlay::Skills {
            tab: SkillsTab::Skills,
            scroll: Cell::new(0),
            skills: if claude {
                discover_local_skills(&skills_dir(&config_dir), &claude_settings_path(&config_dir))
            } else {
                Vec::new()
            },
            agents: if claude {
                discover_local_agents(&agents_dir(&config_dir))
            } else {
                Vec::new()
            },
            usage: if claude { self.account_usages() } else { Vec::new() },
        });
    }

    pub(super) fn open_alerts(&mut self) {
        self.overlay = Some(Overlay::Alerts {
            alerts: read_alerts(ALERT_LOG_CAP),
            scroll: Cell::new(0),
        });
    }

    pub(super) fn open_palette(&mut self) {
        self.overlay = Some(Overlay::Palette(PaletteState {
            query: String::new(),
            index: 0,
        }));
    }

    pub(super) fn on_overlay_key(&mut self, key: &str, now: Instant) {
        self.dirty = true;
        match &self.overlay {
            Some(Overlay::Config { .. }) => self.on_config_key(key, now),
            Some(Overlay::Skills { .. }) => self.on_skills_key(key),
            Some(Overlay::Trace(_)) => self.on_trace_key(key),
            Some(Overlay::Alerts { .. }) => self.on_alerts_key(key),
            Some(Overlay::Search(_)) => self.on_search_key(key, now),
            Some(Overlay::Palette(_)) => self.on_palette_key(key, now),
            None => {}
        }
    }

    fn scroll_by_key(scroll: &Cell<usize>, key: &str) {
        if is_up(key) {
            scroll.set(scroll.get().saturating_sub(1));
        } else if is_down(key) {
            scroll.set(scroll.get() + 1);
        }
    }

    fn on_alerts_key(&mut self, key: &str) {
        match (&self.overlay, key) {
            (_, "\x1b" | "a" | "q") => self.overlay = None,
            (Some(Overlay::Alerts { scroll, .. }), _) => Self::scroll_by_key(scroll, key),
            _ => {}
        }
    }

    fn on_skills_key(&mut self, key: &str) {
        match (&mut self.overlay, key) {
            (_, "\x1b" | "w" | "q") => self.overlay = None,
            (Some(Overlay::Skills { tab, scroll, .. }), "\x1b[C" | "\x1bOC") => {
                *tab = tab.next();
                scroll.set(0);
            }
            (Some(Overlay::Skills { tab, scroll, .. }), "\x1b[D" | "\x1bOD") => {
                *tab = tab.prev();
                scroll.set(0);
            }
            (Some(Overlay::Skills { scroll, .. }), _) => Self::scroll_by_key(scroll, key),
            _ => {}
        }
    }

    fn on_config_key(&mut self, key: &str, now: Instant) {
        let Some(Overlay::Config {
            tab,
            selected,
            capturing,
        }) = &mut self.overlay
        else {
            return;
        };
        let rows = tab_fields(*tab);
        if *capturing {
            *capturing = false;
            if key == "\x1b" {
                return;
            }
            let index = rows[*selected];
            match spec_from_raw(key) {
                Some(spec) => self.apply_config_field(index, &spec, now),
                None => self.flash("That key can't be bound".into(), now),
            }
            return;
        }
        match key {
            "\r" => {
                let index = rows[*selected];
                self.edit_config_field(index, now);
            }
            "\x1b" | "C" | "q" | "?" => self.overlay = None,
            "\x1b[C" | "\x1bOC" | "l" | "\t" => {
                *tab = tab.next();
                *selected = 0;
            }
            "\x1b[D" | "\x1bOD" | "h" | "\x1b[Z" => {
                *tab = tab.prev();
                *selected = 0;
            }
            "\x7f" | "\x08" | "\x1b[3~" if *tab == ConfigTab::Keybindings => {
                let index = rows[*selected];
                self.apply_config_field(index, "", now);
            }
            _ => *selected = step(*selected, key, rows.len()),
        }
    }

    /// Toggles flip right away, keys wait for the next keypress, the rest open a prompt pre-filled
    /// with their current value.
    fn edit_config_field(&mut self, index: usize, now: Instant) {
        let field = &CONFIG_FIELDS[index];
        match field.kind {
            ConfigFieldKind::Toggle => self.apply_config_field(index, "", now),
            ConfigFieldKind::Key => {
                if let Some(Overlay::Config { capturing, .. }) = &mut self.overlay {
                    *capturing = true;
                }
            }
            _ => {
                let value = field.edit_value(&self.config);
                self.open_prompt(
                    field.pretty_name(),
                    value,
                    super::input::PromptAction::ConfigField(index),
                );
            }
        }
    }

    pub(super) fn apply_config_field(&mut self, index: usize, input: &str, now: Instant) {
        let field = &CONFIG_FIELDS[index];
        let Some(next) = field.apply(&self.config, input) else {
            let reason = field
                .key_error(&self.config, input)
                .unwrap_or_else(|| format!("Invalid value for {}", field.pretty_name()));
            self.flash(reason, now);
            return;
        };
        let usage_turned_on = !self.config.ui.show_usage && next.ui.show_usage;
        let max_sessions_changed = self.config.ui.max_sessions_listed != next.ui.max_sessions_listed;
        let ctl_changed = self.config.ui.ctl != next.ui.ctl;
        let layout_changed = self.config.ui.layout != next.ui.layout;
        self.config = next;
        if layout_changed {
            self.clear_screen = true;
            self.resize_all_to_pane();
        }
        let _ = write_deck_config(&self.config, &deck_config_path());
        // A `tools.*.command` edit shouldn't need a restart to take effect.
        self.executables.clear();
        if self.sources_enabled {
            self.executables.warm(&self.config);
        }
        // e.g. `ui.recentProjectsFirst` reorders the tree right away, `accounts.*` regroups it.
        self.rebuild_rows();
        if self.sources_enabled {
            // Turning `ui.gitStatus` on shows markers now, not after the next poll.
            self.poll_git_status();
            if max_sessions_changed {
                self.spawn_discovery();
            }
        }
        let mut message = if field.kind == ConfigFieldKind::Key {
            format!("{} is now {}", field.pretty_name(), field.display(&self.config))
        } else {
            format!("{} updated", field.label)
        };
        if usage_turned_on {
            let mut dirs: Vec<PathBuf> = discover_accounts().into_iter().map(|a| a.config_dir).collect();
            for known in &self.accounts {
                if !dirs.contains(&known.config_dir) {
                    dirs.push(known.config_dir.clone());
                }
            }
            message.push_str(&format!(" ({})", Self::enable_usage_statusline(&dirs)));
        }
        if ctl_changed {
            message.push_str(" (restart sdeck to apply)");
        }
        self.flash(message, now);
    }

    /// Empty while the query is blank: nothing is listed yet, so nothing should run on Enter either.
    fn palette_matches_of(query: &str) -> Vec<usize> {
        if query.trim().is_empty() {
            return Vec::new();
        }
        let labels: Vec<&str> = PALETTE_COMMANDS.iter().map(|c| c.label).collect();
        palette_matches(query, &labels)
    }

    fn on_palette_key(&mut self, key: &str, now: Instant) {
        let Some(Overlay::Palette(palette)) = &mut self.overlay else {
            return;
        };
        match key {
            "\x1b" | "\x03" => self.overlay = None,
            "\r" => {
                let matches = Self::palette_matches_of(&palette.query);
                let command = matches.get(palette.index).map(|&i| PALETTE_COMMANDS[i].id);
                self.overlay = None;
                if let Some(id) = command {
                    self.run_palette_command(id, now);
                }
            }
            "\x1b[A" | "\x1bOA" => palette.index = palette.index.saturating_sub(1),
            "\x1b[B" | "\x1bOB" => {
                let count = Self::palette_matches_of(&palette.query).len();
                palette.index = (palette.index + 1).min(count.saturating_sub(1));
            }
            "\x7f" | "\x08" => {
                palette.query.pop();
                palette.index = 0;
            }
            "\x15" => {
                palette.query.clear();
                palette.index = 0;
            }
            _ => {
                if let Some(text) = typed_text(key) {
                    palette.query.push_str(&text);
                    palette.index = 0;
                }
            }
        }
    }

    fn run_palette_command(&mut self, id: CommandId, now: Instant) {
        match id {
            CommandId::NewSession => {
                let agent = self.active_agent.clone();
                self.new_session(&agent, now);
            }
            CommandId::NewSessionChooseAgent => self.open_agent_picker(false, now),
            CommandId::Rename => self.rename(now),
            CommandId::MoveToFolder => self.open_move_picker(now),
            CommandId::OpenConfig => self.open_config(),
            CommandId::ChooseTheme => self.open_theme_picker(),
            CommandId::ToggleSidebar => self.toggle_sidebar(),
            CommandId::OpenTrash => self.open_trash_picker(now),
        }
    }

    pub(super) fn draw_overlay(&self, frame: &mut Frame, t: Theme) {
        match &self.overlay {
            None => {}
            Some(Overlay::Config {
                tab,
                selected,
                capturing,
            }) => render_config(
                frame,
                t,
                &self.config,
                &deck_config_path().to_string_lossy(),
                *tab,
                *selected,
                *capturing,
            ),
            Some(Overlay::Skills {
                tab,
                scroll,
                skills,
                agents,
                usage,
            }) => render_skills(frame, t, *tab, skills, agents, usage, scroll, &self.active_agent),
            Some(Overlay::Trace(trace)) => {
                render_trace(frame, t, &trace.steps, trace.selected, &trace.detail_scroll)
            }
            Some(Overlay::Alerts { alerts, scroll }) => render_alerts(frame, t, alerts, scroll, now_ms()),
            Some(Overlay::Search(search)) => {
                let rows: Vec<SearchResultRow> = self.search_result_rows(search);
                render_search(frame, t, &search.query, search.loading, &rows, search.index);
            }
            Some(Overlay::Palette(palette)) => {
                let matches = Self::palette_matches_of(&palette.query);
                let labels: Vec<&str> = matches.iter().map(|&i| PALETTE_COMMANDS[i].label).collect();
                render_palette(frame, t, &palette.query, &labels, palette.index);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::test_fixture::fixture;
    use crate::app::Overlay;
    use sdeck_core::store::deck_config::{deck_config_path, read_deck_config};

    fn open(f: &crate::app::test_fixture::Fixture) -> bool {
        f.app.overlay.is_some()
    }

    #[test]
    fn question_mark_opens_config_on_the_keybindings_tab_and_closes() {
        use crate::config_fields::ConfigTab;
        let mut f = fixture();
        f.key("?");
        let Some(Overlay::Config { tab, selected, .. }) = &f.app.overlay else {
            panic!("config is open")
        };
        assert_eq!((*tab, *selected), (ConfigTab::Keybindings, 0));
        f.key("x");
        assert!(open(&f), "unrelated keys are swallowed, not passed to the list");
        f.key("?");
        assert!(!open(&f));
        f.key("?");
        f.key("\x1b");
        assert!(!open(&f));
    }

    #[test]
    fn config_navigates_toggles_edits_and_saves() {
        let mut f = fixture();
        f.key("C");
        f.key("jjjj");
        let Some(Overlay::Config { selected, .. }) = &f.app.overlay else {
            panic!("config is open")
        };
        assert_eq!(*selected, 4);
        // ui.notifications is a toggle: Enter flips it and saves.
        assert!(f.app.config.ui.notifications);
        f.key("\r");
        assert!(!f.app.config.ui.notifications);
        assert!(!read_deck_config(&deck_config_path()).ui.notifications);
        assert!(f.app.message.contains("ui.notifications updated"));

        // ui.maxSessionsListed opens a prompt over the still-open popup.
        f.key("kkkk");
        f.key("\r");
        assert_eq!(f.app.prompt.as_ref().unwrap().value, "10");
        f.key("\x15");
        f.key("12\r");
        assert_eq!(f.app.config.ui.max_sessions_listed, 12);
        assert!(open(&f));

        f.key("\r");
        f.key("\x15");
        f.key("nope\r");
        assert_eq!(f.app.config.ui.max_sessions_listed, 12);
        assert!(f.app.message.contains("Invalid value for Max sessions listed"));
        f.key("C");
        assert!(!open(&f));
    }

    #[test]
    fn config_ctl_row_toggles_saves_and_asks_for_a_restart() {
        let mut f = fixture();
        let index = crate::config_fields::CONFIG_FIELDS
            .iter()
            .position(|c| c.label == "ui.ctl")
            .unwrap();
        f.app.apply_config_field(index, "", std::time::Instant::now());
        assert!(!f.app.config.ui.ctl);
        assert!(!read_deck_config(&deck_config_path()).ui.ctl);
        assert!(f.app.message.contains("restart sdeck"), "{}", f.app.message);
    }

    #[test]
    fn config_accounts_row_toggles() {
        let mut f = fixture();
        f.add(Some("a"), true);
        f.key("C");
        f.key("\x1b[C\x1b[C\x1b[C");
        f.key("j");
        assert!(f.app.config.accounts.show_all_sessions);
        f.key("\r");
        assert!(!f.app.config.accounts.show_all_sessions);
    }

    #[test]
    fn config_tabs_switch_with_arrows_wrap_and_reset_the_selection() {
        use crate::config_fields::ConfigTab;
        let mut f = fixture();
        f.key("C");
        f.key("jj");
        f.key("\x1b[C");
        let Some(Overlay::Config { tab, selected, .. }) = &f.app.overlay else {
            panic!("config is open")
        };
        assert_eq!((*tab, *selected), (ConfigTab::Display, 0));
        f.key("\x1b[D\x1b[D");
        let Some(Overlay::Config { tab, .. }) = &f.app.overlay else {
            panic!("config is open")
        };
        assert_eq!(*tab, ConfigTab::Keybindings);
        f.key("\x1b[C");
        let Some(Overlay::Config { tab, .. }) = &f.app.overlay else {
            panic!("config is open")
        };
        assert_eq!(*tab, ConfigTab::General);
    }

    fn open_key_row(f: &mut crate::app::test_fixture::Fixture, label: &str) {
        use crate::config_fields::{tab_fields, ConfigTab, CONFIG_FIELDS};
        f.key("C\x1b[D");
        let at = tab_fields(ConfigTab::Keybindings)
            .iter()
            .position(|&i| CONFIG_FIELDS[i].label == label)
            .unwrap();
        f.key(&"j".repeat(at));
    }

    #[test]
    fn rebinding_a_row_takes_the_next_key_saves_and_remaps_the_list() {
        let mut f = fixture();
        open_key_row(&mut f, "keys.view.help");
        f.key("\r");
        let Some(Overlay::Config { capturing, .. }) = &f.app.overlay else {
            panic!("config is open")
        };
        assert!(capturing);
        f.key("y");
        assert_eq!(f.app.config.keybindings["help"], "y");
        assert_eq!(read_deck_config(&deck_config_path()).keybindings["help"], "y");
        assert!(f.app.message.contains("Help is now y"), "{}", f.app.message);
        f.key("C");
        f.key("?");
        assert!(!open(&f), "the old key no longer opens help");
        f.key("y");
        assert!(matches!(
            f.app.overlay,
            Some(Overlay::Config {
                tab: crate::config_fields::ConfigTab::Keybindings,
                ..
            })
        ));
    }

    #[test]
    fn rebinding_refuses_a_taken_key_and_backspace_restores_the_default() {
        let mut f = fixture();
        open_key_row(&mut f, "keys.view.help");
        f.key("\rj");
        assert!(f.app.config.keybindings.is_empty());
        assert!(f.app.message.contains("already Move down"), "{}", f.app.message);
        f.key("\ry");
        assert_eq!(f.app.config.keybindings["help"], "y");
        f.key("\x7f");
        assert!(f.app.config.keybindings.is_empty());
        f.key("\r\x1b");
        assert!(f.app.config.keybindings.is_empty());
        assert!(open(&f), "Esc cancels the capture, not the popup");
    }

    #[test]
    fn the_detach_key_is_a_keybindings_row() {
        let mut f = fixture();
        open_key_row(&mut f, "keys.chord.detachKey");
        f.key("\r\x05");
        assert_eq!(f.app.config.ui.detach_key, "ctrl+e");
    }

    #[test]
    fn skills_switch_tabs_and_close() {
        let mut f = fixture();
        f.key("w");
        let Some(Overlay::Skills { tab, .. }) = &f.app.overlay else {
            panic!("skills is open")
        };
        assert_eq!(*tab, crate::view::overlays::SkillsTab::Skills);
        f.key("\x1b[C");
        let Some(Overlay::Skills { tab, .. }) = &f.app.overlay else {
            panic!("skills is open")
        };
        assert_eq!(*tab, crate::view::overlays::SkillsTab::Agents);
        f.key("\x1b[C");
        let Some(Overlay::Skills { tab, .. }) = &f.app.overlay else {
            panic!("skills is open")
        };
        assert_eq!(*tab, crate::view::overlays::SkillsTab::Usage);
        f.key("w");
        assert!(!open(&f));
    }

    #[test]
    fn alerts_open_and_close() {
        let mut f = fixture();
        f.key("a");
        assert!(matches!(f.app.overlay, Some(Overlay::Alerts { .. })));
        f.key("q");
        assert!(!open(&f));
    }

    #[test]
    fn palette_filters_runs_and_ignores_a_blank_query() {
        let mut f = fixture();
        f.key(":");
        f.key("\r");
        assert!(
            !open(&f),
            "Enter on a blank query closes without running anything"
        );

        f.key(":");
        f.key("zzz");
        let Some(Overlay::Palette(p)) = &f.app.overlay else {
            panic!("palette is open")
        };
        assert_eq!(p.query, "zzz");
        f.key("\x15");
        f.key("openco");
        f.key("\r");
        assert!(matches!(f.app.overlay, Some(Overlay::Config { .. })));
        f.key("\x1b");

        f.key(":");
        f.key("choose");
        f.key("\r");
        assert!(
            f.app.picker.is_some(),
            "the chosen command ran after the palette closed"
        );
    }
}
