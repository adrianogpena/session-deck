//! Test-only stand-in for `claude` in PTY tests: reports its environment and arguments, then
//! answers typed lines.
//!
//! - `/exit [code]` exits (default code 0).
//! - `/sign-in-error` prints the API error a failed sign-in shows.
//! - Anything else is echoed back, followed by a `done` marker.

use std::io::{BufRead, Write};

fn main() {
    let mut out = std::io::stdout();
    let config_dir = std::env::var("CLAUDE_CONFIG_DIR").unwrap_or_else(|_| "unset".into());
    let args: Vec<String> = std::env::args().skip(1).collect();
    let _ = writeln!(out, "CLAUDE_CONFIG_DIR={config_dir}");
    let _ = writeln!(out, "ARGS={}", args.join(" "));
    let _ = write!(out, "> ");
    let _ = out.flush();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("/exit") {
            std::process::exit(rest.trim().parse().unwrap_or(0));
        }
        if line == "/sign-in-error" {
            let _ = writeln!(
                out,
                r#"API Error: 401 {{"type":"error","error":{{"type":"authentication_error"}}}}"#
            );
        } else {
            let _ = writeln!(out, "echo: {line}");
            let _ = writeln!(out, "done");
        }
        let _ = write!(out, "> ");
        let _ = out.flush();
    }
}
