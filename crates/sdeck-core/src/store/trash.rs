use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::atomic::write_atomic;
use crate::commands::session_id::{assert_safe_session_id, is_safe_session_id};
use crate::format::now_ms;
use crate::paths::deck_home;

/// A deleted session, kept restorable in `~/.session-deck/trash/` until it's older than the retention.
#[derive(Debug, Clone, PartialEq)]
pub struct TrashedSession {
    pub session_id: String,
    pub agent: String,
    pub title: String,
    /// Where the transcript lived, so it can go back there.
    pub original_path: String,
    pub trashed_at: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum TrashError {
    #[error("That session is no longer in the trash.")]
    NotInTrash,
    #[error("A transcript with the same id already exists where this one was.")]
    AlreadyExists,
    #[error("{0}")]
    UnsafeId(String),
    #[error(transparent)]
    Io(#[from] io::Error),
}

pub const TRASH_TTL_MS: i64 = 30 * 24 * 60 * 60 * 1000;

pub fn trash_dir() -> PathBuf {
    deck_home().join("trash")
}

fn manifest_path() -> PathBuf {
    trash_dir().join("manifest.json")
}

/// Claude keeps a folder named after the session next to some transcripts (tool results, subagent logs).
fn companion_dir(transcript_path: &Path) -> PathBuf {
    let s = transcript_path.to_string_lossy();
    PathBuf::from(s.strip_suffix(".jsonl").unwrap_or(&s).to_string())
}

fn safe_id(session_id: &str) -> Result<&str, TrashError> {
    assert_safe_session_id(session_id).map_err(TrashError::UnsafeId)
}

fn trashed_transcript(session_id: &str) -> Result<PathBuf, TrashError> {
    Ok(trash_dir().join(format!("{}.jsonl", safe_id(session_id)?)))
}

/// Where a session's companion folder lives once trashed.
fn trashed_companion_dir(session_id: &str) -> Result<PathBuf, TrashError> {
    Ok(trash_dir().join(safe_id(session_id)?))
}

fn entry_to_value(e: &TrashedSession) -> Value {
    let mut m = Map::new();
    m.insert("sessionId".into(), e.session_id.clone().into());
    m.insert("agent".into(), e.agent.clone().into());
    m.insert("title".into(), e.title.clone().into());
    m.insert("originalPath".into(), e.original_path.clone().into());
    m.insert("trashedAt".into(), e.trashed_at.into());
    Value::Object(m)
}

fn entry_from_value(v: &Value) -> Option<TrashedSession> {
    Some(TrashedSession {
        session_id: v.get("sessionId")?.as_str()?.to_string(),
        original_path: v.get("originalPath")?.as_str()?.to_string(),
        trashed_at: v.get("trashedAt")?.as_f64()? as i64,
        agent: v
            .get("agent")
            .and_then(Value::as_str)
            .unwrap_or("claude")
            .to_string(),
        title: v
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    })
}

/// Newest first. Tolerant of a missing or damaged manifest (reads as empty).
pub fn list_trash() -> Vec<TrashedSession> {
    let Ok(raw) = fs::read_to_string(manifest_path()) else {
        return vec![];
    };
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&raw) else {
        return vec![];
    };
    let mut entries: Vec<TrashedSession> = items.iter().filter_map(entry_from_value).collect();
    entries.sort_by_key(|e| std::cmp::Reverse(e.trashed_at));
    entries
}

fn write_trash(entries: &[TrashedSession]) -> io::Result<()> {
    let value = Value::Array(entries.iter().map(entry_to_value).collect());
    write_atomic(
        &manifest_path(),
        &format!(
            "{}\n",
            serde_json::to_string_pretty(&value).expect("manifest serializes")
        ),
    )
}

fn copy_recursive(from: &Path, to: &Path) -> io::Result<()> {
    if from.is_dir() {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(from, to).map(|_| ())
    }
}

fn remove_any(path: &Path) -> io::Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

/// Rename, falling back to copy + remove across drives.
fn move_path(from: &Path, to: &Path) -> io::Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::CrossesDevices || err.raw_os_error() == Some(17) => {
            copy_recursive(from, to)?;
            remove_any(from)
        }
        Err(err) => Err(err),
    }
}

/// Moves a Claude session's transcript (and its companion folder, if any) into the trash.
pub fn trash_claude_session(
    transcript_path: &Path,
    session_id: &str,
    title: &str,
) -> Result<TrashedSession, TrashError> {
    let entry = TrashedSession {
        session_id: session_id.to_string(),
        agent: "claude".to_string(),
        title: title.to_string(),
        original_path: transcript_path.to_string_lossy().into_owned(),
        trashed_at: now_ms(),
    };
    move_path(transcript_path, &trashed_transcript(session_id)?)?;
    let companion = companion_dir(transcript_path);
    if companion.exists() {
        move_path(&companion, &trashed_companion_dir(session_id)?)?;
    }
    let mut entries = vec![entry.clone()];
    entries.extend(list_trash().into_iter().filter(|e| e.session_id != session_id));
    write_trash(&entries)?;
    Ok(entry)
}

/// Puts a trashed session back where it was. Refuses rather than overwrite a transcript that's there again.
pub fn restore_session(session_id: &str) -> Result<TrashedSession, TrashError> {
    let entries = list_trash();
    let entry = entries
        .iter()
        .find(|e| e.session_id == session_id)
        .cloned()
        .ok_or(TrashError::NotInTrash)?;
    let original = PathBuf::from(&entry.original_path);
    if original.exists() {
        return Err(TrashError::AlreadyExists);
    }
    move_path(&trashed_transcript(session_id)?, &original)?;
    let companion = trashed_companion_dir(session_id)?;
    if companion.exists() {
        move_path(&companion, &companion_dir(&original))?;
    }
    let remaining: Vec<TrashedSession> = entries.into_iter().filter(|e| *e != entry).collect();
    write_trash(&remaining)?;
    Ok(entry)
}

/// Permanently removes trashed sessions older than `ttl_ms`. Returns how many were removed.
pub fn purge_trash(now_ms: i64, ttl_ms: i64) -> Result<usize, TrashError> {
    let entries = list_trash();
    // An entry with an unsafe id is left in place instead of aborting the whole purge: this runs
    // unguarded at startup, so one malformed manifest entry must not stop the rest.
    let (expired, kept): (Vec<_>, Vec<_>) = entries
        .into_iter()
        .partition(|e| now_ms - e.trashed_at > ttl_ms && is_safe_session_id(&e.session_id));
    if expired.is_empty() {
        return Ok(0);
    }
    for e in &expired {
        let _ = fs::remove_file(trashed_transcript(&e.session_id)?);
        let _ = fs::remove_dir_all(trashed_companion_dir(&e.session_id)?);
    }
    write_trash(&kept)?;
    Ok(expired.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_env::EnvGuard;

    /// A fake `~/.claude/projects/<dir>/<id>.jsonl`, with Claude's companion folder when asked.
    fn fake_transcript(root: &Path, id: &str, with_companion: bool) -> PathBuf {
        let dir = root.join("projects");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join(format!("{id}.jsonl"));
        fs::write(&file, "{\"type\":\"user\"}\n").unwrap();
        if with_companion {
            fs::create_dir_all(dir.join(id)).unwrap();
            fs::write(dir.join(id).join("tool-result.txt"), "x").unwrap();
        }
        file
    }

    fn setup() -> (EnvGuard, tempfile::TempDir) {
        let guard = EnvGuard::new();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("SESSION_DECK_HOME", dir.path().join("deck"));
        (guard, dir)
    }

    #[test]
    fn trash_moves_the_transcript_and_companion_out_and_restore_puts_both_back() {
        let (_g, dir) = setup();
        let file = fake_transcript(dir.path(), "t1", true);
        trash_claude_session(&file, "t1", "My session").unwrap();
        assert!(!file.exists());
        assert!(!companion_dir(&file).exists());
        let listed: Vec<_> = list_trash()
            .into_iter()
            .map(|e| (e.session_id, e.title))
            .collect();
        assert_eq!(listed, vec![("t1".to_string(), "My session".to_string())]);

        restore_session("t1").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "{\"type\":\"user\"}\n");
        assert!(companion_dir(&file).join("tool-result.txt").exists());
        assert!(list_trash().is_empty());
    }

    #[test]
    fn restore_refuses_to_overwrite_a_transcript_that_exists_again() {
        let (_g, dir) = setup();
        let file = fake_transcript(dir.path(), "t2", false);
        trash_claude_session(&file, "t2", "x").unwrap();
        fs::write(&file, "new").unwrap();
        let err = restore_session("t2").unwrap_err();
        assert!(err.to_string().contains("already exists"));
        assert_eq!(fs::read_to_string(&file).unwrap(), "new");
        assert!(list_trash().iter().any(|e| e.session_id == "t2"));
    }

    #[test]
    fn restore_of_an_unknown_id_says_it_is_gone() {
        let (_g, _dir) = setup();
        assert!(matches!(restore_session("nope"), Err(TrashError::NotInTrash)));
    }

    #[test]
    fn purge_removes_only_entries_older_than_the_ttl() {
        let (_g, dir) = setup();
        let file = fake_transcript(dir.path(), "t3", false);
        let entry = trash_claude_session(&file, "t3", "old").unwrap();
        assert_eq!(
            purge_trash(entry.trashed_at + TRASH_TTL_MS - 1, TRASH_TTL_MS).unwrap(),
            0
        );
        assert!(list_trash().iter().any(|e| e.session_id == "t3"));
        assert_eq!(
            purge_trash(entry.trashed_at + TRASH_TTL_MS + 1, TRASH_TTL_MS).unwrap(),
            1
        );
        assert!(list_trash().is_empty());
        let leftovers = fs::read_dir(trash_dir())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".jsonl"))
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn purge_skips_a_manifest_entry_with_an_unsafe_id_instead_of_failing() {
        let (_g, _dir) = setup();
        fs::create_dir_all(trash_dir()).unwrap();
        fs::write(
            manifest_path(),
            r#"[{"sessionId":"../evil","originalPath":"x","trashedAt":1}]"#,
        )
        .unwrap();
        assert_eq!(purge_trash(i64::MAX / 2, TRASH_TTL_MS).unwrap(), 0);
        assert_eq!(list_trash().len(), 1);
    }

    #[test]
    fn list_is_newest_first_and_tolerates_a_damaged_manifest() {
        let (_g, _dir) = setup();
        assert!(list_trash().is_empty());
        fs::create_dir_all(trash_dir()).unwrap();
        fs::write(manifest_path(), "{ not json").unwrap();
        assert!(list_trash().is_empty());
        fs::write(
            manifest_path(),
            r#"[{"sessionId":"a","originalPath":"x","trashedAt":1},{"sessionId":"b","originalPath":"y","trashedAt":5},{"bad":true}]"#,
        )
        .unwrap();
        let ids: Vec<_> = list_trash().into_iter().map(|e| e.session_id).collect();
        assert_eq!(ids, vec!["b", "a"]);
    }
}
