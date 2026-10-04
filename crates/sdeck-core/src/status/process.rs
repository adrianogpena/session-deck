/// Whether `pid` still refers to a live process. A `<pid>.json` file isn't guaranteed to be cleaned
/// up on exit, so a stale file must read as "gone".
#[cfg(windows)]
pub fn is_process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ACCESS_DENIED, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    // SAFETY: plain Win32 calls; the handle is closed before returning.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            // Access denied means the process exists but belongs to someone else.
            return GetLastError() == ERROR_ACCESS_DENIED;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(handle, &mut code) != 0;
        CloseHandle(handle);
        ok && code == STILL_ACTIVE as u32
    }
}

#[cfg(unix)]
pub fn is_process_alive(pid: u32) -> bool {
    // SAFETY: signal 0 only probes for existence.
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    #[test]
    fn own_process_is_alive() {
        assert!(is_process_alive(std::process::id()));
    }

    #[test]
    fn an_exited_child_is_gone() {
        let mut child = Command::new("git")
            .arg("--version")
            .stdout(Stdio::null())
            .spawn()
            .expect("git runs");
        let pid = child.id();
        child.wait().unwrap();
        assert!(!is_process_alive(pid));
    }
}
