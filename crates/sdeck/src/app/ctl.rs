//! The app's side of `sdeck ctl`: starting the control server and answering its requests on the
//! main loop. Read-only by design: nothing here starts, types into or stops a session.

use std::time::Instant;

use sdeck_core::ctl::server::{self, ServerHandle};
use sdeck_core::ctl::{CtlRequest, CtlResponse};
use serde_json::json;

use super::{App, VERSION};
use crate::event::{AppEvent, CtlReply};

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
        match req.method.as_str() {
            "ping" => CtlResponse::ok(req.id, json!({"version": VERSION, "pid": std::process::id()})),
            other => CtlResponse::err(req.id, format!("unknown method: {other}")),
        }
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
}
