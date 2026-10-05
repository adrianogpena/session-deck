use std::collections::HashMap;
use std::sync::Arc;

use super::alert_log::{append_alert, AlertEntry};
use super::session_status::{claim_notification, read_effective_session_status, SessionStatus};
use crate::store::deck_store::DeckStore;

pub struct WatchedSession {
    pub session_id: String,
    pub label: String,
    /// Shown as the toast's title. Falls back to "Session Deck" when `None`.
    pub project: Option<String>,
    /// The session's project root, the working directory of its hooks.
    pub project_root: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    pub title: String,
    pub message: String,
}

/// Called with the session id when the user clicks its toast.
pub type OnActivated = Arc<dyn Fn(&str) + Send + Sync>;

/// Shows a desktop toast; `on_activated` runs (on any thread) if the user clicks it.
pub trait ToastSender {
    fn send(&self, toast: Toast, on_activated: Box<dyn Fn() + Send + Sync>);
}

impl<T: ToastSender + ?Sized> ToastSender for Box<T> {
    fn send(&self, toast: Toast, on_activated: Box<dyn Fn() + Send + Sync>) {
        (**self).send(toast, on_activated);
    }
}

pub struct CheckOptions<'a> {
    /// Statuses worth a toast (`config.ui.notify_statuses`).
    pub statuses: &'a [SessionStatus],
    /// Skip a session the user is already looking at (attached or interacting).
    pub skip: &'a dyn Fn(&str) -> bool,
    /// Toasts are suppressed while the terminal has focus; the transition is still logged to the
    /// alert history. The point of a toast is to reach you when you're not already looking.
    pub focused: bool,
}

fn message(status: SessionStatus, label: &str) -> String {
    let prefix = match status {
        SessionStatus::Waiting => "Waiting",
        SessionStatus::Done => "Done",
        SessionStatus::Error => "Error",
        SessionStatus::Running => "Running",
    };
    format!("{prefix}: {label}")
}

/// A watched session's status changed since the last [`StatusTransitions::update`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub session_id: String,
    pub status: SessionStatus,
    pub label: String,
    pub project: Option<String>,
    pub project_root: String,
    /// The status record's `updated_at`.
    pub at: i64,
}

/// Finds status transitions of watched sessions: not repeatedly, and not on first sight (opening
/// onto an already-waiting session is no transition). A session that leaves the watched list and
/// returns counts as first sight again.
#[derive(Default)]
pub struct StatusTransitions {
    last_status: HashMap<String, Option<SessionStatus>>,
}

impl StatusTransitions {
    pub fn update(&mut self, store: &DeckStore, sessions: &[WatchedSession]) -> Vec<Transition> {
        self.last_status
            .retain(|id, _| sessions.iter().any(|s| &s.session_id == id));

        let mut transitions = Vec::new();
        for session in sessions {
            let id = session.session_id.as_str();
            let record = read_effective_session_status(store, id);
            let current = record.map(|r| r.status);
            let previous = self.last_status.insert(id.to_string(), current);
            let Some(record) = record else {
                continue;
            };
            // `None` previous means first sight; `Some(prev)` equal to current means no transition.
            if previous.is_none_or(|prev| prev == current) {
                continue;
            }
            transitions.push(Transition {
                session_id: id.to_string(),
                status: record.status,
                label: session.label.clone(),
                project: session.project.clone(),
                project_root: session.project_root.clone(),
                at: record.updated_at,
            });
        }
        transitions
    }
}

/// Fires a desktop toast when a watched session transitions into one of the notified statuses.
/// [`claim_notification`] keeps two front ends from both notifying the same event.
pub struct WaitingNotifier<S: ToastSender> {
    transitions: StatusTransitions,
    sender: S,
}

impl<S: ToastSender> WaitingNotifier<S> {
    pub fn new(sender: S) -> Self {
        WaitingNotifier {
            transitions: StatusTransitions::default(),
            sender,
        }
    }

    /// Finds the transitions itself; [`Self::notify`] is for a caller that tracks them.
    pub fn check(
        &mut self,
        store: &DeckStore,
        sessions: &[WatchedSession],
        options: &CheckOptions<'_>,
        on_activated: &OnActivated,
    ) {
        let transitions = self.transitions.update(store, sessions);
        self.notify(&transitions, options, on_activated);
    }

    pub fn notify(&self, transitions: &[Transition], options: &CheckOptions<'_>, on_activated: &OnActivated) {
        for t in transitions {
            let id = t.session_id.as_str();
            // Logged whether or not a toast fires: a history of what happened, not just of what you
            // were interrupted for.
            append_alert(&AlertEntry {
                session_id: t.session_id.clone(),
                status: t.status,
                label: t.label.clone(),
                project: t.project.clone(),
                at: t.at,
            });
            if !options.statuses.contains(&t.status)
                || (options.skip)(id)
                || options.focused
                || !claim_notification(id, t.at)
            {
                continue;
            }
            let toast = Toast {
                title: t.project.clone().unwrap_or_else(|| "Session Deck".to_string()),
                message: message(t.status, &t.label),
            };
            let (callback, id) = (Arc::clone(on_activated), id.to_string());
            self.sender.send(toast, Box::new(move || callback(&id)));
        }
    }
}

#[cfg(windows)]
pub use windows_toast::WindowsToastSender;

#[cfg(windows)]
mod windows_toast {
    use std::fs;
    use std::path::PathBuf;

    use tauri_winrt_notification::{IconCrop, Toast as WinToast};

    use super::{Toast, ToastSender};
    use crate::paths;

    const APP_ID: &str = "SessionDeck.Sdeck";
    const ICON_PNG: &[u8] = include_bytes!("../../assets/icon.png");

    /// Windows toasts via WinRT. The app id is registered per user under
    /// `HKCU\Software\Classes\AppUserModelId` (no admin, no Start-menu shortcut), so toasts show
    /// "Session Deck" with its icon. Clicks are reported while this process is alive.
    pub struct WindowsToastSender {
        icon: Option<PathBuf>,
    }

    impl WindowsToastSender {
        pub fn new() -> Self {
            let icon = write_icon();
            register_app_id(icon.as_deref());
            WindowsToastSender { icon }
        }
    }

    impl Default for WindowsToastSender {
        fn default() -> Self {
            Self::new()
        }
    }

    /// The toast API needs the icon as a file: kept at `<deck home>/icon.png`.
    fn write_icon() -> Option<PathBuf> {
        let path = paths::deck_home().join("icon.png");
        if fs::read(&path).ok().as_deref() != Some(ICON_PNG) {
            fs::create_dir_all(path.parent()?).ok()?;
            fs::write(&path, ICON_PNG).ok()?;
        }
        Some(path)
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn register_app_id(icon: Option<&std::path::Path>) {
        use windows_sys::Win32::System::Registry::{RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};
        let key = wide(&format!("Software\\Classes\\AppUserModelId\\{APP_ID}"));
        let mut values = vec![("DisplayName", "Session Deck".to_string())];
        if let Some(icon) = icon {
            values.push(("IconUri", icon.display().to_string()));
        }
        for (name, value) in values {
            let (name, value) = (wide(name), wide(&value));
            // Best-effort: an unregistered id still shows the toast, just without the friendly name.
            // SAFETY: every pointer is a NUL-terminated UTF-16 buffer alive for the call.
            unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    key.as_ptr(),
                    name.as_ptr(),
                    REG_SZ,
                    value.as_ptr().cast(),
                    (value.len() * 2) as u32,
                );
            }
        }
    }

    impl ToastSender for WindowsToastSender {
        fn send(&self, toast: Toast, on_activated: Box<dyn Fn() + Send + Sync>) {
            let mut win = WinToast::new(APP_ID).title(&toast.title).text1(&toast.message);
            if let Some(icon) = &self.icon {
                win = win.icon(icon, IconCrop::Square, "");
            }
            let _ = win
                .on_activated(move |_| {
                    on_activated();
                    Ok(())
                })
                .show();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_env::EnvGuard;
    use crate::status::alert_log::read_alerts;
    use crate::status::session_status::write_session_status;
    use std::sync::Mutex;
    use std::thread;
    use std::time::Duration;

    type Sent = Arc<Mutex<Vec<(Toast, Box<dyn Fn() + Send + Sync>)>>>;

    #[derive(Default, Clone)]
    struct FakeSender {
        sent: Sent,
    }

    impl ToastSender for FakeSender {
        fn send(&self, toast: Toast, on_activated: Box<dyn Fn() + Send + Sync>) {
            self.sent.lock().unwrap().push((toast, on_activated));
        }
    }

    struct Fixture {
        _g: EnvGuard,
        _tmp: tempfile::TempDir,
        store: DeckStore,
        sent: Sent,
        notifier: WaitingNotifier<FakeSender>,
        activated: Arc<Mutex<Vec<String>>>,
    }

    fn fixture() -> Fixture {
        let g = EnvGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("SESSION_DECK_STATUS_DIR", tmp.path().join("status"));
        let store = DeckStore::new(tmp.path().join("state.json"));
        let sender = FakeSender::default();
        Fixture {
            _g: g,
            _tmp: tmp,
            store,
            sent: Arc::clone(&sender.sent),
            notifier: WaitingNotifier::new(sender),
            activated: Arc::default(),
        }
    }

    const DEFAULT: [SessionStatus; 2] = [SessionStatus::Waiting, SessionStatus::Error];

    fn watched(id: &str, project: Option<&str>) -> WatchedSession {
        WatchedSession {
            session_id: id.to_string(),
            label: format!("label {id}"),
            project: project.map(str::to_string),
            project_root: String::new(),
        }
    }

    impl Fixture {
        fn check_with(&mut self, sessions: &[WatchedSession], skip: &dyn Fn(&str) -> bool, focused: bool) {
            let activated = Arc::clone(&self.activated);
            let on_activated: OnActivated =
                Arc::new(move |id| activated.lock().unwrap().push(id.to_string()));
            let options = CheckOptions {
                statuses: &DEFAULT,
                skip,
                focused,
            };
            self.notifier
                .check(&self.store, sessions, &options, &on_activated);
        }

        fn check(&mut self, sessions: &[WatchedSession]) {
            self.check_with(sessions, &|_| false, false);
        }

        fn toasts(&self) -> Vec<Toast> {
            self.sent.lock().unwrap().iter().map(|(t, _)| t.clone()).collect()
        }
    }

    fn set(id: &str, status: SessionStatus) {
        thread::sleep(Duration::from_millis(3));
        write_session_status(id, status).unwrap();
    }

    #[test]
    fn first_sight_is_silent_and_a_transition_to_waiting_notifies() {
        let mut f = fixture();
        let sessions = [watched("s1", Some("deck"))];
        set("s1", SessionStatus::Waiting);
        f.check(&sessions);
        assert!(f.toasts().is_empty());

        set("s1", SessionStatus::Running);
        f.check(&sessions);
        set("s1", SessionStatus::Waiting);
        f.check(&sessions);
        assert_eq!(
            f.toasts(),
            [Toast {
                title: "deck".to_string(),
                message: "Waiting: label s1".to_string()
            }]
        );
        f.check(&sessions);
        assert_eq!(f.toasts().len(), 1, "no repeat without a new transition");
    }

    #[test]
    fn title_falls_back_and_click_reports_the_session() {
        let mut f = fixture();
        let sessions = [watched("s2", None)];
        f.check(&sessions);
        set("s2", SessionStatus::Error);
        f.check(&sessions);
        assert_eq!(f.toasts()[0].title, "Session Deck");
        assert_eq!(f.toasts()[0].message, "Error: label s2");
        (f.sent.lock().unwrap()[0].1)();
        assert_eq!(*f.activated.lock().unwrap(), ["s2"]);
    }

    #[test]
    fn non_notified_statuses_are_logged_but_not_toasted() {
        let mut f = fixture();
        let sessions = [watched("s3", None)];
        f.check(&sessions);
        set("s3", SessionStatus::Running);
        f.check(&sessions);
        assert!(f.toasts().is_empty());
        let alerts = read_alerts(10);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].status, SessionStatus::Running);
    }

    #[test]
    fn skipped_or_focused_sessions_log_without_a_toast() {
        let mut f = fixture();
        let sessions = [watched("s4", None)];
        f.check(&sessions);
        set("s4", SessionStatus::Waiting);
        f.check_with(&sessions, &|id| id == "s4", false);
        set("s4", SessionStatus::Running);
        f.check(&sessions);
        set("s4", SessionStatus::Error);
        f.check_with(&sessions, &|_| false, true);
        assert!(f.toasts().is_empty());
        assert_eq!(read_alerts(10).len(), 3);
    }

    #[test]
    fn a_claimed_notification_is_not_toasted_twice() {
        let mut f = fixture();
        let sessions = [watched("s5", None)];
        f.check(&sessions);
        set("s5", SessionStatus::Waiting);
        let at = crate::status::session_status::read_session_status("s5")
            .unwrap()
            .updated_at;
        assert!(claim_notification("s5", at), "another front end claims it first");
        f.check(&sessions);
        assert!(f.toasts().is_empty());
    }

    #[test]
    fn transitions_are_found_without_a_notifier() {
        let f = fixture();
        let mut transitions = StatusTransitions::default();
        let sessions = [watched("s7", Some("deck"))];
        set("s7", SessionStatus::Running);
        assert!(transitions.update(&f.store, &sessions).is_empty());
        set("s7", SessionStatus::Waiting);
        let found = transitions.update(&f.store, &sessions);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].session_id, "s7");
        assert_eq!(found[0].status, SessionStatus::Waiting);
        assert_eq!(found[0].label, "label s7");
        assert_eq!(found[0].project.as_deref(), Some("deck"));
        assert!(transitions.update(&f.store, &sessions).is_empty());
        assert!(read_alerts(10).is_empty());
    }

    #[test]
    fn a_session_that_leaves_and_returns_counts_as_first_sight() {
        let mut f = fixture();
        f.check(&[watched("s6", None)]);
        f.check(&[]);
        set("s6", SessionStatus::Waiting);
        f.check(&[watched("s6", None)]);
        assert!(f.toasts().is_empty());
    }
}
