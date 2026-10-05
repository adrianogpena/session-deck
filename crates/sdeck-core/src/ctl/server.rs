//! The control server: a loopback TCP listener on its own thread, one thread per connection. It
//! checks the token and hands each request to `forward` (the app sends it to the main loop), then
//! writes back whatever answer arrives on the reply channel.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use super::endpoint::{endpoint_path, Endpoint};
use super::protocol::{CtlRequest, CtlResponse};

/// An idle connection is closed after this.
const READ_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a connection waits for the main loop's answer, so a stuck loop can't hang a client.
const REPLY_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_LINE: u64 = 1 << 20;

/// Stops accepting and deletes the endpoint file (if it is still ours) on drop.
pub struct ServerHandle {
    endpoint: Endpoint,
    path: PathBuf,
    stop: Arc<AtomicBool>,
}

impl ServerHandle {
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wakes the accept loop so it sees `stop`.
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, self.endpoint.port));
        let _ = TcpStream::connect_timeout(&addr, Duration::from_millis(200));
        // A second sdeck may have taken the file over (last one wins): leave its file alone.
        if Endpoint::read_from(&self.path).as_ref() == Some(&self.endpoint) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Listens on `127.0.0.1` (any free port) and publishes it at `~/.session-deck/ctl.json`.
pub fn start<F>(forward: F) -> io::Result<ServerHandle>
where
    F: Fn(CtlRequest, Sender<CtlResponse>) + Send + Sync + 'static,
{
    start_at(endpoint_path(), forward)
}

pub fn start_at<F>(path: PathBuf, forward: F) -> io::Result<ServerHandle>
where
    F: Fn(CtlRequest, Sender<CtlResponse>) + Send + Sync + 'static,
{
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let endpoint = Endpoint::new(listener.local_addr()?.port());
    endpoint.write_to(&path)?;
    let stop = Arc::new(AtomicBool::new(false));
    let forward = Arc::new(forward);
    let token: Arc<str> = endpoint.token.as_str().into();
    let stopped = Arc::clone(&stop);
    thread::spawn(move || {
        for stream in listener.incoming() {
            if stopped.load(Ordering::SeqCst) {
                return;
            }
            let Ok(stream) = stream else { continue };
            let (token, forward) = (Arc::clone(&token), Arc::clone(&forward));
            thread::spawn(move || {
                let _ = serve(stream, &token, &*forward);
            });
        }
    });
    Ok(ServerHandle { endpoint, path, stop })
}

/// Compares in time independent of where the strings differ.
fn token_matches(given: &str, expected: &str) -> bool {
    given.len() == expected.len()
        && given
            .bytes()
            .zip(expected.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

fn serve<F>(stream: TcpStream, token: &str, forward: &F) -> io::Result<()>
where
    F: Fn(CtlRequest, Sender<CtlResponse>),
{
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    let mut line = String::new();
    loop {
        line.clear();
        let read = (&mut reader).take(MAX_LINE).read_line(&mut line)?;
        if read == 0 || !line.ends_with('\n') {
            return Ok(());
        }
        let (response, close) = match CtlRequest::parse(line.trim()) {
            Err(refusal) => (refusal, false),
            Ok(req) if !token_matches(&req.token, token) => (CtlResponse::err(req.id, "bad token"), true),
            Ok(req) => {
                let id = req.id;
                let (tx, rx) = mpsc::channel();
                forward(req, tx);
                let answer = rx
                    .recv_timeout(REPLY_TIMEOUT)
                    .unwrap_or_else(|_| CtlResponse::err(id, "sdeck did not answer in time"));
                (answer, false)
            }
        };
        writer.write_all(format!("{}\n", response.to_line()).as_bytes())?;
        writer.flush()?;
        if close {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn echo_server(path: PathBuf) -> ServerHandle {
        start_at(path, |req: CtlRequest, reply: Sender<CtlResponse>| {
            if req.method == "ping" {
                let _ = reply.send(CtlResponse::ok(req.id, json!("pong")));
            }
        })
        .unwrap()
    }

    #[test]
    fn ctl_server_answers_checks_the_token_and_cleans_up() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ctl.json");
        let server = echo_server(path.clone());
        assert_eq!(Endpoint::probe_at(&path).as_ref(), Some(server.endpoint()));

        let mut client = server.endpoint().connect().unwrap();
        let pong = client.call("ping", json!({})).unwrap();
        assert_eq!(pong.outcome, Ok(json!("pong")));
        let silent = client.call("other", json!({})).unwrap();
        assert_eq!(silent.outcome, Err("sdeck did not answer in time".into()));

        let mut wrong = server.endpoint().clone();
        wrong.token = "x".repeat(64);
        let refused = wrong.connect().unwrap().call("ping", json!({})).unwrap();
        assert_eq!(refused.outcome, Err("bad token".into()));

        drop(server);
        assert!(!path.exists());
    }

    #[test]
    fn ctl_server_leaves_a_file_another_instance_took_over() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ctl.json");
        let first = echo_server(path.clone());
        let second = echo_server(path.clone());
        drop(first);
        assert_eq!(Endpoint::read_from(&path).as_ref(), Some(second.endpoint()));
    }

    #[test]
    fn ctl_token_comparison() {
        assert!(token_matches("abc", "abc"));
        assert!(!token_matches("abd", "abc"));
        assert!(!token_matches("ab", "abc"));
    }
}
