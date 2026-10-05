use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

use super::session_status::SessionStatus;
use super::waiting_notifier::Transition;

/// The config key of the hook for a status.
pub fn hook_key(status: SessionStatus) -> &'static str {
    match status {
        SessionStatus::Running => "onRunning",
        SessionStatus::Waiting => "onWaiting",
        SessionStatus::Done => "onDone",
        SessionStatus::Error => "onError",
    }
}

/// Starts the user's hook line for a transition through the platform shell and returns at once.
/// Fire and forget: output is discarded, and a thread reaps the child.
pub fn run_hook(line: &str, transition: &Transition) -> io::Result<()> {
    let mut command = shell(line);
    command
        .env("SDECK_SESSION_ID", &transition.session_id)
        .env("SDECK_STATUS", transition.status.as_str())
        .env("SDECK_LABEL", &transition.label)
        .env("SDECK_PROJECT", transition.project.as_deref().unwrap_or(""))
        .env("SDECK_CWD", &transition.project_root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if !transition.project_root.is_empty() && Path::new(&transition.project_root).is_dir() {
        command.current_dir(&transition.project_root);
    }
    let mut child = command.spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(windows)]
fn shell(line: &str) -> Command {
    use std::os::windows::process::CommandExt;
    let mut command = Command::new("cmd");
    command.arg("/C").raw_arg(line);
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    command
}

#[cfg(not(windows))]
fn shell(line: &str) -> Command {
    let mut command = Command::new("sh");
    command.arg("-c").arg(line);
    command
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn a_hook_receives_the_five_variables_and_the_project_root_as_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.txt");
        #[cfg(windows)]
        let line = format!(
            "(echo %SDECK_SESSION_ID%& echo %SDECK_STATUS%& echo %SDECK_LABEL%& echo %SDECK_PROJECT%& echo %SDECK_CWD%& cd) > \"{}\"",
            out.display()
        );
        #[cfg(not(windows))]
        let line = format!(
            "printf '%s\n' \"$SDECK_SESSION_ID\" \"$SDECK_STATUS\" \"$SDECK_LABEL\" \"$SDECK_PROJECT\" \"$SDECK_CWD\" \"$(pwd)\" > '{}'",
            out.display()
        );
        let root = dir.path().to_string_lossy().to_string();
        let transition = Transition {
            session_id: "abc".into(),
            status: SessionStatus::Waiting,
            label: "fix".into(),
            project: Some("proj".into()),
            project_root: root.clone(),
            at: 1,
        };
        run_hook(&line, &transition).unwrap();

        let deadline = Instant::now() + Duration::from_secs(10);
        let text = loop {
            if let Ok(text) = std::fs::read_to_string(&out) {
                if text.lines().count() >= 6 {
                    break text;
                }
            }
            assert!(Instant::now() < deadline, "hook produced no output");
            std::thread::sleep(Duration::from_millis(50));
        };
        let lines: Vec<&str> = text.lines().map(str::trim).collect();
        assert_eq!(&lines[..4], ["abc", "waiting", "fix", "proj"]);
        assert_eq!(lines[4], root);
        assert_eq!(
            std::fs::canonicalize(lines[5]).unwrap(),
            std::fs::canonicalize(&root).unwrap()
        );
    }

    #[test]
    fn each_status_has_its_own_key() {
        let keys: Vec<&str> = SessionStatus::ALL.iter().map(|s| hook_key(*s)).collect();
        assert_eq!(keys, ["onRunning", "onWaiting", "onDone", "onError"]);
    }
}
