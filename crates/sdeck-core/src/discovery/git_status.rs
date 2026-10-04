use super::git_project::run_git;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitStatus {
    /// `None` on a detached HEAD (or when the branch name couldn't be read).
    pub branch: Option<String>,
    /// Commits ahead of the upstream branch. 0 when there's no upstream.
    pub ahead: u32,
    /// Commits behind the upstream branch. 0 when there's no upstream.
    pub behind: u32,
    /// Changed + untracked entries.
    pub dirty: u32,
}

/// Parses `git status --porcelain=v2 --branch --ahead-behind` output. A stable, documented format,
/// so this is a real contract, not best-effort.
pub fn parse_git_status_porcelain(output: &str) -> GitStatus {
    let mut status = GitStatus::default();
    for line in output.split('\n') {
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix("# branch.head ") {
            let name = name.trim();
            status.branch = (name != "(detached)").then(|| name.to_string());
        } else if let Some(counts) = line.strip_prefix("# branch.ab ") {
            if let Some((ahead, behind)) = parse_ahead_behind(counts) {
                status.ahead = ahead;
                status.behind = behind;
            }
        } else if !line.starts_with('#') {
            status.dirty += 1;
        }
    }
    status
}

fn parse_ahead_behind(counts: &str) -> Option<(u32, u32)> {
    let (ahead, behind) = counts.split_once(' ')?;
    Some((
        ahead.strip_prefix('+')?.parse().ok()?,
        behind.strip_prefix('-')?.parse().ok()?,
    ))
}

/// `None` for anything that isn't a clean read: not a repo, git missing, or a transient error;
/// callers just show nothing.
pub fn read_git_status(cwd: &str) -> Option<GitStatus> {
    // --no-optional-locks: never blocks on (or trips) a lock the agent's own git commands hold.
    run_git(&[
        "-C",
        cwd,
        "--no-optional-locks",
        "status",
        "--porcelain=v2",
        "--branch",
        "--ahead-behind",
    ])
    .map(|out| parse_git_status_porcelain(&out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::git_project::test_repo::init_repo;

    #[test]
    fn reads_the_branch_name() {
        let status = parse_git_status_porcelain("# branch.oid abc123\n# branch.head main\n");
        assert_eq!(
            status,
            GitStatus {
                branch: Some("main".into()),
                ahead: 0,
                behind: 0,
                dirty: 0
            }
        );
    }

    #[test]
    fn detached_head_is_no_branch() {
        assert_eq!(
            parse_git_status_porcelain("# branch.head (detached)\n").branch,
            None
        );
    }

    #[test]
    fn reads_ahead_behind_counts() {
        let status = parse_git_status_porcelain("# branch.head main\n# branch.ab +2 -1\n");
        assert_eq!((status.ahead, status.behind), (2, 1));
    }

    #[test]
    fn ahead_behind_default_to_zero_without_upstream() {
        let status = parse_git_status_porcelain("# branch.head main\n");
        assert_eq!((status.ahead, status.behind), (0, 0));
    }

    #[test]
    fn counts_changed_and_untracked_entries_as_dirty() {
        let output = [
            "# branch.head main",
            "1 .M N... 100644 100644 100644 abc123 def456 src/app.ts",
            "2 R. N... 100644 100644 100644 abc123 def456 R100 src/new.ts\tsrc/old.ts",
            "u UU N... 100644 100644 100644 100644 a b c d src/conflict.ts",
            "? untracked.txt",
        ]
        .join("\n");
        assert_eq!(parse_git_status_porcelain(&output).dirty, 4);
    }

    #[test]
    fn ignores_blank_lines_and_empty_input() {
        assert_eq!(parse_git_status_porcelain("# branch.head main\n\n\n").dirty, 0);
        assert_eq!(parse_git_status_porcelain(""), GitStatus::default());
    }

    #[test]
    fn real_repo_reports_branch_and_dirty_count() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let cwd = tmp.path().to_string_lossy().into_owned();
        let clean = read_git_status(&cwd).unwrap();
        assert_eq!((clean.branch.as_deref(), clean.dirty), (Some("main"), 0));

        std::fs::write(tmp.path().join("a.txt"), "changed").unwrap();
        std::fs::write(tmp.path().join("new.txt"), "new").unwrap();
        assert_eq!(read_git_status(&cwd).unwrap().dirty, 2);
    }

    #[test]
    fn non_repo_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(read_git_status(&tmp.path().to_string_lossy()), None);
    }
}
