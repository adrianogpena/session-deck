//! The footer text input (`prompt`) and the centered list picker, with their keys. Port of
//! `App.onPromptKey` and `onPickerKey`; what a submit or a pick does lives with each feature.

use std::time::Instant;

use super::App;

/// What Enter does with the typed text.
pub(super) enum PromptAction {
    AddProject,
    SendPrompt(u64),
    RenameSession(u64),
    RenameFolder(String),
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
            "\x1b" | "q" | "\x03" => self.picker = None,
            _ => {}
        }
        self.dirty = true;
    }

    fn pick(&mut self, picker: Picker, now: Instant) {
        match picker.action {
            PickerAction::SetActiveAgent(ids) => self.set_active_agent(&ids[picker.index], now),
            PickerAction::NewSession(ids) => self.new_session(&ids[picker.index], now),
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
