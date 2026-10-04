use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process;

use serde_json::{json, Map, Value};

use super::account::current_account_email;
use super::session_status::{ensure_session_status_dir, session_status_dir};
use crate::commands::session_id::is_safe_session_id;
use crate::format::now_ms;

/// Claude Code's statusLine hook, run as `sdeck statusline-hook`: reads the JSON payload from stdin,
/// drops the session's usage record where the USAGE rows read it, and returns the line to display.
/// Never fails: a payload it can't use just yields an empty line.
pub fn run_statusline_hook(payload: &str) -> String {
    let Ok(Value::Object(payload)) = serde_json::from_str::<Value>(payload) else {
        return String::new();
    };
    let _ = write_usage_record(&payload, now_ms());
    status_line(&payload)
}

fn pct(value: Option<&Value>) -> Option<f64> {
    value.and_then(Value::as_f64).filter(|n| n.is_finite())
}

/// One decimal, so 0.9% doesn't read as 0.
fn one_decimal(n: f64) -> f64 {
    (n * 10.0).round() / 10.0
}

fn context_percent(payload: &Map<String, Value>) -> Option<f64> {
    pct(payload.get("context_window")?.get("used_percentage")).map(one_decimal)
}

/// `used_percentage` and `resets_at` (epoch seconds, returned as ms) of one rate-limit window.
fn window(payload: &Map<String, Value>, key: &str) -> (Option<f64>, Option<i64>) {
    let Some(window) = payload.get("rate_limits").and_then(|r| r.get(key)) else {
        return (None, None);
    };
    let resets_ms = window
        .get("resets_at")
        .and_then(Value::as_f64)
        .map(|s| (s * 1000.0) as i64);
    (pct(window.get("used_percentage")).map(one_decimal), resets_ms)
}

/// Merged into the existing file, not written whole: `rate_limits` is absent on some calls, and such
/// a call must not erase the last known 5h/7d reading.
fn write_usage_record(payload: &Map<String, Value>, now: i64) -> io::Result<()> {
    let Some(session_id) = payload
        .get("session_id")
        .and_then(Value::as_str)
        .filter(|id| is_safe_session_id(id))
    else {
        return Ok(());
    };
    ensure_session_status_dir()?;
    let path = session_status_dir().join(format!("{session_id}.usage.json"));
    let mut record: Map<String, Value> = match fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
    {
        Some(Value::Object(previous)) => previous,
        _ => Map::new(),
    };
    if let Some(context) = context_percent(payload) {
        record.insert("contextPercent".into(), json!(context));
    }
    for (key, percent_key, resets_key) in [
        ("five_hour", "fiveHourPercent", "fiveHourResetsAt"),
        ("seven_day", "sevenDayPercent", "sevenDayResetsAt"),
    ] {
        let (percent, resets_at) = window(payload, key);
        if let Some(percent) = percent {
            record.insert(percent_key.into(), json!(percent));
        }
        if let Some(resets_at) = resets_at {
            record.insert(resets_key.into(), json!(resets_at));
        }
    }
    record.insert("updatedAt".into(), json!(now));
    if let Some(email) = current_account_email() {
        record.insert("accountEmail".into(), json!(email));
    }
    // Write-then-rename so a reader (sdeck) never sees a half-written file.
    let tmp = path.with_file_name(format!(".{session_id}.usage.json.{}.tmp", process::id()));
    fs::write(&tmp, Value::Object(record).to_string())?;
    fs::rename(&tmp, &path)
}

fn status_line(payload: &Map<String, Value>) -> String {
    let cwd = payload
        .get("workspace")
        .and_then(|w| w.get("current_dir"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    match context_percent(payload) {
        Some(ctx) if !cwd.is_empty() => format!("{cwd} · ctx {}%", ctx.floor()),
        _ => cwd.to_string(),
    }
}

/// The statusLine `command` for this executable. Claude Code runs it through a POSIX shell, even on
/// Windows, so the path is quoted and uses forward slashes.
pub fn statusline_command(exe: &Path) -> String {
    format!("\"{}\" statusline-hook", exe.to_string_lossy().replace('\\', "/"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusLineOutcome {
    Added,
    /// The account already has a `statusLine`: left alone, it may be the user's own script.
    AlreadySet,
    /// `settings.json` is not a JSON object: left alone rather than overwritten.
    Unreadable,
    Failed,
}

/// Adds `statusLine` to `<config dir>/settings.json` unless one is set. Writes through a symlinked
/// `settings.json`, so an account sharing another's settings shares the result.
pub fn enable_statusline(config_dir: &Path, command: &str) -> StatusLineOutcome {
    let path = config_dir.join("settings.json");
    let mut settings = match fs::read_to_string(&path) {
        Ok(raw) => match serde_json::from_str::<Value>(&raw) {
            Ok(Value::Object(map)) => map,
            _ => return StatusLineOutcome::Unreadable,
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => Map::new(),
        Err(_) => return StatusLineOutcome::Failed,
    };
    if settings.contains_key("statusLine") {
        return StatusLineOutcome::AlreadySet;
    }
    settings.insert(
        "statusLine".into(),
        json!({"type": "command", "command": command}),
    );
    let written = fs::create_dir_all(config_dir).and_then(|()| {
        let mut text = serde_json::to_string_pretty(&Value::Object(settings)).unwrap_or_default();
        text.push('\n');
        fs::write(&path, text)
    });
    match written {
        Ok(()) => StatusLineOutcome::Added,
        Err(_) => StatusLineOutcome::Failed,
    }
}

/// [`enable_statusline`] for each config dir, in order.
pub fn enable_statusline_for_all(config_dirs: &[PathBuf], command: &str) -> Vec<StatusLineOutcome> {
    config_dirs
        .iter()
        .map(|dir| enable_statusline(dir, command))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_env::EnvGuard;

    fn fixture() -> (EnvGuard, tempfile::TempDir) {
        let g = EnvGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("SESSION_DECK_STATUS_DIR", tmp.path().join("status"));
        let config = tmp.path().join("config");
        fs::create_dir_all(&config).unwrap();
        fs::write(
            config.join(".claude.json"),
            r#"{"oauthAccount":{"emailAddress":"me@x.com"}}"#,
        )
        .unwrap();
        std::env::set_var("CLAUDE_CONFIG_DIR", &config);
        (g, tmp)
    }

    fn read_record(id: &str) -> Value {
        let path = session_status_dir().join(format!("{id}.usage.json"));
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn writes_context_rate_limits_and_account() {
        let (_g, _tmp) = fixture();
        let out = run_statusline_hook(
            r#"{"session_id":"s1","workspace":{"current_dir":"/p"},
                "context_window":{"used_percentage":6.04},
                "rate_limits":{"five_hour":{"used_percentage":0.93,"resets_at":100},
                               "seven_day":{"used_percentage":24.0,"resets_at":200}}}"#,
        );
        assert_eq!(out, "/p · ctx 6%");
        let r = read_record("s1");
        assert_eq!(r["contextPercent"], json!(6.0));
        assert_eq!(r["fiveHourPercent"], json!(0.9));
        assert_eq!(r["fiveHourResetsAt"], json!(100_000));
        assert_eq!(r["sevenDayPercent"], json!(24.0));
        assert_eq!(r["accountEmail"], json!("me@x.com"));
        assert!(r["updatedAt"].as_i64().unwrap() > 0);
    }

    #[test]
    fn a_call_without_rate_limits_keeps_the_last_reading() {
        let (_g, _tmp) = fixture();
        run_statusline_hook(
            r#"{"session_id":"s1","rate_limits":{"five_hour":{"used_percentage":40,"resets_at":100}}}"#,
        );
        run_statusline_hook(r#"{"session_id":"s1","context_window":{"used_percentage":9}}"#);
        let r = read_record("s1");
        assert_eq!(r["fiveHourPercent"], json!(40.0));
        assert_eq!(r["contextPercent"], json!(9.0));
    }

    #[test]
    fn unusable_payloads_and_unsafe_ids_write_nothing() {
        let (_g, _tmp) = fixture();
        assert_eq!(run_statusline_hook("not json"), "");
        run_statusline_hook(r#"{"session_id":"../x"}"#);
        run_statusline_hook(r#"{"workspace":{}}"#);
        assert!(!session_status_dir().exists());
    }

    #[test]
    fn enabling_adds_a_status_line_and_keeps_other_settings() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("settings.json"),
            r#"{"model":"opus","env":{"A":"1"}}"#,
        )
        .unwrap();
        assert_eq!(
            enable_statusline(tmp.path(), "sdeck statusline-hook"),
            StatusLineOutcome::Added
        );
        let settings: Value =
            serde_json::from_str(&fs::read_to_string(tmp.path().join("settings.json")).unwrap()).unwrap();
        assert_eq!(settings["model"], json!("opus"));
        assert_eq!(settings["env"]["A"], json!("1"));
        assert_eq!(settings["statusLine"]["command"], json!("sdeck statusline-hook"));
        assert_eq!(settings.as_object().unwrap().keys().next().unwrap(), "model");
    }

    #[test]
    fn enabling_creates_a_missing_settings_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("new-account");
        assert_eq!(enable_statusline(&dir, "c"), StatusLineOutcome::Added);
        assert!(dir.join("settings.json").exists());
    }

    #[test]
    fn enabling_leaves_an_existing_status_line_and_broken_files_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let own = r#"{"statusLine":{"type":"command","command":"mine.sh"}}"#;
        fs::write(tmp.path().join("settings.json"), own).unwrap();
        assert_eq!(enable_statusline(tmp.path(), "c"), StatusLineOutcome::AlreadySet);
        assert_eq!(fs::read_to_string(tmp.path().join("settings.json")).unwrap(), own);

        fs::write(tmp.path().join("settings.json"), "{ not json").unwrap();
        assert_eq!(enable_statusline(tmp.path(), "c"), StatusLineOutcome::Unreadable);
        assert_eq!(
            fs::read_to_string(tmp.path().join("settings.json")).unwrap(),
            "{ not json"
        );
    }

    #[test]
    fn the_command_quotes_the_path_and_uses_forward_slashes() {
        assert_eq!(
            statusline_command(Path::new(r"C:\Users\me\bin\sdeck.exe")),
            r#""C:/Users/me/bin/sdeck.exe" statusline-hook"#
        );
    }
}
