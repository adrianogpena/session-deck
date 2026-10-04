use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use serde_json::{json, Value};

use crate::format::one_line;
use crate::status::account::Account;

/// One folder Claude Code created under an account's `projects/` for a distinct cwd.
pub fn list_project_dir_names(account: &Account) -> Vec<String> {
    let Ok(entries) = fs::read_dir(account.projects_dir()) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect()
}

pub fn list_session_files(account: &Account, dir_name: &str) -> Vec<String> {
    let Ok(entries) = fs::read_dir(account.projects_dir().join(dir_name)) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|f| f.ends_with(".jsonl"))
        .collect()
}

/// Reverses Claude Code's "-"-for-path-separator folder-name encoding. Lossy (a literal "-" is
/// indistinguishable from an encoded separator), so only a last resort when no session has a
/// recorded `cwd`.
pub fn decode_project_path(dir_name: &str) -> String {
    let b = dir_name.as_bytes();
    let (prefix, rest) = if b.len() >= 3 && b[0].is_ascii_alphabetic() && &b[1..3] == b"--" {
        (format!("{}:\\", b[0] as char), &dir_name[3..])
    } else {
        (String::new(), dir_name)
    };
    prefix + &rest.replace('-', "\\")
}

/// `content` may be a plain string, an array of content blocks, or a single block object; keeps
/// only text/thinking.
pub fn extract_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .map(extract_text_from_part)
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(_) => extract_text_from_part(content),
        _ => String::new(),
    }
}

fn extract_text_from_part(part: &Value) -> String {
    match part {
        Value::String(s) => s.clone(),
        Value::Object(block) => ["text", "thinking"]
            .iter()
            .find_map(|k| block.get(*k).and_then(Value::as_str))
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    }
}

/// Assistant text for display: `text` blocks only, deliberately excluding `thinking`.
pub fn extract_assistant_display_text(content: &Value) -> String {
    let Some(blocks) = content.as_array() else {
        return String::new();
    };
    blocks
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

const HIDDEN_USER_PROMPT_PREFIXES: [&str; 9] = [
    "<local-command-caveat>",
    "<command-name>",
    "<command-message>",
    "<command-args>",
    "<local-command-stdout>",
    "<local-command-stderr>",
    "<local-command-exit-code>",
    "<usage>",
    "agentId:",
];

/// Filters out synthetic "user" turns (slash-command echoes, usage notices, ...) that aren't prompts.
pub fn is_displayable_user_prompt(raw_prompt: &str) -> bool {
    let normalized = one_line(raw_prompt);
    !normalized.is_empty()
        && !HIDDEN_USER_PROMPT_PREFIXES
            .iter()
            .any(|p| normalized.starts_with(p))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionMeta {
    pub cwd: Option<String>,
    pub first_prompt: Option<String>,
    /// Claude Code's own title for the session, see [`find_session_title`].
    pub title: Option<String>,
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() > max {
        let mut out: String = text.chars().take(max - 1).collect();
        out.push('…');
        out
    } else {
        text.to_string()
    }
}

fn mtime_ms(path: &Path) -> Option<i64> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    let since = modified.duration_since(UNIX_EPOCH).ok()?;
    Some(since.as_millis() as i64)
}

/// Parsed JSON records of a transcript, skipping blank and unparseable lines.
fn read_records(path: &Path) -> Vec<Value> {
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    BufReader::new(file)
        .split(b'\n')
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_slice(&line).ok())
        .collect()
}

/// A record's message when it is `kind` with role `role`.
fn message_of<'a>(record: &'a Value, kind: &str, role: &str) -> Option<&'a Value> {
    let message = record.get("message")?;
    (record.get("type").and_then(Value::as_str) == Some(kind)
        && message.get("role").and_then(Value::as_str) == Some(role))
    .then_some(message)
}

type MtimeCache<T> = Mutex<Option<HashMap<PathBuf, (i64, T)>>>;

static META_CACHE: MtimeCache<SessionMeta> = Mutex::new(None);
static SEARCH_CACHE: MtimeCache<String> = Mutex::new(None);

fn cached<T: Clone>(cache: &MtimeCache<T>, path: &Path, parse: impl FnOnce() -> T) -> Option<T> {
    let mtime = mtime_ms(path)?;
    {
        let guard = cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((m, v)) = guard.as_ref().and_then(|c| c.get(path)) {
            if *m == mtime {
                return Some(v.clone());
            }
        }
    }
    let value = parse();
    cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(path.to_path_buf(), (mtime, value.clone()));
    Some(value)
}

/// A session's starting `cwd`, first prompt and title. Cached by mtime so an unchanged session is
/// never re-parsed.
pub fn read_session_meta(path: &Path) -> SessionMeta {
    cached(&META_CACHE, path, || parse_session_meta(path)).unwrap_or_default()
}

/// Call on a manual refresh so an externally-edited transcript is never served from a stale cache.
pub fn clear_session_meta_cache() {
    *META_CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

fn parse_session_meta(path: &Path) -> SessionMeta {
    let mut cwd: Option<String> = None;
    let mut first_prompt: Option<String> = None;
    if let Ok(file) = File::open(path) {
        for line in BufReader::new(file).split(b'\n').map_while(Result::ok) {
            if cwd.is_some() && first_prompt.is_some() {
                break;
            }
            let Ok(record) = serde_json::from_slice::<Value>(&line) else {
                continue;
            };
            if cwd.is_none() {
                cwd = record.get("cwd").and_then(Value::as_str).map(str::to_string);
            }
            if first_prompt.is_none() {
                if let Some(message) = message_of(&record, "user", "user") {
                    let text = extract_text(message.get("content").unwrap_or(&Value::Null));
                    let text = text.trim();
                    if !text.is_empty() && is_displayable_user_prompt(text) {
                        first_prompt = Some(truncate(&one_line(text), 80));
                    }
                }
            }
        }
    }
    SessionMeta {
        cwd,
        first_prompt,
        title: read_session_title(path),
    }
}

/// Tail windows to scan for the title, smallest first. Claude Code re-appends its title records
/// throughout a session, so the newest copy sits near the end of the file.
const TITLE_TAIL_WINDOWS: [u64; 2] = [64 * 1024, 1024 * 1024];

fn read_session_title(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    for window in TITLE_TAIL_WINDOWS {
        let length = window.min(size);
        file.seek(SeekFrom::Start(size - length)).ok()?;
        let mut buffer = vec![0u8; length as usize];
        file.read_exact(&mut buffer).ok()?;
        // The window's first line is usually cut mid-record; find_session_title skips it.
        let text = String::from_utf8_lossy(&buffer);
        let lines: Vec<&str> = text.split('\n').collect();
        let title = find_session_title(&lines);
        if title.is_some() || length == size {
            return title;
        }
    }
    None
}

/// The title Claude Code itself shows in `/resume`: the newest `/rename` (`custom-title` record)
/// wins over the newest AI-generated summary (`ai-title` record). Both are undocumented formats, so
/// best-effort. Unparseable lines are skipped.
pub fn find_session_title(lines: &[&str]) -> Option<String> {
    let mut ai_title: Option<String> = None;
    for line in lines.iter().rev() {
        if !line.contains("\"custom-title\"") && !line.contains("\"ai-title\"") {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let field = |key: &str| {
            record
                .get(key)
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .map(clean_title)
        };
        match record.get("type").and_then(Value::as_str) {
            Some("custom-title") => {
                if let Some(t) = field("customTitle") {
                    return Some(t);
                }
            }
            Some("ai-title") if ai_title.is_none() => ai_title = field("aiTitle"),
            _ => {}
        }
    }
    ai_title
}

fn clean_title(title: &str) -> String {
    truncate(&one_line(title), 80)
}

/// Full session content (all prompts + assistant text) for search; cached separately from
/// [`read_session_meta`] since it is a heavier read.
pub fn read_session_search_text(path: &Path) -> String {
    cached(&SEARCH_CACHE, path, || parse_session_search_text(path)).unwrap_or_default()
}

pub fn clear_search_text_cache() {
    *SEARCH_CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

fn parse_session_search_text(path: &Path) -> String {
    let mut parts: Vec<String> = Vec::new();
    for record in read_records(path) {
        if let Some(m) = message_of(&record, "user", "user") {
            let text = extract_text(m.get("content").unwrap_or(&Value::Null));
            let text = text.trim();
            if !text.is_empty() && is_displayable_user_prompt(text) {
                parts.push(text.to_string());
            }
        } else if let Some(m) = message_of(&record, "assistant", "assistant") {
            let text = extract_assistant_display_text(m.get("content").unwrap_or(&Value::Null));
            let text = text.trim();
            if !text.is_empty() {
                parts.push(text.to_string());
            }
        }
    }
    parts.join("\n")
}

/// When a session last called the API and how long Anthropic keeps its prompt cache after that.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CacheState {
    /// Epoch ms of the last assistant message.
    pub last_request_ms: i64,
    pub ttl_ms: i64,
}

const CACHE_TTL_5M_MS: i64 = 5 * 60 * 1000;
const CACHE_TTL_1H_MS: i64 = 60 * 60 * 1000;
const CACHE_TAIL_BYTES: u64 = 512 * 1024;

static CACHE_STATE_CACHE: MtimeCache<Option<CacheState>> = Mutex::new(None);

/// The session's prompt-cache state, from the newest assistant usage in its transcript. Cached by mtime.
pub fn read_cache_state(path: &Path) -> Option<CacheState> {
    cached(&CACHE_STATE_CACHE, path, || parse_cache_state(path)).flatten()
}

fn parse_cache_state(path: &Path) -> Option<CacheState> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(CACHE_TAIL_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail).ok()?;
    let body = if start > 0 {
        tail.iter()
            .position(|&b| b == b'\n')
            .map_or(&tail[..0], |i| &tail[i + 1..])
    } else {
        &tail[..]
    };
    let mut last_request_ms = None;
    for line in body.split(|&b| b == b'\n').rev() {
        let Ok(record) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        let Some(usage) = message_of(&record, "assistant", "assistant").and_then(|m| m.get("usage")) else {
            continue;
        };
        if last_request_ms.is_none() {
            let stamp = record.get("timestamp").and_then(Value::as_str)?;
            last_request_ms = Some(
                chrono::DateTime::parse_from_rfc3339(stamp)
                    .ok()?
                    .timestamp_millis(),
            );
        }
        let written = |key: &str| {
            usage
                .get("cache_creation")
                .and_then(|c| c.get(key))
                .and_then(Value::as_u64)
                .unwrap_or(0)
        };
        let ttl_ms = if written("ephemeral_1h_input_tokens") > 0 {
            CACHE_TTL_1H_MS
        } else if written("ephemeral_5m_input_tokens") > 0 {
            CACHE_TTL_5M_MS
        } else {
            continue;
        };
        return Some(CacheState {
            last_request_ms: last_request_ms?,
            ttl_ms,
        });
    }
    last_request_ms.map(|last_request_ms| CacheState {
        last_request_ms,
        ttl_ms: CACHE_TTL_5M_MS,
    })
}

/// The last assistant reply's text, for "Copy Last Response". Not cached: a one-off action.
pub fn read_last_assistant_response(path: &Path) -> Option<String> {
    read_records(path)
        .iter()
        .filter_map(|r| message_of(r, "assistant", "assistant"))
        .map(|m| extract_assistant_display_text(m.get("content").unwrap_or(&Value::Null)))
        .map(|t| t.trim().to_string())
        .rfind(|t| !t.is_empty())
}

/// Renames a Claude session that isn't running by appending the records Claude's own `/rename`
/// writes: `custom-title` is what `/resume` and Session Deck list, `agent-name` is what a resumed
/// Claude shows on its input box border. The file belongs to the session's own account dir. A
/// running Claude keeps its title in memory and re-writes it, so rename a running session through
/// Claude itself instead.
pub fn append_claude_rename_records(file: &Path, session_id: &str, name: &str) -> std::io::Result<()> {
    let ends_mid_line = File::open(file)
        .and_then(|mut f| {
            let size = f.metadata()?.len();
            if size == 0 {
                return Ok(false);
            }
            f.seek(SeekFrom::Start(size - 1))?;
            let mut last = [0u8; 1];
            f.read_exact(&mut last)?;
            Ok(last[0] != b'\n')
        })
        .unwrap_or(false);
    let mut out = String::new();
    if ends_mid_line {
        out.push('\n'); // never glue our record onto a partial last line
    }
    for record in [
        json!({"type": "custom-title", "customTitle": name, "sessionId": session_id}),
        json!({"type": "agent-name", "agentName": name, "sessionId": session_id}),
    ] {
        out.push_str(&record.to_string());
        out.push('\n');
    }
    fs::OpenOptions::new()
        .append(true)
        .open(file)?
        .write_all(out.as_bytes())
}

/// A transcript found under one account, with the facts the session list needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeSession {
    pub account: Account,
    pub dir_name: String,
    pub id: String,
    pub file: PathBuf,
    pub cwd: String,
    pub title: String,
    pub mtime_ms: i64,
}

struct Candidate {
    account: Account,
    dir_name: String,
    id: String,
    file: PathBuf,
    mtime_ms: i64,
}

fn to_session(c: &Candidate) -> Option<ClaudeSession> {
    let meta = read_session_meta(&c.file);
    let first_prompt = meta.first_prompt?; // empty/aborted sessions
    let cwd = meta.cwd.unwrap_or_else(|| decode_project_path(&c.dir_name));
    let title = one_line(meta.title.as_deref().unwrap_or(&first_prompt));
    Some(ClaudeSession {
        account: c.account.clone(),
        dir_name: c.dir_name.clone(),
        id: c.id.clone(),
        file: c.file.clone(),
        cwd,
        title,
        mtime_ms: c.mtime_ms,
    })
}

/// The most recent sessions across every account, newest first, capped at `max_sessions`. A
/// project folder whose sessions all fall outside the cap would vanish from the tree with no way
/// to start a new session there, so its single most recent session is kept regardless of the cap.
pub fn discover_claude_sessions(accounts: &[Account], max_sessions: usize) -> Vec<ClaudeSession> {
    let mut candidates: Vec<Candidate> = Vec::new();
    for account in accounts {
        for dir_name in list_project_dir_names(account) {
            for f in list_session_files(account, &dir_name) {
                let file = account.projects_dir().join(&dir_name).join(&f);
                let Some(mtime_ms) = mtime_ms(&file) else {
                    continue; // vanished mid-scan
                };
                candidates.push(Candidate {
                    account: account.clone(),
                    dir_name: dir_name.clone(),
                    id: f.trim_end_matches(".jsonl").to_string(),
                    file,
                    mtime_ms,
                });
            }
        }
    }
    candidates.sort_by_key(|c| std::cmp::Reverse(c.mtime_ms));

    let dir_key = |c: &Candidate| (c.account.config_dir.clone(), c.dir_name.clone());
    let mut found: Vec<ClaudeSession> = Vec::new();
    let mut dirs_seen = HashSet::new();
    for c in &candidates {
        if found.len() >= max_sessions {
            break;
        }
        if let Some(session) = to_session(c) {
            found.push(session);
            dirs_seen.insert(dir_key(c));
        }
    }
    for c in &candidates {
        let key = dir_key(c);
        if dirs_seen.contains(&key) {
            continue;
        }
        // Candidates are in global mtime-desc order, so the first valid one is the most recent.
        if let Some(session) = to_session(c) {
            found.push(session);
            dirs_seen.insert(key);
        }
    }
    found
}

/// The account and transcript of session `id`, wherever it lives; `None` for a brand-new session
/// with nothing written yet.
pub fn find_session_file(accounts: &[Account], id: &str) -> Option<(Account, PathBuf)> {
    accounts.iter().find_map(|account| {
        list_project_dir_names(account).into_iter().find_map(|dir| {
            let file = account.projects_dir().join(dir).join(format!("{id}.jsonl"));
            file.exists().then(|| (account.clone(), file))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_env::EnvGuard;
    use crate::status::account::discover_accounts;
    use std::time::{Duration, SystemTime};

    fn ai_title(t: &str) -> String {
        json!({"type": "ai-title", "aiTitle": t, "sessionId": "s"}).to_string()
    }
    fn custom_title(t: &str) -> String {
        json!({"type": "custom-title", "customTitle": t, "sessionId": "s"}).to_string()
    }
    fn user_line() -> String {
        json!({"type": "user", "message": {"role": "user", "content": "hi"}}).to_string()
    }
    fn title_of(lines: &[String]) -> Option<String> {
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        find_session_title(&refs)
    }

    #[test]
    fn title_returns_the_newest_ai_title() {
        let lines = [
            ai_title("Old title"),
            user_line(),
            ai_title("New title"),
            user_line(),
        ];
        assert_eq!(title_of(&lines).as_deref(), Some("New title"));
    }

    #[test]
    fn title_prefers_rename_over_newer_ai_title() {
        let lines = [custom_title("My name"), ai_title("AI title")];
        assert_eq!(title_of(&lines).as_deref(), Some("My name"));
    }

    #[test]
    fn title_ignores_agent_name_records() {
        let agent = json!({"type": "agent-name", "agentName": "agent", "sessionId": "s"}).to_string();
        assert_eq!(
            title_of(&[ai_title("AI title"), agent]).as_deref(),
            Some("AI title")
        );
    }

    #[test]
    fn title_skips_cut_off_first_line_and_blank_titles() {
        let cut = ai_title("Cut title")[10..].to_string();
        let lines = [cut, ai_title("Real title"), ai_title("   ")];
        assert_eq!(title_of(&lines).as_deref(), Some("Real title"));
    }

    #[test]
    fn title_is_none_without_a_title_record() {
        assert_eq!(title_of(&[user_line(), String::new()]), None);
    }

    #[test]
    fn extract_text_cases() {
        assert_eq!(extract_text(&json!("hello")), "hello");
        let content = json!([
            {"type": "text", "text": "first"},
            {"type": "tool_use", "input": {}},
            {"type": "thinking", "thinking": "second"},
        ]);
        assert_eq!(extract_text(&content), "first\nsecond");
        assert_eq!(extract_text(&json!({"type": "text", "text": "solo"})), "solo");
        for v in [Value::Null, json!(42)] {
            assert_eq!(extract_text(&v), "");
        }
    }

    #[test]
    fn assistant_display_text_keeps_only_text_blocks() {
        let content = json!([
            {"type": "thinking", "thinking": "skip me"},
            {"type": "text", "text": "first"},
            {"type": "tool_use", "input": {}},
            {"type": "text", "text": "second"},
        ]);
        assert_eq!(extract_assistant_display_text(&content), "first\nsecond");
        assert_eq!(extract_assistant_display_text(&json!("plain string")), "");
    }

    #[test]
    fn displayable_prompt_filter() {
        for hidden in [
            "",
            "   ",
            "<command-name>/clear</command-name>",
            "agentId: abc123",
        ] {
            assert!(!is_displayable_user_prompt(hidden), "{hidden:?}");
        }
        assert!(is_displayable_user_prompt("fix the bug in foo.ts"));
    }

    #[test]
    fn decode_project_path_reverses_the_encoding() {
        assert_eq!(decode_project_path("C--Users-me-app"), "C:\\Users\\me\\app");
    }

    fn transcript(cwd: &str, prompt: &str) -> String {
        let user = json!({"type": "user", "cwd": cwd, "message": {"role": "user", "content": prompt}});
        let reply = json!({
            "type": "assistant",
            "message": {"role": "assistant", "content": [{"type": "text", "text": "answer"}]}
        });
        format!("{user}\n{reply}\n")
    }

    fn put(account: &Account, dir: &str, id: &str, body: &str, age_secs: u64) -> PathBuf {
        let folder = account.projects_dir().join(dir);
        fs::create_dir_all(&folder).unwrap();
        let file = folder.join(format!("{id}.jsonl"));
        fs::write(&file, body).unwrap();
        let when = SystemTime::now() - Duration::from_secs(age_secs);
        File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(when)
            .unwrap();
        file
    }

    fn two_accounts() -> (EnvGuard, tempfile::TempDir, Vec<Account>) {
        let g = EnvGuard::new();
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("SDECK_USER_HOME", home.path());
        let login = |email: &str| format!(r#"{{"oauthAccount":{{"emailAddress":"{email}"}}}}"#);
        fs::write(home.path().join(".claude.json"), login("work@x.com")).unwrap();
        fs::create_dir_all(home.path().join(".claude")).unwrap();
        fs::create_dir_all(home.path().join(".claude-me")).unwrap();
        fs::write(home.path().join(".claude-me/.claude.json"), login("me@x.com")).unwrap();
        let accounts = discover_accounts();
        (g, home, accounts)
    }

    fn assistant_at(stamp: &str, created_5m: u64, created_1h: u64) -> String {
        json!({
            "type": "assistant",
            "timestamp": stamp,
            "message": {"role": "assistant", "usage": {"cache_creation": {
                "ephemeral_5m_input_tokens": created_5m,
                "ephemeral_1h_input_tokens": created_1h
            }}}
        })
        .to_string()
    }

    fn cache_state_of(lines: &[String]) -> Option<CacheState> {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        fs::write(&file, lines.join("\n")).unwrap();
        parse_cache_state(&file)
    }

    #[test]
    fn cache_state_takes_the_ttl_of_the_newest_cache_write_and_the_time_of_the_last_reply() {
        let state = cache_state_of(&[
            assistant_at("2026-10-04T10:00:00.000Z", 900, 0),
            assistant_at("2026-10-04T10:05:00.000Z", 0, 0),
        ])
        .unwrap();
        assert_eq!(state.ttl_ms, CACHE_TTL_5M_MS);
        assert_eq!(state.last_request_ms, 1_791_108_300_000);

        let state = cache_state_of(&[
            assistant_at("2026-10-04T10:00:00.000Z", 0, 900),
            assistant_at("2026-10-04T10:05:00.000Z", 0, 0),
        ])
        .unwrap();
        assert_eq!(state.ttl_ms, CACHE_TTL_1H_MS);
    }

    #[test]
    fn cache_state_defaults_to_five_minutes_and_is_none_without_a_reply() {
        let state = cache_state_of(&[assistant_at("2026-10-04T10:00:00.000Z", 0, 0)]).unwrap();
        assert_eq!(state.ttl_ms, CACHE_TTL_5M_MS);
        assert_eq!(cache_state_of(&[transcript("C:\\a", "hi")]), None);
    }

    #[test]
    fn meta_reads_cwd_first_prompt_and_title() {
        let (_g, _home, accounts) = two_accounts();
        let body = format!(
            "{}{}\n",
            transcript("C:\\proj", "  fix   the\nbug "),
            ai_title("Fix it")
        );
        let file = put(&accounts[0], "C--proj", "a", &body, 0);
        let meta = read_session_meta(&file);
        assert_eq!(meta.cwd.as_deref(), Some("C:\\proj"));
        assert_eq!(meta.first_prompt.as_deref(), Some("fix the bug"));
        assert_eq!(meta.title.as_deref(), Some("Fix it"));
        assert_eq!(read_session_search_text(&file), "fix   the\nbug\nanswer");
        assert_eq!(read_last_assistant_response(&file).as_deref(), Some("answer"));
    }

    #[test]
    fn finds_sessions_of_both_accounts_tagged_and_newest_first() {
        let (_g, _home, accounts) = two_accounts();
        put(
            &accounts[0],
            "C--proj",
            "old",
            &transcript("C:\\proj", "one"),
            100,
        );
        put(&accounts[1], "C--proj", "new", &transcript("C:\\proj", "two"), 10);
        put(&accounts[1], "C--empty", "blank", "{\"type\":\"summary\"}\n", 5);
        let found = discover_claude_sessions(&accounts, 30);
        let ids: Vec<_> = found.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["new", "old"]);
        assert_eq!(found[0].account.email.as_deref(), Some("me@x.com"));
        assert_eq!(found[1].account.email.as_deref(), Some("work@x.com"));
        assert_eq!(found[0].cwd, found[1].cwd);
    }

    #[test]
    fn cap_keeps_each_project_folders_newest_session() {
        let (_g, _home, accounts) = two_accounts();
        put(&accounts[0], "C--a", "a1", &transcript("C:\\a", "p"), 10);
        put(&accounts[0], "C--a", "a2", &transcript("C:\\a", "p"), 20);
        put(&accounts[0], "C--b", "b1", &transcript("C:\\b", "p"), 30);
        put(&accounts[0], "C--b", "b2", &transcript("C:\\b", "p"), 40);
        let ids: Vec<_> = discover_claude_sessions(&accounts, 1)
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(ids, ["a1", "b1"]);
    }

    #[test]
    fn rename_appends_records_in_the_sessions_own_account_dir() {
        let (_g, _home, accounts) = two_accounts();
        let file = put(&accounts[1], "C--proj", "s1", "{\"type\":\"user\"", 0);
        let (account, found) = find_session_file(&accounts, "s1").unwrap();
        assert_eq!(account.email.as_deref(), Some("me@x.com"));
        assert_eq!(found, file);
        append_claude_rename_records(&file, "s1", "New name").unwrap();
        let text = fs::read_to_string(&file).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(find_session_title(&lines).as_deref(), Some("New name"));
        assert!(lines[2].contains("\"agent-name\""));
        assert!(find_session_file(&accounts, "missing").is_none());
    }
}
