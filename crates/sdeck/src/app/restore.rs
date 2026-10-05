//! Restoring running sessions: `running.json` follows the live set, and the next start offers to
//! resume those conversations. The agent processes themselves don't survive a quit.

use std::time::Instant;

use sdeck_core::status::process::is_process_alive;
use sdeck_core::store::deck_config::RestoreSessions;
use sdeck_core::store::running_set::{read_running_set, write_running_set, RunningEntry, RunningSet};

use super::App;

/// "Restore N sessions?" with Yes (default) and No.
pub(super) struct RestoreConfirm {
    pub uids: Vec<u64>,
    pub yes: bool,
}

impl App {
    /// Live sessions with an id, plus the ones still queued to restore.
    fn running_entries(&self) -> Vec<RunningEntry> {
        self.sessions
            .iter()
            .filter(|s| s.is_live() || self.restore_queue.contains(&s.uid))
            .filter_map(|s| {
                Some(RunningEntry {
                    id: s.id.clone()?,
                    agent: s.agent.clone(),
                })
            })
            .collect()
    }

    /// Writes the live set to `running.json` when it changed. Never while quitting (stopping the
    /// PTYs on the way out must not empty it) and never before the restore decision is made.
    pub(super) fn sync_running_set(&mut self) {
        if self.quit || !self.restore_decided {
            return;
        }
        let entries = self.running_entries();
        if self.running_written.as_ref() == Some(&entries) {
            return;
        }
        let set = RunningSet {
            pid: std::process::id(),
            sessions: entries.clone(),
        };
        if write_running_set(&set).is_ok() {
            self.running_written = Some(entries);
        }
    }

    /// After a discovery: offers (or, with `always`, starts) the sessions of the previous run, once.
    /// Skipped while another sdeck is alive (a second window must not take over its sessions) or
    /// when something is already live.
    pub(super) fn check_restore(&mut self) {
        if self.restore_decided || self.restore_confirm.is_some() {
            return;
        }
        let set = read_running_set();
        let other_sdeck = set.pid != std::process::id() && is_process_alive(set.pid);
        if self.config.ui.restore_sessions == RestoreSessions::Never
            || set.sessions.is_empty()
            || other_sdeck
            || self.sessions.iter().any(|s| s.is_live())
        {
            self.restore_decided = true;
            return;
        }
        let uids: Vec<u64> = set
            .sessions
            .iter()
            .filter_map(|e| {
                self.sessions
                    .iter()
                    .find(|s| s.id.as_deref() == Some(e.id.as_str()) && s.agent == e.agent)
                    .map(|s| s.uid)
            })
            .collect();
        if uids.is_empty() {
            self.restore_decided = true;
            self.sync_running_set();
        } else if self.config.ui.restore_sessions == RestoreSessions::Always {
            self.begin_restore(uids);
        } else {
            self.restore_confirm = Some(RestoreConfirm { uids, yes: true });
            self.dirty = true;
        }
    }

    fn begin_restore(&mut self, uids: Vec<u64>) {
        self.restore_queue = uids;
        self.restore_decided = true;
        self.sync_running_set();
    }

    pub(super) fn on_restore_confirm_key(&mut self, key: &str) {
        let Some(confirm) = self.restore_confirm.as_mut() else {
            return;
        };
        let answer = match key {
            "\x1b[C" | "\x1b[D" | "\t" => {
                confirm.yes = !confirm.yes;
                None
            }
            "\r" => Some(confirm.yes),
            "y" | "Y" => Some(true),
            "n" | "N" | "\x1b" | "q" | "\x03" => Some(false),
            _ => None,
        };
        if let Some(yes) = answer {
            let uids = self.restore_confirm.take().map(|c| c.uids).unwrap_or_default();
            if yes {
                self.begin_restore(uids);
            } else {
                self.restore_decided = true;
                self.sync_running_set();
            }
        }
        self.dirty = true;
    }

    /// Starts one queued session per tick, so a restore isn't a burst of agent spawns in one frame.
    pub(super) fn tick_restore(&mut self, now: Instant) {
        let Some(&uid) = self.restore_queue.first() else {
            return;
        };
        self.ensure_live(uid, now);
        self.restore_queue.retain(|&u| u != uid);
        self.sync_running_set();
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use sdeck_core::store::deck_config::RestoreSessions;
    use sdeck_core::store::running_set::{read_running_set, write_running_set, RunningEntry, RunningSet};

    use crate::app::test_fixture::{fixture, Fixture};
    use crate::event::AppEvent;

    fn entry(id: &str) -> RunningEntry {
        RunningEntry {
            id: id.into(),
            agent: "claude".into(),
        }
    }

    fn previous_run(f: &mut Fixture, pid: u32, ids: &[&str]) -> Vec<u64> {
        let uids = ids.iter().map(|id| f.add(Some(id), true)).collect();
        write_running_set(&RunningSet {
            pid,
            sessions: ids.iter().map(|id| entry(id)).collect(),
        })
        .unwrap();
        f.app.restore_decided = false;
        uids
    }

    fn live_count(f: &Fixture) -> usize {
        f.app.sessions.iter().filter(|s| s.is_live()).count()
    }

    fn tick(f: &mut Fixture) {
        f.app.handle(AppEvent::Tick, Instant::now());
    }

    fn saved_ids() -> Vec<String> {
        read_running_set().sessions.into_iter().map(|e| e.id).collect()
    }

    fn sleeper() -> std::process::Child {
        let mut cmd = if cfg!(windows) {
            let mut c = std::process::Command::new("ping");
            c.args(["-n", "30", "127.0.0.1"]);
            c
        } else {
            let mut c = std::process::Command::new("sleep");
            c.arg("30");
            c
        };
        cmd.stdout(std::process::Stdio::null()).spawn().unwrap()
    }

    #[test]
    fn the_file_follows_live_sessions_and_quitting_keeps_it() {
        let mut f = fixture();
        let a = f.add(Some("a"), true);
        let b = f.add(Some("b"), true);
        assert!(f.app.start(a, Instant::now()));
        assert!(f.app.start(b, Instant::now()));
        assert_eq!(saved_ids(), ["a", "b"]);
        assert_eq!(read_running_set().pid, std::process::id());

        f.app.kill(a);
        assert_eq!(saved_ids(), ["b"]);

        f.key("q");
        f.key("y");
        assert!(f.app.should_quit());
        let Fixture { app, .. } = f;
        drop(app);
        assert_eq!(saved_ids(), ["b"]);
    }

    #[test]
    fn ask_prompts_and_yes_starts_every_session_one_per_tick() {
        let mut f = fixture();
        let uids = previous_run(&mut f, 0, &["a", "b"]);
        f.app.check_restore();
        assert!(f.app.restore_confirm.is_some());
        assert_eq!(live_count(&f), 0);

        f.key("y");
        assert!(f.app.restore_confirm.is_none());
        assert_eq!(saved_ids().len(), 2);
        tick(&mut f);
        assert_eq!(live_count(&f), 1);
        tick(&mut f);
        assert_eq!(live_count(&f), 2);
        assert!(uids.iter().all(|&u| f.session(u).unwrap().is_live()));
        assert_eq!(saved_ids().len(), 2);
    }

    #[test]
    fn no_discards_the_set() {
        let mut f = fixture();
        previous_run(&mut f, 0, &["a"]);
        f.app.check_restore();
        f.key("n");
        tick(&mut f);
        assert_eq!(live_count(&f), 0);
        assert!(saved_ids().is_empty());
    }

    #[test]
    fn always_restores_without_asking() {
        let mut f = fixture();
        f.app.config.ui.restore_sessions = RestoreSessions::Always;
        previous_run(&mut f, 0, &["a"]);
        f.app.check_restore();
        assert!(f.app.restore_confirm.is_none());
        tick(&mut f);
        assert_eq!(live_count(&f), 1);
    }

    #[test]
    fn never_does_nothing() {
        let mut f = fixture();
        f.app.config.ui.restore_sessions = RestoreSessions::Never;
        previous_run(&mut f, 0, &["a"]);
        f.app.check_restore();
        tick(&mut f);
        assert!(f.app.restore_confirm.is_none());
        assert_eq!(live_count(&f), 0);
    }

    #[test]
    fn another_live_sdeck_keeps_its_sessions_and_a_dead_one_does_not() {
        let mut other = sleeper();
        let mut f = fixture();
        previous_run(&mut f, other.id(), &["a"]);
        f.app.check_restore();
        assert!(f.app.restore_confirm.is_none());
        assert!(f.app.restore_decided);
        let pid = other.id();
        let _ = other.kill();
        let _ = other.wait();
        previous_run(&mut f, pid, &["a"]);
        f.app.restore_decided = false;
        f.app.check_restore();
        assert!(f.app.restore_confirm.is_some());
    }

    #[test]
    fn ids_no_longer_listed_are_dropped() {
        let mut f = fixture();
        previous_run(&mut f, 0, &["a"]);
        write_running_set(&RunningSet {
            pid: 0,
            sessions: vec![entry("gone")],
        })
        .unwrap();
        f.app.check_restore();
        assert!(f.app.restore_confirm.is_none());
        assert!(saved_ids().is_empty());
    }
}
