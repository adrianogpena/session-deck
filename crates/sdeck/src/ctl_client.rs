//! `sdeck ctl <list|status|screen|wait|ping>`: the read-only client of the running sdeck.
//! Exit codes: 0 ok · 1 error · 2 sdeck not running · 3 `wait` timed out.

use std::time::{Duration, Instant};

use sdeck_core::ctl::{CtlClient, Endpoint};
use sdeck_core::store::deck_config::{ctl_enabled, deck_config_path, read_deck_config};
use serde_json::{json, Value};

const POLL_INTERVAL: Duration = Duration::from_millis(500);
const STATUSES: [&str; 8] = [
    "running", "waiting", "done", "idle", "starting", "error", "exited", "stopped",
];

#[derive(Debug, PartialEq)]
pub struct Args {
    pub method: String,
    pub id: Option<String>,
    pub json: bool,
    pub rows: Option<u64>,
    pub statuses: Vec<String>,
    pub timeout: Option<Duration>,
}

pub fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut it = args.iter();
    let method = it
        .next()
        .ok_or("usage: sdeck ctl <list|status|screen|wait|ping> [id] [options]")?;
    if !["list", "status", "screen", "wait", "ping"].contains(&method.as_str()) {
        return Err(format!("unknown ctl command: {method}"));
    }
    let mut out = Args {
        method: method.clone(),
        id: None,
        json: false,
        rows: None,
        statuses: Vec::new(),
        timeout: None,
    };
    while let Some(arg) = it.next() {
        let mut value = |flag: &str| it.next().cloned().ok_or(format!("{flag} needs a value"));
        match arg.as_str() {
            "--json" => out.json = true,
            "--rows" => {
                out.rows = Some(value("--rows")?.parse().map_err(|_| "--rows needs a number")?);
            }
            "--timeout" => {
                let secs: f64 = value("--timeout")?
                    .parse()
                    .map_err(|_| "--timeout needs seconds")?;
                if !secs.is_finite() || secs < 0.0 {
                    return Err("--timeout needs seconds".into());
                }
                out.timeout = Some(Duration::from_secs_f64(secs));
            }
            "--status" => {
                for s in value("--status")?.split(',') {
                    let s = s.trim().to_lowercase();
                    if !STATUSES.contains(&s.as_str()) {
                        return Err(format!("unknown status {s:?}; one of {}", STATUSES.join(", ")));
                    }
                    out.statuses.push(s);
                }
            }
            flag if flag.starts_with("--") => return Err(format!("unknown option: {flag}")),
            id if out.id.is_none() => out.id = Some(id.to_string()),
            extra => return Err(format!("unexpected argument: {extra}")),
        }
    }
    let needs_id = matches!(out.method.as_str(), "status" | "screen" | "wait");
    if needs_id && out.id.is_none() {
        return Err(format!("sdeck ctl {} needs a session id", out.method));
    }
    if out.method == "wait" && out.statuses.is_empty() {
        return Err("sdeck ctl wait needs --status <status>[,<status>]".into());
    }
    Ok(out)
}

#[derive(Debug, PartialEq)]
pub enum WaitOutcome {
    Matched(String),
    TimedOut,
    Failed(String),
}

/// Polls `fetch` (the session's status) until it is one of `wanted`. Always fetches once, so a
/// zero timeout is a plain check.
pub fn wait_for(
    mut fetch: impl FnMut() -> Result<String, String>,
    wanted: &[String],
    timeout: Option<Duration>,
    interval: Duration,
) -> WaitOutcome {
    let deadline = timeout.map(|t| Instant::now() + t);
    loop {
        match fetch() {
            Ok(status) if wanted.contains(&status) => return WaitOutcome::Matched(status),
            Ok(_) => {}
            Err(e) => return WaitOutcome::Failed(e),
        }
        if deadline.is_some_and(|d| Instant::now() + interval > d) {
            return WaitOutcome::TimedOut;
        }
        std::thread::sleep(interval);
    }
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

pub fn format_list(sessions: &Value) -> String {
    let rows = sessions.as_array().map(Vec::as_slice).unwrap_or_default();
    rows.iter()
        .map(|s| {
            format!(
                "{}  {:<8}  {:<7}  {}  {}",
                text(s, "id"),
                text(s, "status"),
                text(s, "agent"),
                text(s, "project"),
                text(s, "title")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn format_status(s: &Value) -> String {
    let live = if s.get("live").and_then(Value::as_bool) == Some(true) {
        "yes"
    } else {
        "no"
    };
    [
        format!("id:       {}", text(s, "id")),
        format!("status:   {}", text(s, "status")),
        format!("agent:    {}", text(s, "agent")),
        format!("title:    {}", text(s, "title")),
        format!("project:  {}", text(s, "project")),
        format!("live:     {live}"),
        format!("cwd:      {}", text(s, "cwd")),
        format!("updated:  {}", text(s, "updatedAt")),
    ]
    .join("\n")
}

fn call(client: &mut CtlClient, method: &str, params: Value) -> Result<Value, String> {
    match client.call(method, params) {
        Ok(resp) => resp.outcome,
        Err(e) => Err(format!("lost contact with sdeck: {e}")),
    }
}

fn print_result(result: &Value, json: bool, human: impl Fn(&Value) -> String) {
    if json {
        println!("{result}");
    } else {
        let out = human(result);
        if !out.is_empty() {
            println!("{out}");
        }
    }
}

/// Runs the command and returns the process exit code.
pub fn run(args: &[String]) -> i32 {
    let args = match parse_args(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let Some(endpoint) = Endpoint::probe() else {
        if ctl_enabled(&read_deck_config(&deck_config_path())) {
            eprintln!("sdeck is not running");
        } else {
            eprintln!("sdeck ctl is turned off (ui.ctl is false in the config)");
        }
        return 2;
    };
    let mut client = match endpoint.connect() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("can't reach sdeck: {e}");
            return 2;
        }
    };
    let id_params = json!({"id": args.id});
    let result = match args.method.as_str() {
        "list" => call(&mut client, "list", json!({})).map(|r| print_result(&r, args.json, format_list)),
        "ping" => {
            call(&mut client, "ping", json!({})).map(|r| print_result(&r, args.json, |r| r.to_string()))
        }
        "status" => {
            call(&mut client, "status", id_params).map(|r| print_result(&r, args.json, format_status))
        }
        "screen" => {
            let params = json!({"id": args.id, "rows": args.rows});
            call(&mut client, "screen", params).map(|r| print_result(&r, args.json, |r| text(r, "text")))
        }
        _ => {
            let outcome = wait_for(
                || call(&mut client, "status", id_params.clone()).map(|r| text(&r, "status")),
                &args.statuses,
                args.timeout,
                POLL_INTERVAL,
            );
            match outcome {
                WaitOutcome::Matched(status) => {
                    println!("{status}");
                    return 0;
                }
                WaitOutcome::TimedOut => {
                    eprintln!("timed out waiting for {}", args.statuses.join(" or "));
                    return 3;
                }
                WaitOutcome::Failed(e) => Err(e),
            }
        }
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_each_command_and_its_options() {
        let list = parse_args(&args(&["list", "--json"])).unwrap();
        assert_eq!((list.method.as_str(), list.json, list.id), ("list", true, None));

        let screen = parse_args(&args(&["screen", "ab12", "--rows", "20"])).unwrap();
        assert_eq!((screen.id.as_deref(), screen.rows), (Some("ab12"), Some(20)));

        let wait = parse_args(&args(&[
            "wait",
            "ab12",
            "--status",
            "Waiting,done",
            "--timeout",
            "1.5",
        ]))
        .unwrap();
        assert_eq!(wait.statuses, ["waiting", "done"]);
        assert_eq!(wait.timeout, Some(Duration::from_millis(1500)));
    }

    #[test]
    fn rejects_bad_arguments() {
        for bad in [
            &[][..],
            &["start"],
            &["status"],
            &["wait", "x"],
            &["wait", "x", "--status", "sleeping"],
            &["screen", "x", "--rows", "many"],
            &["list", "--bogus"],
            &["list", "extra", "more"],
            &["wait", "x", "--status", "done", "--timeout", "-1"],
            &["screen", "x", "--rows"],
        ] {
            assert!(parse_args(&args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn wait_matches_times_out_and_fails() {
        let wanted = args(&["done"]);
        let mut seen = vec!["running", "running", "done"].into_iter();
        let matched = wait_for(
            || Ok(seen.next().unwrap().to_string()),
            &wanted,
            None,
            Duration::from_millis(1),
        );
        assert_eq!(matched, WaitOutcome::Matched("done".into()));

        let timed_out = wait_for(
            || Ok("running".into()),
            &wanted,
            Some(Duration::from_millis(20)),
            Duration::from_millis(5),
        );
        assert_eq!(timed_out, WaitOutcome::TimedOut);

        let failed = wait_for(|| Err("gone".into()), &wanted, None, Duration::from_millis(1));
        assert_eq!(failed, WaitOutcome::Failed("gone".into()));
    }

    #[test]
    fn formats_list_rows() {
        let sessions =
            json!([{"id": "a1", "status": "idle", "agent": "claude", "project": "p", "title": "t"}]);
        assert_eq!(format_list(&sessions), "a1  idle      claude   p  t");
        assert_eq!(format_list(&json!([])), "");
    }
}
