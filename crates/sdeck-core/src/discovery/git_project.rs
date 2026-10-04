use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use super::path_utils::normalize_fs_path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRoot {
    /// Absolute path git (or the raw cwd, if it's not a git repo) considers this project's root.
    pub root: String,
    pub is_git_repo: bool,
}

const GIT_TIMEOUT: Duration = Duration::from_secs(5);

/// Runs `git` with `args`, returning stdout on a zero exit. `None` for a missing git, a non-zero
/// exit or a timeout, which callers treat as "no git".
pub(crate) fn run_git(args: &[&str]) -> Option<String> {
    let mut command = Command::new("git");
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = command.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = thread::spawn(move || {
        let mut out = Vec::new();
        let _ = stdout.read_to_end(&mut out);
        out
    });

    let deadline = Instant::now() + GIT_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return None;
            }
        }
    };
    let out = reader.join().ok()?;
    status
        .success()
        .then(|| String::from_utf8_lossy(&out).into_owned())
}

static CACHE: Mutex<Option<HashMap<String, ProjectRoot>>> = Mutex::new(None);

/// Resolves a working directory to its project root, git-aware: `git rev-parse --git-common-dir`
/// consolidates worktrees and subfolders under one root. Falls back to the raw cwd when git is
/// missing or it's not a repo. Requires git >= 2.31 for `--path-format=absolute`.
pub fn resolve_project_root(cwd: &str) -> ProjectRoot {
    // Case-insensitive on Windows: cwds differing only by casing are still the same directory.
    let key = normalize_fs_path(cwd);
    if let Some(hit) = CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|c| c.get(&key))
    {
        return hit.clone();
    }
    let resolved = run_git(&[
        "-C",
        cwd,
        "rev-parse",
        "--path-format=absolute",
        "--git-common-dir",
    ])
    .and_then(|out| {
        let git_dir = out.trim().to_string();
        Path::new(&git_dir)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
    })
    .filter(|root| !root.is_empty())
    .map_or_else(
        || ProjectRoot {
            root: cwd.to_string(),
            is_git_repo: false,
        },
        |root| ProjectRoot {
            root,
            is_git_repo: true,
        },
    );
    CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(key, resolved.clone());
    resolved
}

/// Call before a manual refresh so a folder that became/stopped being a git repo is picked up.
pub fn clear_project_root_cache() {
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

#[cfg(test)]
pub(crate) mod test_repo {
    use super::run_git;
    use std::path::Path;
    use std::process::Command;

    pub fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@x.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .expect("git runs");
        assert!(
            status.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&status.stderr)
        );
    }

    /// A repo on branch `main` with one commit.
    pub fn init_repo(dir: &Path) {
        git(dir, &["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        git(dir, &["add", "."]);
        git(dir, &["commit", "-q", "-m", "init"]);
        assert!(run_git(&["-C", &dir.to_string_lossy(), "rev-parse", "HEAD"]).is_some());
    }
}

#[cfg(test)]
mod tests {
    use super::test_repo::{git, init_repo};
    use super::*;
    use crate::discovery::path_utils::normalize_fs_path;

    #[test]
    fn subfolder_and_worktree_resolve_to_the_repo_root() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(repo.join("sub")).unwrap();
        init_repo(&repo);
        let worktree = tmp.path().join("wt");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                &worktree.to_string_lossy(),
                "-b",
                "other",
            ],
        );

        let expected = normalize_fs_path(&repo.to_string_lossy());
        for cwd in [repo.join("sub"), worktree.clone(), repo.clone()] {
            let found = resolve_project_root(&cwd.to_string_lossy());
            assert!(found.is_git_repo, "{cwd:?}");
            assert_eq!(normalize_fs_path(&found.root), expected, "{cwd:?}");
        }
    }

    #[test]
    fn a_plain_folder_is_its_own_root() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path().to_string_lossy().into_owned();
        assert_eq!(
            resolve_project_root(&cwd),
            ProjectRoot {
                root: cwd,
                is_git_repo: false
            }
        );
    }

    #[test]
    fn a_missing_folder_is_not_an_error() {
        let found = resolve_project_root("Z:\\definitely\\not\\here");
        assert!(!found.is_git_repo);
    }
}
