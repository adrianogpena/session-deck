use std::fs;
use std::io;
use std::path::Path;
use std::thread;
use std::time::Duration;

use crate::format::now_ms;

const RENAME_RETRIES: u32 = 10;
const RENAME_RETRY_MS: u64 = 20;

/// On Windows a rename fails (access denied / sharing violation) while another process has the target
/// open for a read; that clears within milliseconds.
fn is_transient(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::PermissionDenied || matches!(err.raw_os_error(), Some(5) | Some(32))
}

/// Writes `contents` to `path` via a temp file in the same folder plus a rename, so readers never see
/// a half-written file. Creates the parent folder.
pub fn write_atomic(path: &Path, contents: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut tmp_name = path.as_os_str().to_owned();
    tmp_name.push(format!(".{}.{}.tmp", std::process::id(), now_ms()));
    let tmp = std::path::PathBuf::from(tmp_name);
    fs::write(&tmp, contents)?;
    let mut attempt = 0;
    loop {
        match fs::rename(&tmp, path) {
            Ok(()) => return Ok(()),
            Err(err) if attempt < RENAME_RETRIES && is_transient(&err) => {
                attempt += 1;
                thread::sleep(Duration::from_millis(RENAME_RETRY_MS));
            }
            Err(err) => {
                let _ = fs::remove_file(&tmp);
                return Err(err);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_creates_the_folder_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a").join("b.json");
        write_atomic(&file, "one").unwrap();
        write_atomic(&file, "two").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "two");
        let names: Vec<_> = fs::read_dir(file.parent().unwrap()).unwrap().collect();
        assert_eq!(names.len(), 1);
    }
}
