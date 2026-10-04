//! Port of `gitStatusTracker.ts`: one `git status` per distinct cwd, cached between polls. `get`
//! never blocks; `read_all` is what actually shells out, so callers run it off the main thread.

use std::collections::{HashMap, HashSet};

use sdeck_core::concurrency::map_with_concurrency;
use sdeck_core::discovery::git_status::{read_git_status, GitStatus};

const POLL_CONCURRENCY: usize = 6;

pub type GitStatuses = Vec<(String, Option<GitStatus>)>;

#[derive(Debug, Default)]
pub struct GitStatusTracker {
    by_cwd: HashMap<String, Option<GitStatus>>,
}

impl GitStatusTracker {
    pub fn get(&self, cwd: &str) -> Option<&GitStatus> {
        self.by_cwd.get(cwd).and_then(Option::as_ref)
    }

    /// Replaces the cache with a poll's result, dropping cwds no longer polled.
    pub fn replace(&mut self, statuses: GitStatuses) {
        self.by_cwd = statuses.into_iter().collect();
    }
}

/// Reads `git status` for every distinct cwd in `cwds`. Blocking.
pub fn read_all(cwds: &[String]) -> GitStatuses {
    let mut seen = HashSet::new();
    let unique: Vec<String> = cwds
        .iter()
        .filter(|c| seen.insert((*c).clone()))
        .cloned()
        .collect();
    let statuses = map_with_concurrency(&unique, POLL_CONCURRENCY, |cwd, _| read_git_status(cwd));
    unique.into_iter().zip(statuses).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
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
            .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }

    #[test]
    fn polls_each_distinct_cwd_once_and_get_never_shells_out() {
        let repo = tempfile::tempdir().unwrap();
        git(repo.path(), &["init", "-q", "-b", "main"]);
        std::fs::write(repo.path().join("a.txt"), "a").unwrap();
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-q", "-m", "one"]);
        std::fs::write(repo.path().join("b.txt"), "b").unwrap();
        let plain = tempfile::tempdir().unwrap();
        let repo_cwd = repo.path().to_string_lossy().into_owned();
        let plain_cwd = plain.path().to_string_lossy().into_owned();

        let mut tracker = GitStatusTracker::default();
        assert!(tracker.get(&repo_cwd).is_none());
        let polled = read_all(&[repo_cwd.clone(), plain_cwd.clone(), repo_cwd.clone()]);
        assert_eq!(polled.len(), 2);
        tracker.replace(polled);
        let status = tracker.get(&repo_cwd).unwrap();
        assert_eq!((status.branch.as_deref(), status.dirty), (Some("main"), 1));
        assert!(tracker.get(&plain_cwd).is_none());

        tracker.replace(read_all(&[plain_cwd]));
        assert!(tracker.get(&repo_cwd).is_none());
    }
}
