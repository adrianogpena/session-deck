use std::path::Path;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};

/// Keeps a file or directory watch alive; dropping it stops the watch.
pub struct PathWatch {
    _watcher: notify::RecommendedWatcher,
}

const DEBOUNCE: Duration = Duration::from_millis(50);

/// Calls `listener` (debounced) on a background thread whenever anything at `path` changes.
pub fn watch_path(path: &Path, listener: impl Fn() + Send + 'static) -> notify::Result<PathWatch> {
    let (tx, rx) = mpsc::channel::<()>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if res.is_ok() {
            let _ = tx.send(());
        }
    })?;
    watcher.watch(path, RecursiveMode::NonRecursive)?;
    thread::spawn(move || {
        // The sender lives in the watcher; dropping the handle disconnects the channel and ends this thread.
        while rx.recv().is_ok() {
            loop {
                match rx.recv_timeout(DEBOUNCE) {
                    Ok(()) => continue,
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            listener();
        }
    });
    Ok(PathWatch { _watcher: watcher })
}
