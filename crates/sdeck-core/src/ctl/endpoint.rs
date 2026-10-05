//! `~/.session-deck/ctl.json`: where the running sdeck listens (`{port, token, pid}`), and a client
//! that talks to it.

use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};

use super::protocol::{CtlRequest, CtlResponse};
use crate::paths::deck_home;
use crate::store::atomic::write_atomic;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
/// Longer than the server's own 2 s wait for the main loop, so its timeout answer still arrives.
const CALL_TIMEOUT: Duration = Duration::from_secs(5);

pub fn endpoint_path() -> PathBuf {
    deck_home().join("ctl.json")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub port: u16,
    pub token: String,
    pub pid: u32,
}

/// 64 hex chars from two v4 UUIDs.
fn new_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

impl Endpoint {
    /// This process listening on `port`, with a fresh token.
    pub fn new(port: u16) -> Self {
        Self {
            port,
            token: new_token(),
            pid: std::process::id(),
        }
    }

    pub fn read_from(path: &Path) -> Option<Self> {
        let raw = fs::read_to_string(path).ok()?;
        let v: Value = serde_json::from_str(&raw).ok()?;
        Some(Self {
            port: u16::try_from(v.get("port")?.as_u64()?).ok()?,
            token: v.get("token")?.as_str()?.to_string(),
            pid: u32::try_from(v.get("pid")?.as_u64()?).ok()?,
        })
    }

    pub fn write_to(&self, path: &Path) -> io::Result<()> {
        let body = json!({"port": self.port, "token": self.token, "pid": self.pid});
        write_atomic(path, &serde_json::to_string_pretty(&body).unwrap_or_default())
    }

    /// The running sdeck, if one answers a `ping`.
    pub fn probe() -> Option<Self> {
        Self::probe_at(&endpoint_path())
    }

    /// Reads `path` and pings it. A file nobody listens behind is stale (sdeck crashed): it is deleted,
    /// unless a new sdeck replaced it meanwhile.
    pub fn probe_at(path: &Path) -> Option<Self> {
        let endpoint = Self::read_from(path)?;
        let Ok(mut client) = endpoint.connect() else {
            if Self::read_from(path).as_ref() == Some(&endpoint) {
                let _ = fs::remove_file(path);
            }
            return None;
        };
        match client.call("ping", json!({})) {
            Ok(CtlResponse { outcome: Ok(_), .. }) => Some(endpoint),
            _ => None,
        }
    }

    pub fn connect(&self) -> io::Result<CtlClient> {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, self.port));
        let stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)?;
        stream.set_read_timeout(Some(CALL_TIMEOUT))?;
        stream.set_write_timeout(Some(CALL_TIMEOUT))?;
        Ok(CtlClient {
            token: self.token.clone(),
            reader: BufReader::new(stream.try_clone()?),
            writer: stream,
            next_id: 1,
        })
    }
}

/// One connection to the running sdeck; requests go one at a time.
pub struct CtlClient {
    token: String,
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    next_id: u64,
}

impl CtlClient {
    pub fn call(&mut self, method: &str, params: Value) -> io::Result<CtlResponse> {
        let id = self.next_id;
        self.next_id += 1;
        let line = CtlRequest::new(&self.token, id, method, params).to_line();
        self.writer.write_all(format!("{line}\n").as_bytes())?;
        self.writer.flush()?;
        let mut reply = String::new();
        if self.reader.read_line(&mut reply)? == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        CtlResponse::parse(reply.trim())
            .filter(|r| r.id == id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "unreadable reply from sdeck"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn round_trips_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ctl.json");
        let endpoint = Endpoint::new(4321);
        assert_eq!(endpoint.token.len(), 64);
        endpoint.write_to(&path).unwrap();
        assert_eq!(Endpoint::read_from(&path), Some(endpoint));
        fs::write(&path, "{\"port\":70000,\"token\":\"t\",\"pid\":1}").unwrap();
        assert_eq!(Endpoint::read_from(&path), None);
    }

    #[test]
    fn probe_removes_a_stale_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ctl.json");
        // A port that was just free: nothing listens on it any more.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        Endpoint::new(port).write_to(&path).unwrap();
        assert_eq!(Endpoint::probe_at(&path), None);
        assert!(!path.exists());
    }

    #[test]
    fn probe_with_no_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Endpoint::probe_at(&dir.path().join("ctl.json")), None);
    }
}
