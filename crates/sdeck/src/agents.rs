//! Local Claude Code subagents (`<config dir>/agents`), port of `agents.ts`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::frontmatter::parse_name_and_description;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalAgent {
    pub name: String,
    pub description: String,
}

pub fn agents_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("agents")
}

/// Every personal subagent directly under `agents_dir` (a real directory or a symlink), identified
/// by the one `*.agent.md` file directly inside it. A folder without one, or with several, is skipped.
pub fn discover_local_agents(agents_dir: &Path) -> Vec<LocalAgent> {
    let Ok(entries) = fs::read_dir(agents_dir) else {
        return Vec::new();
    };
    let mut agents: Vec<LocalAgent> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let files: Vec<PathBuf> = fs::read_dir(entry.path())
                .ok()?
                .filter_map(Result::ok)
                .map(|f| f.path())
                .filter(|p| {
                    p.file_name()
                        .is_some_and(|n| n.to_string_lossy().ends_with(".agent.md"))
                })
                .collect();
            let [file] = files.as_slice() else {
                return None;
            };
            let meta = parse_name_and_description(&fs::read_to_string(file).ok()?);
            Some(LocalAgent {
                name: meta
                    .name
                    .unwrap_or_else(|| entry.file_name().to_string_lossy().into_owned()),
                description: meta.description.unwrap_or_default(),
            })
        })
        .collect();
    agents.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.name.cmp(&b.name))
    });
    agents
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_directory_gives_no_agents() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(discover_local_agents(&tmp.path().join("nope")).is_empty());
    }

    #[test]
    fn reads_every_folder_with_exactly_one_agent_file_and_skips_the_rest() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let put = |folder: &str, file: &str, body: &str| {
            fs::create_dir_all(dir.join(folder)).unwrap();
            fs::write(dir.join(folder).join(file), body).unwrap();
        };
        put(
            "mmp-pr-preflight",
            "mmp-pr-preflight.agent.md",
            "---\nname: mmp-pr-preflight\ndescription: Runs CI checks locally.\n---\n",
        );
        put(
            "code-reviewer",
            "code-reviewer.agent.md",
            "---\nname: code-reviewer\ndescription: Reviews a diff.\n---\n",
        );
        put("two", "a.agent.md", "---\nname: a\n---\n");
        put("two", "b.agent.md", "---\nname: b\n---\n");
        fs::create_dir_all(dir.join("no-agent-file")).unwrap();
        fs::write(dir.join("stray.agent.md"), "---\nname: stray\n---\n").unwrap();

        assert_eq!(
            discover_local_agents(dir),
            vec![
                LocalAgent {
                    name: "code-reviewer".into(),
                    description: "Reviews a diff.".into()
                },
                LocalAgent {
                    name: "mmp-pr-preflight".into(),
                    description: "Runs CI checks locally.".into()
                },
            ]
        );
    }
}
