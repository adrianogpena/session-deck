use std::env;
use std::path::PathBuf;

fn env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// The user's home directory; `SDECK_USER_HOME` replaces it (tests point it at a temp dir).
pub fn user_home() -> PathBuf {
    env_path("SDECK_USER_HOME")
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// sdeck's own state folder: `SESSION_DECK_HOME` or `~/.session-deck`.
pub fn deck_home() -> PathBuf {
    env_path("SESSION_DECK_HOME").unwrap_or_else(|| user_home().join(".session-deck"))
}

/// Where status/usage files are dropped: `SESSION_DECK_STATUS_DIR` or `~/.claude/session-deck-status`.
pub fn status_dir() -> PathBuf {
    env_path("SESSION_DECK_STATUS_DIR").unwrap_or_else(|| claude_dir().join("session-deck-status"))
}

/// The default Claude config dir. Per-account paths come from `account::Account`, not from here.
pub fn claude_dir() -> PathBuf {
    user_home().join(".claude")
}

pub fn copilot_dir() -> PathBuf {
    user_home().join(".copilot")
}

#[cfg(test)]
pub(crate) mod test_env {
    use std::ffi::OsString;
    use std::sync::{Mutex, MutexGuard};

    static LOCK: Mutex<()> = Mutex::new(());

    pub const VARS: [&str; 4] = [
        "SDECK_USER_HOME",
        "SESSION_DECK_HOME",
        "SESSION_DECK_STATUS_DIR",
        "CLAUDE_CONFIG_DIR",
    ];

    /// Serializes tests that touch the process environment and restores the variables on drop.
    pub struct EnvGuard {
        _lock: MutexGuard<'static, ()>,
        saved: Vec<(&'static str, Option<OsString>)>,
    }

    impl EnvGuard {
        pub fn new() -> Self {
            let lock = LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let saved = VARS.iter().map(|v| (*v, std::env::var_os(v))).collect();
            for v in VARS {
                std::env::remove_var(v);
            }
            EnvGuard { _lock: lock, saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (name, value) in &self.saved {
                match value {
                    Some(v) => std::env::set_var(name, v),
                    None => std::env::remove_var(name),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_env::EnvGuard;
    use super::*;

    #[test]
    fn user_home_override_redirects_every_derived_path() {
        let _g = EnvGuard::new();
        let home = std::env::temp_dir().join("sdeck-home");
        std::env::set_var("SDECK_USER_HOME", &home);
        assert_eq!(user_home(), home);
        assert_eq!(deck_home(), home.join(".session-deck"));
        assert_eq!(claude_dir(), home.join(".claude"));
        assert_eq!(copilot_dir(), home.join(".copilot"));
        assert_eq!(status_dir(), home.join(".claude").join("session-deck-status"));
    }

    #[test]
    fn deck_home_and_status_dir_have_their_own_overrides() {
        let _g = EnvGuard::new();
        let home = std::env::temp_dir().join("sdeck-home");
        let deck = std::env::temp_dir().join("sdeck-deck");
        let status = std::env::temp_dir().join("sdeck-status");
        std::env::set_var("SDECK_USER_HOME", &home);
        std::env::set_var("SESSION_DECK_HOME", &deck);
        std::env::set_var("SESSION_DECK_STATUS_DIR", &status);
        assert_eq!(deck_home(), deck);
        assert_eq!(status_dir(), status);
        assert_eq!(claude_dir(), home.join(".claude"));
    }

    #[test]
    fn without_overrides_paths_hang_off_the_real_home() {
        let _g = EnvGuard::new();
        assert_eq!(deck_home(), user_home().join(".session-deck"));
        assert!(status_dir().ends_with("session-deck-status"));
    }

    #[test]
    fn empty_override_is_ignored() {
        let _g = EnvGuard::new();
        std::env::set_var("SESSION_DECK_HOME", "");
        assert_eq!(deck_home(), user_home().join(".session-deck"));
    }
}
