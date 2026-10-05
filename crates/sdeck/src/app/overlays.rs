//! The popups opened by `?`, `C`, `w`, `a` and `:` — one at a time, and while one is open it takes
//! every key (a footer prompt, opened from the config popup, takes them first). Port of the
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
use super::{App, VERSION};
use crate::agents::{agents_dir, discover_local_agents, LocalAgent};
use crate::commands::{palette_matches, CommandId, PALETTE_COMMANDS};
use crate::config_fields::{ConfigFieldKind, CONFIG_FIELDS};
use crate::skills::{claude_settings_path, discover_local_skills, skills_dir, LocalSkill};
use crate::theme::Theme;
use crate::view::overlays::AccountUsage;
use crate::view::overlays::{
    render_alerts, render_config, render_help, render_palette, render_search, render_skills, render_trace,
    SearchResultRow, SkillsTab,
};

pub(super) struct PaletteState {
    pub query: String,
    pub index: usize,
}

pub(super) enum Overlay {
    Help {
        scroll: Cell<usize>,
    },
    Config {
        selected: usize,
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
    pub(super) fn open_help(&mut self) {
        self.overlay = Some(Overlay::Help { scroll: Cell::new(0) });
    }

    pub(super) fn open_config(&mut self) {
        self.overlay = Some(Overlay::Config { selected: 0 });
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
            Some(Overlay::Help { .. }) => self.on_help_key(key),
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

    fn on_help_key(&mut self, key: &str) {
        match (&self.overlay, key) {
            (_, "\x1b" | "?" | "q") => self.overlay = None,
            (Some(Overlay::Help { scroll }), _) => Self::scroll_by_key(scroll, key),
            _ => {}
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
        let Some(Overlay::Config { selected }) = &mut self.overlay else {
            return;
        };
        match key {
            "\r" => {
                let index = *selected;
                self.edit_config_field(index, now);
            }
            "\x1b" | "C" | "q" => self.overlay = None,
            _ => *selected = step(*selected, key, CONFIG_FIELDS.len()),
        }
    }

    /// Toggles flip right away; the rest open a prompt pre-filled with their current value.
    fn edit_config_field(&mut self, index: usize, now: Instant) {
        let field = &CONFIG_FIELDS[index];
        if field.kind == ConfigFieldKind::Toggle {
            self.apply_config_field(index, "", now);
        } else {
            let value = field.edit_value(&self.config);
            self.open_prompt(
                field.label.clone(),
                value,
                super::input::PromptAction::ConfigField(index),
            );
        }
    }

    pub(super) fn apply_config_field(&mut self, index: usize, input: &str, now: Instant) {
        let field = &CONFIG_FIELDS[index];
        let Some(next) = field.apply(&self.config, input) else {
            self.flash(format!("Invalid value for {}", field.label), now);
            return;
        };
        let usage_turned_on = !self.config.ui.show_usage && next.ui.show_usage;
        self.config = next;
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
        }
        let mut message = format!("{} updated", field.label);
        if usage_turned_on {
            let mut dirs: Vec<PathBuf> = discover_accounts().into_iter().map(|a| a.config_dir).collect();
            for known in &self.accounts {
                if !dirs.contains(&known.config_dir) {
                    dirs.push(known.config_dir.clone());
                }
            }
            message.push_str(&format!(" ({})", Self::enable_usage_statusline(&dirs)));
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
            CommandId::ToggleSidebar => self.toggle_sidebar(),
            CommandId::OpenTrash => self.open_trash_picker(now),
        }
    }

    pub(super) fn draw_overlay(&self, frame: &mut Frame, t: Theme) {
        match &self.overlay {
            None => {}
            Some(Overlay::Help { scroll }) => render_help(frame, t, scroll, VERSION),
            Some(Overlay::Config { selected }) => render_config(
                frame,
                t,
                &self.config,
                &deck_config_path().to_string_lossy(),
                *selected,
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
    fn question_mark_opens_help_that_scrolls_and_closes() {
        let mut f = fixture();
        f.key("?");
        assert!(matches!(f.app.overlay, Some(Overlay::Help { .. })));
        f.key("jjk");
        let Some(Overlay::Help { scroll }) = &f.app.overlay else {
            panic!("help is open")
        };
        assert_eq!(scroll.get(), 1);
        f.key("x");
        assert!(open(&f), "unrelated keys are swallowed, not passed to the list");
        f.key("\x1b");
        assert!(!open(&f));
        f.key("?");
        f.key("?");
        assert!(!open(&f));
    }

    #[test]
    fn config_navigates_toggles_edits_and_saves() {
        let mut f = fixture();
        f.key("C");
        f.key("j");
        let Some(Overlay::Config { selected }) = &f.app.overlay else {
            panic!("config is open")
        };
        assert_eq!(*selected, 1);
        // ui.notifications is a toggle: Enter flips it and saves.
        assert!(f.app.config.ui.notifications);
        f.key("\r");
        assert!(!f.app.config.ui.notifications);
        assert!(!read_deck_config(&deck_config_path()).ui.notifications);
        assert!(f.app.message.contains("ui.notifications updated"));

        // ui.maxSessionsListed opens a prompt over the still-open popup.
        f.key("k");
        f.key("\r");
        assert_eq!(f.app.prompt.as_ref().unwrap().value, "30");
        f.key("\x15");
        f.key("12\r");
        assert_eq!(f.app.config.ui.max_sessions_listed, 12);
        assert!(open(&f));

        f.key("\r");
        f.key("\x15");
        f.key("nope\r");
        assert_eq!(f.app.config.ui.max_sessions_listed, 12);
        assert!(f.app.message.contains("Invalid value for ui.maxSessionsListed"));
        f.key("C");
        assert!(!open(&f));
    }

    #[test]
    fn config_accounts_row_toggles() {
        let mut f = fixture();
        f.add(Some("a"), true);
        f.key("C");
        let last = crate::config_fields::CONFIG_FIELDS.len() - 2;
        for _ in 0..last {
            f.key("j");
        }
        assert!(f.app.config.accounts.show_all_sessions);
        f.key("\r");
        assert!(!f.app.config.accounts.show_all_sessions);
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
