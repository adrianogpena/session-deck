//! `F4`: which logged-in Claude account new sessions launch as. New in the Rust version.

use std::time::Instant;

use sdeck_core::status::account::Account;
use sdeck_core::store::deck_config::{deck_config_path, write_deck_config};

use super::App;

fn label(account: &Account) -> String {
    account
        .email
        .clone()
        .unwrap_or_else(|| account.config_dir.to_string_lossy().into_owned())
}

impl App {
    pub(super) fn cycle_account(&mut self, now: Instant) {
        if self.accounts.len() < 2 {
            self.flash("Only one account configured".into(), now);
            return;
        }
        let active = self.active_account().map(|a| a.config_dir.clone());
        let at = self
            .accounts
            .iter()
            .position(|a| Some(&a.config_dir) == active.as_ref())
            .unwrap_or(0);
        let next = self.accounts[(at + 1) % self.accounts.len()].clone();
        self.switch_account(&next, now);
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
    fn f4_persists_the_account_and_new_sessions_launch_as_it() {
        let mut f = fixture();
        let (_, me) = two_accounts(&mut f);
        f.add(Some("abc"), true);

        f.key("\x1bOS");
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
    fn f4_with_one_account_only_flashes() {
        let mut f = fixture();
        f.key("\x1bOS");
        assert_eq!(f.app.message, "Only one account configured");
        assert_eq!(f.app.config.ui.active_account_config_dir, None);
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
