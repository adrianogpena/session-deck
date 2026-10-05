//! Local Claude Code skills (`~/.claude/skills`) and their `skillOverrides` state, port of
//! `skills.ts`.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::frontmatter::{frontmatter_flag, parse_name_and_description};

/// The four states Claude Code's `skillOverrides` setting can put a skill in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillState {
    /// Visible to the model and auto-triggerable (the default with no override and no
    /// `disable-model-invocation: true` in the skill's frontmatter).
    On,
    /// Name visible, no description: no context cost, but it can't fire on its own.
    NameOnly,
    /// Hidden from the model's context, still reachable by typing its name in `/`.
    UserInvocableOnly,
    /// Removed entirely, even from `/`.
    Off,
}

impl SkillState {
    pub const ALL: [SkillState; 4] = [Self::On, Self::NameOnly, Self::UserInvocableOnly, Self::Off];

    fn parse(s: &str) -> Option<Self> {
        match s {
            "on" => Some(Self::On),
            "name-only" => Some(Self::NameOnly),
            "user-invocable-only" => Some(Self::UserInvocableOnly),
            "off" => Some(Self::Off),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::On => "ON — visible + auto-triggerable",
            Self::NameOnly => "NAME-ONLY — name visible, no description",
            Self::UserInvocableOnly => "USER-INVOCABLE-ONLY — hidden from context, still in the / menu",
            Self::Off => "OFF — removed entirely, even from /",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalSkill {
    pub name: String,
    pub description: String,
    pub state: SkillState,
}

pub fn skills_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("skills")
}

pub fn claude_settings_path(config_dir: &Path) -> PathBuf {
    config_dir.join("settings.json")
}

/// `skillOverrides` from `~/.claude/settings.json`: empty if the file is missing, unreadable, or
/// has none. Unknown state values are dropped.
pub fn read_skill_overrides(settings_path: &Path) -> HashMap<String, SkillState> {
    let overrides = fs::read_to_string(settings_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|root| root.get("skillOverrides").cloned());
    let Some(Value::Object(overrides)) = overrides else {
        return HashMap::new();
    };
    overrides
        .into_iter()
        .filter_map(|(name, value)| Some((name, SkillState::parse(value.as_str()?)?)))
        .collect()
}

/// Every personal skill directly under `skills_dir` (a real directory or a symlink), each with its
/// effective state. An entry without its own `SKILL.md` (e.g. a nested bundle) is skipped.
pub fn discover_local_skills(skills_dir: &Path, settings_path: &Path) -> Vec<LocalSkill> {
    let Ok(entries) = fs::read_dir(skills_dir) else {
        return Vec::new();
    };
    let overrides = read_skill_overrides(settings_path);
    let mut skills: Vec<LocalSkill> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let text = fs::read_to_string(entry.path().join("SKILL.md")).ok()?;
            let meta = parse_name_and_description(&text);
            let name = meta
                .name
                .unwrap_or_else(|| entry.file_name().to_string_lossy().into_owned());
            let default_state = if frontmatter_flag(&text, "disable-model-invocation") {
                SkillState::UserInvocableOnly
            } else {
                SkillState::On
            };
            let state = overrides.get(&name).copied().unwrap_or(default_state);
            Some(LocalSkill {
                name,
                description: meta.description.unwrap_or_default(),
                state,
            })
        })
        .collect();
    skills.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.name.cmp(&b.name))
    });
    skills
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(dir: &Path, name: &str, body: &str) {
        fs::create_dir_all(dir.join(name)).unwrap();
        fs::write(dir.join(name).join("SKILL.md"), body).unwrap();
    }

    #[test]
    fn overrides_are_empty_for_a_missing_malformed_or_unrelated_file_and_keep_only_known_states() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(read_skill_overrides(&tmp.path().join("missing.json")).is_empty());
        let file = tmp.path().join("settings.json");
        fs::write(&file, "{ not json").unwrap();
        assert!(read_skill_overrides(&file).is_empty());
        fs::write(&file, r#"{"theme":"dark"}"#).unwrap();
        assert!(read_skill_overrides(&file).is_empty());
        fs::write(
            &file,
            r#"{"skillOverrides":{"mermaid-diagrams":"user-invocable-only","bogus":"nope"}}"#,
        )
        .unwrap();
        let overrides = read_skill_overrides(&file);
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides["mermaid-diagrams"], SkillState::UserInvocableOnly);
    }

    #[test]
    fn discovery_reads_skills_skips_entries_without_skill_md_and_applies_overrides() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("skills");
        assert!(discover_local_skills(&dir, &tmp.path().join("s.json")).is_empty());
        skill(
            &dir,
            "plan-review",
            "---\nname: plan-review\ndescription: Review a plan.\n---\n",
        );
        skill(
            &dir,
            "mermaid-diagrams",
            "---\nname: mermaid-diagrams\ndescription: Draw diagrams.\n---\n",
        );
        fs::create_dir_all(dir.join("synced")).unwrap();
        let settings = tmp.path().join("settings.json");
        fs::write(
            &settings,
            r#"{"skillOverrides":{"mermaid-diagrams":"user-invocable-only"}}"#,
        )
        .unwrap();

        let skills = discover_local_skills(&dir, &settings);
        assert_eq!(
            skills,
            vec![
                LocalSkill {
                    name: "mermaid-diagrams".into(),
                    description: "Draw diagrams.".into(),
                    state: SkillState::UserInvocableOnly,
                },
                LocalSkill {
                    name: "plan-review".into(),
                    description: "Review a plan.".into(),
                    state: SkillState::On,
                },
            ]
        );
    }

    #[test]
    fn disable_model_invocation_in_frontmatter_means_user_invocable_only_unless_overridden() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("skills");
        skill(
            &dir,
            "batch-plan",
            "---\nname: batch-plan\ndescription: Plan.\ndisable-model-invocation: true\n---\n",
        );
        skill(
            &dir,
            "forced-on",
            "---\nname: forced-on\ndescription: X.\ndisable-model-invocation: true\n---\n",
        );
        let settings = tmp.path().join("settings.json");
        fs::write(&settings, r#"{"skillOverrides":{"forced-on":"on"}}"#).unwrap();

        let states: Vec<_> = discover_local_skills(&dir, &settings)
            .into_iter()
            .map(|s| (s.name, s.state))
            .collect();
        assert_eq!(
            states,
            vec![
                ("batch-plan".to_string(), SkillState::UserInvocableOnly),
                ("forced-on".to_string(), SkillState::On),
            ]
        );
    }

    #[test]
    fn a_skill_without_frontmatter_takes_its_folder_name() {
        let tmp = tempfile::tempdir().unwrap();
        skill(tmp.path(), "bare", "# Just a heading");
        let skills = discover_local_skills(tmp.path(), &tmp.path().join("none.json"));
        assert_eq!(
            (skills[0].name.as_str(), skills[0].description.as_str()),
            ("bare", "")
        );
    }
}
