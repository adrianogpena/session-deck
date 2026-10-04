use std::path::{Component, Path, PathBuf, MAIN_SEPARATOR};

use crate::paths::user_home;

/// Makes `p` absolute and removes `.`/`..` segments lexically (the path need not exist).
fn resolve(p: &Path) -> PathBuf {
    let abs = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let mut out = PathBuf::new();
    for comp in abs.components() {
        match comp {
            Component::ParentDir => {
                if !out.pop() {
                    out.push(comp);
                }
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// A case-insensitive-safe key for *comparing* filesystem paths on Windows — never for display.
pub fn normalize_fs_path(fs_path: &str) -> String {
    let resolved = resolve(Path::new(fs_path)).to_string_lossy().into_owned();
    if cfg!(windows) {
        resolved.to_lowercase()
    } else {
        resolved
    }
}

/// True when `candidate` resolves to `root` itself or somewhere inside it.
pub fn is_inside(root: &str, candidate: &str) -> bool {
    let root = normalize_fs_path(root);
    let candidate = normalize_fs_path(candidate);
    candidate == root || candidate.starts_with(&format!("{root}{MAIN_SEPARATOR}"))
}

fn expand_home_with(root: &str, home: &Path) -> String {
    if root == "~" {
        return home.to_string_lossy().into_owned();
    }
    if let Some(rest) = root.strip_prefix("~/").or_else(|| root.strip_prefix("~\\")) {
        return home.join(rest).to_string_lossy().into_owned();
    }
    root.to_string()
}

/// `~` is a shell convention, never expanded by the OS. Only a leading `~` is handled — no `~user` support.
pub fn expand_home(root: &str) -> String {
    expand_home_with(root, &user_home())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn join(parts: &[&str]) -> String {
        parts.iter().collect::<PathBuf>().to_string_lossy().into_owned()
    }

    #[test]
    fn normalize_resolves_relative_segments_away() {
        assert_eq!(
            normalize_fs_path(&join(&["some", "dir"])),
            normalize_fs_path(&join(&["some", ".", "dir"]))
        );
        assert_eq!(
            normalize_fs_path(&join(&["some", "x", "..", "dir"])),
            normalize_fs_path(&join(&["some", "dir"]))
        );
    }

    #[test]
    fn normalize_is_case_insensitive_on_windows_only() {
        let lower = normalize_fs_path("C:\\Users\\me\\app");
        let upper = normalize_fs_path("C:\\USERS\\ME\\APP");
        if cfg!(windows) {
            assert_eq!(lower, upper);
        } else {
            assert_ne!(lower, upper);
        }
    }

    #[test]
    fn is_inside_accepts_root_itself_and_real_children() {
        let root = "/claude/projects";
        assert!(is_inside(root, root));
        assert!(is_inside(root, &join(&[root, "proj", "session.jsonl"])));
    }

    #[test]
    fn is_inside_rejects_a_path_that_escapes_root_via_parent() {
        let root = "/claude/projects";
        assert!(!is_inside(root, &join(&[root, "..", "settings.json"])));
    }

    #[test]
    fn is_inside_rejects_a_sibling_that_shares_a_name_prefix() {
        assert!(!is_inside("/claude/projects", "/claude/projects-evil/x"));
    }

    #[test]
    fn expand_home_expands_bare_tilde() {
        let home = Path::new("/h/me");
        assert_eq!(expand_home_with("~", home), home.to_string_lossy());
    }

    #[test]
    fn expand_home_expands_leading_tilde_slash_and_backslash() {
        let home = Path::new("/h/me");
        assert_eq!(
            expand_home_with("~/foo/bar", home),
            home.join("foo/bar").to_string_lossy()
        );
        assert_eq!(
            expand_home_with("~\\foo\\bar", home),
            home.join("foo\\bar").to_string_lossy()
        );
    }

    #[test]
    fn expand_home_leaves_absolute_paths_and_tilde_user_untouched() {
        let home = Path::new("/h/me");
        assert_eq!(
            expand_home_with("C:/Source/my-project", home),
            "C:/Source/my-project"
        );
        assert_eq!(expand_home_with("~someuser/foo", home), "~someuser/foo");
    }

    #[test]
    fn expand_home_uses_the_overridable_user_home() {
        let _g = crate::paths::test_env::EnvGuard::new();
        std::env::set_var("SDECK_USER_HOME", "/h/me");
        assert_eq!(expand_home("~"), "/h/me");
    }
}
