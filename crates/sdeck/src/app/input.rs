//! The footer text input (`prompt`) and the centered list picker, with their keys. Port of
//! `App.onPromptKey` and `onPickerKey`; what a submit or a pick does lives with each feature.

use std::path::PathBuf;
use std::time::Instant;

use sdeck_core::status::account::Account;
use sdeck_core::store::tree_prefs::delete_folder;

use super::App;

/// What Enter does with the typed text.
pub(super) enum PromptAction {
    AddProject,
    SendPrompt(u64),
    RenameSession(u64),
    RenameFolder(String),
    /// Creates the folder, moving these projects into it when given.
    NewFolder(Vec<String>),
    SetTags(String),
    AddTags(Vec<String>),
    /// A config popup row, by its index in `CONFIG_FIELDS`.
    ConfigField(usize),
    /// The folder name of a new Claude account.
    NewAccountDir,
}

pub(super) struct TextPrompt {
    pub label: String,
    pub value: String,
    pub action: PromptAction,
}

/// What Enter does with the chosen entry; the ids are the agents the picker lists, in order.
pub(super) enum PickerAction {
    SetActiveAgent(Vec<String>),
    NewSession(Vec<String>),
    /// Session ids of the trash entries listed, in order.
    RestoreFromTrash(Vec<String>),
    /// Entry 0 is the top level, then the folders (ids, in order), then "+ New folder…".
    MoveToFolder {
        project_keys: Vec<String>,
        folder_ids: Vec<String>,
        bulk: bool,
    },
    /// The accounts listed, in order, then "+ Add account…".
    SwitchAccount(Vec<Account>),
    /// Space checks entries (`SHAREABLE` order); Enter creates `dir` linking the checked ones to `source`.
    ShareWithAccount {
        dir: PathBuf,
        source: PathBuf,
        checked: Vec<bool>,
    },
    /// The accounts listed, in order; Enter opens `AccountActions` for the chosen one.
    ManageAccounts(Vec<Account>),
    /// "Edit shared entries…", "Delete account…".
    AccountActions(Account),
    /// Space checks entries (`SHAREABLE` order); Enter makes `dir`'s symlinks to `source` match.
    EditAccountLinks {
        dir: PathBuf,
        source: PathBuf,
        checked: Vec<bool>,
    },
}

/// What `y` does.
pub(super) enum ConfirmAction {
    DeleteAccount(PathBuf),
    RemoveProject(String),
    DeleteFolder(String),
    RemoveTag(String),
}

pub(super) struct Confirm {
    pub question: String,
    pub action: ConfirmAction,
}

pub(super) struct Picker {
    pub title: String,
    pub items: Vec<String>,
    pub index: usize,
    pub action: PickerAction,
}

impl App {
    pub(super) fn open_prompt(&mut self, label: String, value: String, action: PromptAction) {
        self.prompt = Some(TextPrompt { label, value, action });
        self.dirty = true;
    }

    pub(super) fn on_prompt_key(&mut self, key: &str, now: Instant) {
        let Some(prompt) = self.prompt.as_mut() else {
            return;
        };
        match key {
            "\r" => {
                if let Some(TextPrompt { value, action, .. }) = self.prompt.take() {
                    self.submit_prompt(action, value, now);
                }
            }
            "\x1b" | "\x03" => self.prompt = None,
            "\x7f" | "\x08" => {
                prompt.value.pop();
            }
            "\x15" => prompt.value.clear(),
            _ if !key.starts_with('\x1b') => {
                prompt.value.extend(key.chars().filter(|c| !c.is_control()));
            }
            _ => {}
        }
        self.dirty = true;
    }

    fn submit_prompt(&mut self, action: PromptAction, value: String, now: Instant) {
        match action {
            PromptAction::AddProject => self.add_project(&value, now),
            PromptAction::SendPrompt(uid) => self.send_prompt(uid, &value, now),
            PromptAction::RenameSession(uid) => self.rename_session(uid, &value, now),
            PromptAction::RenameFolder(id) => self.rename_folder(&id, &value),
            PromptAction::NewFolder(keys) => self.new_folder(&value, &keys, now),
            PromptAction::SetTags(id) => self.set_session_tags(&id, &value, now),
            PromptAction::AddTags(ids) => self.add_tags_to_sessions(&ids, &value, now),
            PromptAction::ConfigField(index) => self.apply_config_field(index, &value, now),
            PromptAction::NewAccountDir => self.new_account_dir(&value, now),
        }
    }

    pub(super) fn on_picker_key(&mut self, key: &str, now: Instant) {
        let Some(picker) = self.picker.as_mut() else {
            return;
        };
        match key {
            "\x1b[A" | "\x1bOA" | "k" => picker.index = picker.index.saturating_sub(1),
            "\x1b[B" | "\x1bOB" | "j" => picker.index = (picker.index + 1).min(picker.items.len() - 1),
            "\r" => {
                if let Some(picker) = self.picker.take() {
                    self.pick(picker, now);
                }
            }
            "\x1b" | "q" | "\x03" => {
                let back = self.picker.take().map(|p| p.action);
                if key != "\x03" {
                    self.go_back_from(back, now);
                }
            }
            " " => {
                if let PickerAction::ShareWithAccount { checked, .. }
                | PickerAction::EditAccountLinks { checked, .. } = &mut picker.action
                {
                    let on = !checked[picker.index];
                    checked[picker.index] = on;
                    let label = &picker.items[picker.index][4..];
                    picker.items[picker.index] = format!("[{}] {label}", if on { 'x' } else { ' ' });
                }
            }
            _ => {}
        }
        self.dirty = true;
    }

    /// Esc in an account submenu returns to the menu it came from.
    fn go_back_from(&mut self, action: Option<PickerAction>, now: Instant) {
        match action {
            Some(PickerAction::ManageAccounts(_)) => self.open_account_picker(),
            Some(PickerAction::AccountActions(_)) => self.open_manage_accounts(),
            Some(PickerAction::EditAccountLinks { dir, .. }) => {
                if let Some(account) = self.accounts.iter().find(|a| a.config_dir == dir).cloned() {
                    self.open_account_actions(account, now);
                }
            }
            _ => {}
        }
    }

    pub(super) fn on_confirm_key(&mut self, key: &str, now: Instant) {
        if let Some(Confirm { action, .. }) = self.confirm.take() {
            if key == "y" || key == "Y" {
                match action {
                    ConfirmAction::DeleteAccount(dir) => self.delete_account(&dir, now),
                    ConfirmAction::RemoveProject(key) => self.remove_project_now(&key, now),
                    ConfirmAction::DeleteFolder(id) => self.change_tree(|t| delete_folder(&t, &id)),
                    ConfirmAction::RemoveTag(name) => self.remove_tag_everywhere(&name, now),
                }
            }
        }
        self.dirty = true;
    }

    fn pick(&mut self, picker: Picker, now: Instant) {
        match picker.action {
            PickerAction::SetActiveAgent(ids) => self.set_active_agent(&ids[picker.index], now),
            PickerAction::NewSession(ids) => self.new_session(&ids[picker.index], now),
            PickerAction::RestoreFromTrash(ids) => self.restore_from_trash(&ids[picker.index], now),
            PickerAction::MoveToFolder {
                project_keys,
                folder_ids,
                bulk,
            } => self.move_to_folder(picker.index, project_keys, &folder_ids, bulk),
            PickerAction::SwitchAccount(accounts) => match accounts.get(picker.index) {
                Some(account) => self.switch_account(account, now),
                None if picker.index == accounts.len() => self.open_prompt(
                    "New account folder (e.g. claude-personal)".into(),
                    String::new(),
                    PromptAction::NewAccountDir,
                ),
                None => self.open_manage_accounts(),
            },
            PickerAction::ManageAccounts(accounts) => {
                self.open_account_actions(accounts[picker.index].clone(), now)
            }
            PickerAction::AccountActions(account) => match picker.index {
                0 => self.open_edit_account_links(&account, now),
                _ => self.ask_delete_account(&account, now),
            },
            PickerAction::EditAccountLinks { dir, source, checked } => {
                self.apply_account_links(&dir, &source, &checked, now)
            }
            PickerAction::ShareWithAccount { dir, source, checked } => {
                self.create_account(&dir, &source, &checked, now)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::test_fixture::fixture;

    #[test]
    fn typing_editing_and_cancelling_a_prompt() {
        let mut f = fixture();
        f.key("p");
        assert!(f.app.prompt.is_some());
        f.key("ab\x7fc\x1b[A");
        assert_eq!(f.app.prompt.as_ref().unwrap().value, "ac");
        f.key("\x15");
        assert_eq!(f.app.prompt.as_ref().unwrap().value, "");
        f.key("xyz");
        f.key("\x1b");
        assert!(f.app.prompt.is_none());
    }
}
