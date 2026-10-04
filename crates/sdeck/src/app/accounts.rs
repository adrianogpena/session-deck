//! `F4`: which logged-in Claude account new sessions launch as. New in the Rust version.

use std::path::{Path, PathBuf};
use std::time::Instant;

use sdeck_core::paths;
use sdeck_core::status::account::{
    account_dir_for_name, create_account_dir, discover_accounts, Account, SHAREABLE,
};
use sdeck_core::status::statusline::{enable_statusline_for_all, statusline_command, StatusLineOutcome};
use sdeck_core::store::deck_config::{deck_config_path, write_deck_config};

use super::input::{Picker, PickerAction};
use super::App;

fn label(account: &Account) -> String {
    account
        .email
        .clone()
        .unwrap_or_else(|| account.config_dir.to_string_lossy().into_owned())
}

impl App {
    /// Adds the usage statusLine to each config dir without one, and says what happened in a short phrase.
    pub(super) fn enable_usage_statusline(dirs: &[PathBuf]) -> String {
        let Ok(exe) = std::env::current_exe() else {
            return "statusLine not added (executable path unknown)".into();
        };
        let outcomes = enable_statusline_for_all(dirs, &statusline_command(&exe));
        let count = |wanted| outcomes.iter().filter(|o| **o == wanted).count();
        let mut parts = vec![format!("statusLine added to {}", count(StatusLineOutcome::Added))];
        for (outcome, text) in [
            (StatusLineOutcome::AlreadySet, "already set"),
            (StatusLineOutcome::Unreadable, "settings.json unreadable"),
            (StatusLineOutcome::Failed, "failed"),
        ] {
            if count(outcome) > 0 {
                parts.push(format!("{} {text}", count(outcome)));
            }
        }
        parts.join(", ")
    }

    /// `F4`: the accounts (a `*` marks the one new sessions launch as) and "+ Add account…". Accounts
    /// are rediscovered first, so one logged in since startup is listed.
    pub(super) fn open_account_picker(&mut self) {
        for found in discover_accounts() {
            match self
                .accounts
                .iter_mut()
                .find(|a| a.config_dir == found.config_dir)
            {
                Some(known) => known.email = found.email.or(known.email.take()),
                None => self.accounts.push(found),
            }
        }
        let active = self.active_account().map(|a| a.config_dir.clone());
        let mut items: Vec<String> = self
            .accounts
            .iter()
            .map(|a| {
                format!(
                    "{} {}",
                    if Some(&a.config_dir) == active.as_ref() {
                        '*'
                    } else {
                        ' '
                    },
                    label(a)
                )
            })
            .collect();
        items.push("+ Add account…".into());
        let index = self
            .accounts
            .iter()
            .position(|a| Some(&a.config_dir) == active.as_ref())
            .unwrap_or(0);
        self.picker = Some(Picker {
            title: "Switch account".into(),
            items,
            index,
            action: PickerAction::SwitchAccount(self.accounts.clone()),
        });
        self.dirty = true;
    }

    /// Enter on the folder-name prompt: asks which of the default account's entries to share.
    pub(super) fn new_account_dir(&mut self, input: &str, now: Instant) {
        let dir = match account_dir_for_name(input) {
            Ok(dir) => dir,
            Err(message) => return self.flash(message, now),
        };
        if self.accounts.iter().any(|a| a.config_dir == dir) {
            return self.flash(format!("{} is already an account", dir.display()), now);
        }
        let source = self
            .accounts
            .iter()
            .find(|a| a.is_default)
            .map_or_else(paths::claude_dir, |a| a.config_dir.clone());
        self.picker = Some(Picker {
            title: format!("Share with {} · Space checks, Enter creates", dir.display()),
            items: SHAREABLE
                .iter()
                .map(|(_, label, on)| format!("[{}] {label}", if *on { 'x' } else { ' ' }))
                .collect(),
            index: 0,
            action: PickerAction::ShareWithAccount {
                dir,
                source,
                checked: SHAREABLE.iter().map(|(_, _, on)| *on).collect(),
            },
        });
        self.dirty = true;
    }

    pub(super) fn create_account(&mut self, dir: &Path, source: &Path, checked: &[bool], now: Instant) {
        let entries: Vec<&str> = SHAREABLE
            .iter()
            .zip(checked)
            .filter(|(_, on)| **on)
            .map(|((entry, _, _), _)| *entry)
            .collect();
        match create_account_dir(dir, source, &entries) {
            Ok(linked) => {
                let mut message = format!("Created {} ({} linked)", dir.display(), linked.len());
                if self.config.ui.show_usage {
                    let usage = Self::enable_usage_statusline(&[dir.to_path_buf()]);
                    message.push_str(&format!(" ({usage})"));
                }
                let account = Account {
                    config_dir: dir.to_path_buf(),
                    email: None,
                    is_default: false,
                };
                if !self.accounts.iter().any(|a| a.config_dir == account.config_dir) {
                    self.accounts.push(account.clone());
                }
                if self.selected_project().is_some() {
                    self.switch_account(&account, now);
                    self.new_session("claude", now);
                    self.flash(format!("{message}. Run /login here; it then shows in F4"), now);
                } else {
                    self.flash(
                        format!("{message}. Select a project and press n after F4 to log in"),
                        now,
                    );
                }
            }
            Err(message) => self.flash(message, now),
        }
    }

    /// Persists the choice (unset for the default account) and rebuilds the tree, whose
    /// other-account tags, hidden sessions and usage rows all follow the active account.
    pub(super) fn switch_account(&mut self, account: &Account, now: Instant) {
        self.config.ui.active_account_config_dir =
            (!account.is_default).then(|| account.config_dir.to_string_lossy().into_owned());
        let _ = write_deck_config(&self.config, &deck_config_path());
        self.rebuild_rows();
        self.flash(format!("New sessions will launch as {}", label(account)), now);
    }
}

#[cfg(test)]
mod tests {
    use sdeck_core::status::account::Account;
    use sdeck_core::store::deck_config::{deck_config_path, read_deck_config};

    use crate::app::test_fixture::{fixture, Fixture};
    use crate::event::AppEvent;
    use std::time::Instant;

    fn two_accounts(f: &mut Fixture) -> (Account, Account) {
        let work = Account {
            config_dir: f.home.path().join(".claude"),
            email: Some("work@x.com".into()),
            is_default: true,
        };
        let me = Account {
            config_dir: f.home.path().join(".claude-me"),
            email: Some("me@x.com".into()),
            is_default: false,
        };
        f.app.accounts = vec![work.clone(), me.clone()];
        (work, me)
    }

    #[test]
    fn turning_on_ui_show_usage_adds_the_status_line_to_every_account_without_one() {
        let mut f = fixture();
        let (work, me) = two_accounts(&mut f);
        std::fs::create_dir_all(&me.config_dir).unwrap();
        std::fs::create_dir_all(&work.config_dir).unwrap();
        let own = r#"{"statusLine":{"type":"command","command":"mine.sh"}}"#;
        std::fs::write(work.config_dir.join("settings.json"), own).unwrap();
        let index = crate::config_fields::CONFIG_FIELDS
            .iter()
            .position(|c| c.label == "ui.showUsage")
            .unwrap();
        assert!(!f.app.config.ui.show_usage);

        f.app.apply_config_field(index, "", Instant::now());

        assert!(f.app.config.ui.show_usage);
        let added = std::fs::read_to_string(me.config_dir.join("settings.json")).unwrap();
        assert!(added.contains("statusline-hook"), "{added}");
        assert_eq!(
            std::fs::read_to_string(work.config_dir.join("settings.json")).unwrap(),
            own
        );
        assert!(
            f.app.message.contains("statusLine added to 1"),
            "{}",
            f.app.message
        );

        f.app.apply_config_field(index, "", Instant::now());
        assert!(!f.app.message.contains("statusLine"), "{}", f.app.message);
    }

    #[test]
    fn f4_persists_the_account_and_new_sessions_launch_as_it() {
        let mut f = fixture();
        let (_, me) = two_accounts(&mut f);
        f.add(Some("abc"), true);

        f.key("\x1bOS");
        f.key("j\r");
        assert_eq!(f.app.message, "New sessions will launch as me@x.com");
        let saved = read_deck_config(&deck_config_path());
        assert_eq!(
            saved.ui.active_account_config_dir.as_deref(),
            Some(me.config_dir.to_string_lossy().as_ref())
        );
        f.key("n");
        let fresh = f.app.attached.expect("attached");
        f.wait_for_screen(fresh, &format!("CLAUDE_CONFIG_DIR={}", me.config_dir.display()));
        f.key("\x0bq");

        f.key("\x1bOS");
        f.key("k\r");
        assert_eq!(f.app.message, "New sessions will launch as work@x.com");
        assert_eq!(
            read_deck_config(&deck_config_path()).ui.active_account_config_dir,
            None
        );
        f.key("n");
        let fresh = f.app.attached.expect("attached");
        f.wait_for_screen(fresh, "CLAUDE_CONFIG_DIR=unset");
    }

    #[test]
    fn f4_with_one_account_lists_it_and_add_account() {
        let mut f = fixture();
        f.key("\x1bOS");
        let picker = f.app.picker.as_ref().expect("picker");
        assert_eq!(picker.title, "Switch account");
        assert_eq!(picker.items.len(), 2);
        assert_eq!(picker.items[1], "+ Add account…");
        assert_eq!(f.app.config.ui.active_account_config_dir, None);
    }

    #[test]
    fn adding_an_account_asks_for_the_folder_then_what_to_share() {
        let mut f = fixture();
        std::fs::create_dir_all(f.home.path().join(".claude/skills")).unwrap();
        f.key("\x1bOS");
        f.key("j\r");
        assert!(f.app.prompt.is_some());
        f.key("work\r");
        let picker = f.app.picker.as_ref().expect("share picker");
        assert!(picker.items[0].starts_with("[x] skills"));
        assert!(picker.items[4].starts_with("[ ] settings.json"));
        f.key(" j ");
        assert!(f.app.picker.as_ref().unwrap().items[0].starts_with("[ ] skills"));
        f.key("\r");
        assert!(f.app.picker.is_none());
        assert!(f.home.path().join(".claude-work").is_dir());
        assert!(f.app.message.starts_with("Created"));
    }

    #[test]
    fn adding_an_account_with_a_project_selected_opens_claude_as_it_to_log_in() {
        let mut f = fixture();
        f.add(Some("abc"), true);
        f.key("\x1bOS");
        f.key("j\r");
        f.key("work\r");
        f.key("\r");
        let dir = f.home.path().join(".claude-work");
        assert!(dir.is_dir());
        assert!(f.app.message.contains("/login"), "{}", f.app.message);
        let fresh = f.app.attached.expect("attached");
        f.wait_for_screen(fresh, &format!("CLAUDE_CONFIG_DIR={}", dir.display()));
    }

    #[test]
    fn a_toast_click_on_a_hidden_accounts_session_switches_account_and_selects_it() {
        let mut f = fixture();
        let (_, me) = two_accounts(&mut f);
        f.app.config.accounts.show_all_sessions = false;
        let mine = f.add(Some("mine"), true);
        let theirs = f.add(Some("theirs"), true);
        f.app.session_mut(theirs).unwrap().account = Some(me.clone());
        f.app.session_mut(mine).unwrap().account = Some(f.app.accounts[0].clone());
        f.app.rebuild_rows();
        assert!(f.app.rows.iter().all(|r| !matches!(
            r,
            crate::tree::TreeRow::Session { uid, .. } if *uid == theirs
        )));

        f.app
            .handle(AppEvent::ToastClicked("theirs".into()), Instant::now());
        assert_eq!(
            f.app.active_account().map(|a| &a.config_dir),
            Some(&me.config_dir)
        );
        assert_eq!(f.app.selected_session().map(|s| s.uid), Some(theirs));
    }
}
