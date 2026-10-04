//! Test-only helpers: a lock + restore guard for the home-redirecting environment variables.

use std::ffi::OsString;
use std::sync::{Mutex, MutexGuard};

static LOCK: Mutex<()> = Mutex::new(());

const VARS: [&str; 4] = [
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
