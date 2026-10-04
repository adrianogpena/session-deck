use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::paths;

/// A Claude config dir: `~/.claude` (default) or a sibling `~/.claude-*` with a logged-in account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub config_dir: PathBuf,
    pub email: Option<String>,
    pub is_default: bool,
}

impl Account {
    pub fn projects_dir(&self) -> PathBuf {
        self.config_dir.join("projects")
    }

    pub fn sessions_dir(&self) -> PathBuf {
        self.config_dir.join("sessions")
    }

    /// Claude keeps the default account's `.claude.json` in the home root, not inside `~/.claude`.
    pub fn config_file(&self) -> PathBuf {
        config_file_for(&self.config_dir, self.is_default)
    }
}

fn config_file_for(config_dir: &Path, is_default: bool) -> PathBuf {
    if is_default {
        paths::user_home().join(".claude.json")
    } else {
        config_dir.join(".claude.json")
    }
}

fn read_email(config_file: &Path) -> Option<String> {
    let text = fs::read_to_string(config_file).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&text).ok()?;
    let email = parsed.get("oauthAccount")?.get("emailAddress")?.as_str()?;
    (!email.is_empty()).then(|| email.to_string())
}

/// The email of the account logged in to the current process's config dir (`CLAUDE_CONFIG_DIR`
/// when set, else the home-root `.claude.json`), or `None` when it can't be determined.
pub fn current_account_email() -> Option<String> {
    let file = match env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir).join(".claude.json"),
        None => paths::user_home().join(".claude.json"),
    };
    read_email(&file)
}

/// The default account (always returned, so its sessions are found even when logged out) followed
/// by each `~/.claude-*` dir with a logged-in account, ordered by dir name.
pub fn discover_accounts() -> Vec<Account> {
    let default_dir = paths::claude_dir();
    let mut accounts = vec![Account {
        email: read_email(&config_file_for(&default_dir, true)),
        config_dir: default_dir,
        is_default: true,
    }];
    let mut siblings: Vec<PathBuf> = fs::read_dir(paths::user_home())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(".claude-"))
        .filter(|e| e.path().is_dir())
        .map(|e| e.path())
        .collect();
    siblings.sort_by_key(|p| p.file_name().map(|n| n.to_os_string()));
    for dir in siblings {
        if let Some(email) = read_email(&config_file_for(&dir, false)) {
            accounts.push(Account {
                config_dir: dir,
                email: Some(email),
                is_default: false,
            });
        }
    }
    accounts
}

/// What a new account's config dir can share with another one: (entry, label, checked by default).
pub const SHAREABLE: &[(&str, &str, bool)] = &[
    ("skills", "skills", true),
    ("agents", "agents", true),
    ("commands", "commands", true),
    ("CLAUDE.md", "CLAUDE.md", true),
    ("settings.json", "settings.json", false),
    ("plugins", "plugins", false),
    ("projects", "projects (session history)", false),
];

/// The config dir for a typed folder name: `personal`, `claude-personal` and `.claude-personal` all
/// give `~/.claude-personal`. Errors on an empty or path-like name.
pub fn account_dir_for_name(input: &str) -> Result<PathBuf, String> {
    let name = input.trim().trim_start_matches('.');
    let name = name.strip_prefix("claude-").unwrap_or(name);
    if name.is_empty() || name.contains(['/', '\\', ':']) || name == ".." {
        return Err("Use a plain folder name, e.g. claude-personal".into());
    }
    Ok(paths::user_home().join(format!(".claude-{name}")))
}

#[cfg(windows)]
fn link(target: &Path, link: &Path) -> std::io::Result<()> {
    if target.is_dir() {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    }
}

#[cfg(unix)]
fn link(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// Creates `dir` and symlinks each of `entries` in it to the same entry of `source`. Entries that
/// don't exist in `source` or already exist in `dir` are skipped. Returns the entries linked.
pub fn create_account_dir(dir: &Path, source: &Path, entries: &[&str]) -> Result<Vec<String>, String> {
    fs::create_dir_all(dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    let mut linked = Vec::new();
    for entry in entries {
        let (from, to) = (source.join(entry), dir.join(entry));
        if !from.exists() || to.symlink_metadata().is_ok() {
            continue;
        }
        link(&from, &to).map_err(|e| format!("Could not link {entry}: {e}"))?;
        linked.push((*entry).to_string());
    }
    Ok(linked)
}

/// The discovered account whose config dir is `path`.
pub fn account_for_config_dir(path: &Path) -> Option<Account> {
    discover_accounts().into_iter().find(|a| a.config_dir == path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_env::EnvGuard;

    fn write(path: PathBuf, email: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            format!(r#"{{"oauthAccount":{{"emailAddress":"{email}"}}}}"#),
        )
        .unwrap();
    }

    fn fixture() -> (EnvGuard, tempfile::TempDir) {
        let g = EnvGuard::new();
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("SDECK_USER_HOME", home.path());
        write(home.path().join(".claude.json"), "work@x.com");
        fs::create_dir_all(home.path().join(".claude")).unwrap();
        write(home.path().join(".claude-personal/.claude.json"), "me@x.com");
        fs::create_dir_all(home.path().join(".claude-empty")).unwrap();
        (g, home)
    }

    #[test]
    fn discovers_default_first_and_skips_logged_out_siblings() {
        let (_g, home) = fixture();
        let accounts = discover_accounts();
        assert_eq!(accounts.len(), 2);
        assert!(accounts[0].is_default);
        assert_eq!(accounts[0].email.as_deref(), Some("work@x.com"));
        assert_eq!(accounts[0].config_dir, home.path().join(".claude"));
        assert_eq!(accounts[1].email.as_deref(), Some("me@x.com"));
        assert!(!accounts[1].is_default);
    }

    #[test]
    fn logged_out_default_is_still_returned() {
        let _g = EnvGuard::new();
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("SDECK_USER_HOME", home.path());
        let accounts = discover_accounts();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].email, None);
    }

    #[test]
    fn dirs_and_lookup() {
        let (_g, home) = fixture();
        let personal = account_for_config_dir(&home.path().join(".claude-personal")).unwrap();
        assert_eq!(
            personal.projects_dir(),
            home.path().join(".claude-personal/projects")
        );
        assert_eq!(
            personal.sessions_dir(),
            home.path().join(".claude-personal/sessions")
        );
        assert_eq!(
            personal.config_file(),
            home.path().join(".claude-personal/.claude.json")
        );
        assert!(account_for_config_dir(&home.path().join(".claude-empty")).is_none());
    }

    #[test]
    fn account_dir_names_are_normalised() {
        let (_g, home) = fixture();
        for input in ["personal", "claude-personal", ".claude-personal", " personal "] {
            assert_eq!(
                account_dir_for_name(input).unwrap(),
                home.path().join(".claude-personal")
            );
        }
        assert!(account_dir_for_name("").is_err());
        assert!(account_dir_for_name("a/b").is_err());
    }

    #[test]
    fn creating_an_account_links_only_existing_entries() {
        let (_g, home) = fixture();
        let source = home.path().join(".claude");
        fs::create_dir_all(source.join("skills")).unwrap();
        fs::write(source.join("CLAUDE.md"), "x").unwrap();
        let dir = home.path().join(".claude-new");
        let linked = create_account_dir(&dir, &source, &["skills", "agents", "CLAUDE.md"]);
        match linked {
            Ok(linked) => {
                assert_eq!(linked, ["skills", "CLAUDE.md"]);
                assert!(dir.join("skills").exists());
                assert!(!dir.join("agents").exists());
            }
            Err(e) => assert!(dir.is_dir(), "{e}"),
        }
    }

    #[test]
    fn current_email_follows_config_dir_env() {
        let (_g, home) = fixture();
        assert_eq!(current_account_email().as_deref(), Some("work@x.com"));
        std::env::set_var("CLAUDE_CONFIG_DIR", home.path().join(".claude-personal"));
        assert_eq!(current_account_email().as_deref(), Some("me@x.com"));
        std::env::set_var("CLAUDE_CONFIG_DIR", home.path().join(".claude-empty"));
        assert_eq!(current_account_email(), None);
    }
}
