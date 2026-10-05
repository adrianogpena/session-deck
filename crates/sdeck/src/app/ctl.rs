//! The app's side of `sdeck ctl`: starting the control server and answering its requests on the
//! main loop. Read-only by design: nothing here starts, types into or stops a session.

use std::time::Instant;

use chrono::{DateTime, SecondsFormat, Utc};
use sdeck_core::ctl::server::{self, ServerHandle};
use sdeck_core::ctl::{CtlRequest, CtlResponse};
use serde_json::{json, Value};

use super::{App, VERSION};
use crate::event::{AppEvent, CtlReply};
use crate::sessions::{display_title, DeckSession, SessionStatus};
use crate::tree::project_labels;

pub fn status_name(status: SessionStatus) -> &'static str {
    match status {
        SessionStatus::Running => "running",
        SessionStatus::Waiting => "waiting",
        SessionStatus::Done => "done",
        SessionStatus::Idle => "idle",
        SessionStatus::Starting => "starting",
        SessionStatus::Error => "error",
        SessionStatus::Exited => "exited",
        SessionStatus::Stopped => "stopped",
    }
}

impl App {
    /// Starts the control server; each request reaches the main loop as an [`AppEvent::Ctl`]. If it
    /// can't start, ctl stays off for this run and a notice says why.
    pub fn start_ctl_server(&mut self) -> Option<ServerHandle> {
        let tx = self.tx.clone();
        match server::start(move |req, reply| {
            let _ = tx.send(AppEvent::Ctl(req, CtlReply(reply)));
        }) {
            Ok(handle) => Some(handle),
            Err(err) => {
                self.flash(format!("sdeck ctl is off: {err}"), Instant::now());
                None
            }
        }
    }

    pub(super) fn handle_ctl(&self, req: &CtlRequest) -> CtlResponse {
        let outcome = match req.method.as_str() {
            "ping" => Ok(json!({"version": VERSION, "pid": std::process::id()})),
            "list" => Ok(self.ctl_list()),
            "status" => self
                .ctl_find(&req.params)
                .map(|s| self.ctl_entry(s, &self.ctl_labels())),
            "screen" => self.ctl_screen(&req.params),
            other => Err(format!("unknown method: {other}")),
        };
        CtlResponse { id: req.id, outcome }
    }

    fn ctl_labels(&self) -> std::collections::HashMap<String, String> {
        let mut roots: Vec<String> = self.sessions.iter().map(|s| s.project_root.clone()).collect();
        roots.sort();
        roots.dedup();
        project_labels(&roots)
    }

    fn ctl_entry(&self, s: &DeckSession, labels: &std::collections::HashMap<String, String>) -> Value {
        let updated = DateTime::<Utc>::from_timestamp_millis(s.mtime_ms)
            .map(|d| d.to_rfc3339_opts(SecondsFormat::Secs, true));
        json!({
            "id": s.id,
            "agent": s.agent,
            "title": display_title(s, &self.store),
            "project": labels.get(&s.project_root).cloned().unwrap_or_else(|| s.project_root.clone()),
            "status": status_name(self.procs.status_of(s)),
            "live": s.is_live(),
            "cwd": s.cwd,
            "updatedAt": updated,
        })
    }

    /// Every session that has an agent session id yet.
    fn ctl_list(&self) -> Value {
        let labels = self.ctl_labels();
        Value::Array(
            self.sessions
                .iter()
                .filter(|s| s.id.is_some())
                .map(|s| self.ctl_entry(s, &labels))
                .collect(),
        )
    }

    /// The session `params.id` names: an exact id, else a unique prefix of one.
    fn ctl_find(&self, params: &Value) -> Result<&DeckSession, String> {
        let wanted = params
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or("missing session id")?;
        let with_id = || self.sessions.iter().filter(|s| s.id.is_some());
        if let Some(exact) = with_id().find(|s| s.id.as_deref() == Some(wanted)) {
            return Ok(exact);
        }
        let mut matches = with_id().filter(|s| s.id.as_deref().is_some_and(|id| id.starts_with(wanted)));
        match (matches.next(), matches.next()) {
            (Some(only), None) => Ok(only),
            (Some(_), Some(_)) => Err(format!("ambiguous session id: {wanted}")),
            _ => Err(format!("no session with id {wanted}")),
        }
    }

    /// The live session's visible screen as text, trailing blank rows dropped; `params.rows` keeps
    /// only the last N.
    fn ctl_screen(&self, params: &Value) -> Result<Value, String> {
        let s = self.ctl_find(params)?;
        let live = s
            .live
            .as_ref()
            .filter(|_| s.is_live())
            .ok_or("session is not running here")?;
        let screen = live.screen();
        let (_, cols) = screen.size();
        let mut rows: Vec<String> = screen.rows(0, cols).map(|r| r.trim_end().to_string()).collect();
        while rows.last().is_some_and(String::is_empty) {
            rows.pop();
        }
        if let Some(n) = params.get("rows").and_then(Value::as_u64) {
            let keep = usize::try_from(n).unwrap_or(usize::MAX);
            rows.drain(..rows.len().saturating_sub(keep));
        }
        Ok(json!({"text": rows.join("\n")}))
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpStream;
    use std::time::{Duration, Instant};

    use sdeck_core::ctl::{endpoint_path, CtlRequest, CtlResponse, Endpoint};
    use serde_json::json;

    use crate::app::test_fixture::fixture;
    use crate::app::VERSION;
    use crate::event::AppEvent;

    fn ask(stream: &mut TcpStream, token: &str, id: u64, method: &str) -> CtlResponse {
        let line = CtlRequest::new(token, id, method, json!({})).to_line();
        writeln!(stream, "{line}").unwrap();
        let mut reply = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut reply)
            .unwrap();
        CtlResponse::parse(reply.trim()).unwrap()
    }

    #[test]
    fn ctl_ping_is_answered_by_the_app_and_a_wrong_token_is_refused() {
        let mut f = fixture();
        let server = f.app.start_ctl_server().unwrap();
        let endpoint = Endpoint::read_from(&endpoint_path()).unwrap();
        assert_eq!(&endpoint, server.endpoint());

        let client = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(("127.0.0.1", endpoint.port)).unwrap();
            let pong = ask(&mut stream, &endpoint.token, 1, "ping");
            let unknown = ask(&mut stream, &endpoint.token, 2, "stop");
            let mut intruder = TcpStream::connect(("127.0.0.1", endpoint.port)).unwrap();
            let refused = ask(&mut intruder, "wrong", 3, "ping");
            (pong, unknown, refused)
        });
        for _ in 0..2 {
            let ev = f.rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(matches!(ev, AppEvent::Ctl(..)));
            f.app.handle(ev, Instant::now());
        }
        let (pong, unknown, refused) = client.join().unwrap();
        assert_eq!(
            pong.outcome,
            Ok(json!({"version": VERSION, "pid": std::process::id()}))
        );
        assert_eq!(unknown.outcome, Err("unknown method: stop".into()));
        assert_eq!(refused.outcome, Err("bad token".into()));

        drop(server);
        assert!(!endpoint_path().exists());
    }

    fn call(f: &crate::app::test_fixture::Fixture, method: &str, params: serde_json::Value) -> CtlResponse {
        f.app.handle_ctl(&CtlRequest::new("t", 1, method, params))
    }

    #[test]
    fn ctl_list_and_status_report_sessions_and_resolve_prefixes() {
        let mut f = fixture();
        f.add(Some("aaaa-1"), true);
        f.add(Some("aaaa-2"), true);
        f.add(Some("bbbb-3"), true);
        f.add(None, false);

        let list = call(&f, "list", json!({})).outcome.unwrap();
        let rows = list.as_array().unwrap();
        assert_eq!(rows.len(), 3);
        assert!(rows
            .iter()
            .all(|r| r["status"] == "stopped" && r["live"] == false));
        assert_eq!(rows[0]["agent"], "claude");
        assert_eq!(rows[0]["updatedAt"], "1970-01-01T00:00:00Z");

        let one = call(&f, "status", json!({"id": "bbbb"})).outcome.unwrap();
        assert_eq!(one["id"], "bbbb-3");
        assert_eq!(
            call(&f, "status", json!({"id": "aaaa"})).outcome,
            Err("ambiguous session id: aaaa".into())
        );
        assert_eq!(
            call(&f, "status", json!({"id": "aaaa-1"})).outcome.unwrap()["id"],
            "aaaa-1"
        );
        assert_eq!(
            call(&f, "status", json!({"id": "zzz"})).outcome,
            Err("no session with id zzz".into())
        );
        assert_eq!(
            call(&f, "status", json!({})).outcome,
            Err("missing session id".into())
        );
    }

    #[test]
    fn ctl_screen_needs_a_live_session() {
        let mut f = fixture();
        f.add(Some("aaaa-1"), true);
        assert_eq!(
            call(&f, "screen", json!({"id": "aaaa-1"})).outcome,
            Err("session is not running here".into())
        );
    }

    #[test]
    fn ctl_screen_returns_the_live_agent_text_and_honours_rows() {
        let mut f = fixture();
        let uid = f.add(Some("abc"), true);
        let transcript = f.home.path().join("abc.jsonl");
        std::fs::write(&transcript, "").unwrap();
        f.app.session_mut(uid).unwrap().file = Some(transcript);
        f.key("s");
        f.wait_for_screen(uid, "ARGS=--resume abc");

        let full = call(&f, "screen", json!({"id": "abc"})).outcome.unwrap();
        assert!(full["text"].as_str().unwrap().contains("ARGS=--resume abc"));
        let last = call(&f, "screen", json!({"id": "abc", "rows": 1}))
            .outcome
            .unwrap();
        assert!(!last["text"].as_str().unwrap().contains('\n'));
        assert_eq!(call(&f, "list", json!({})).outcome.unwrap()[0]["live"], true);
    }
}
