//! Claude-like stand-in for the demo sandbox. Registers a live pid file like `claude` does, draws a
//! scripted chat screen for its session (from `demo-scenarios.json` in the demo home), redraws it
//! when the terminal is resized, and answers typed lines. `DEMO_HEADLESS=1` only registers and
//! idles, to pose as a session open in another terminal.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

const ORANGE: &str = "\x1b[38;5;209m";
const DIM: &str = "\x1b[2m";
const GREEN: &str = "\x1b[32m";
const CYAN: &str = "\x1b[36m";
const RESET: &str = "\x1b[0m";
const PROMPT_BG: &str = "\x1b[48;5;237m";

fn home() -> PathBuf {
    std::env::var_os("SDECK_USER_HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}

fn write_pid_file(path: &Path, id: &str, status: &str, cwd: &str) {
    let body = json!({
        "pid": std::process::id(),
        "sessionId": id,
        "status": status,
        "cwd": cwd,
        "startedAt": now_ms() as u64,
    });
    let _ = std::fs::write(path, body.to_string());
}

fn text<'a>(scenario: &'a Value, key: &str, default: &'a str) -> &'a str {
    scenario.get(key).and_then(Value::as_str).unwrap_or(default)
}

fn wrap(s: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in s.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// `│ content │` padded to `width` columns.
fn visible_width(s: &str) -> usize {
    let mut n = 0;
    let mut in_escape = false;
    for c in s.chars() {
        match (in_escape, c) {
            (true, 'm') => in_escape = false,
            (true, _) => {}
            (false, '') => in_escape = true,
            (false, _) => n += 1,
        }
    }
    n
}

fn boxed(content: &str, width: usize) -> String {
    let pad = width.saturating_sub(visible_width(content) + 4);
    format!("│ {content}{} │", " ".repeat(pad))
}

fn box_top(width: usize) -> String {
    format!("╭{}╮", "─".repeat(width.saturating_sub(2)))
}

fn box_bottom(width: usize) -> String {
    format!("╰{}╯", "─".repeat(width.saturating_sub(2)))
}

struct Screen {
    scenario: Value,
    cwd: String,
    status: String,
    /// Lines added by typing into the session, already rendered.
    log: Vec<String>,
    /// Replaces the scenario's spinner text while a typed request is being worked on.
    activity: String,
}

fn render(screen: &Screen, cols: usize, rows: usize) -> String {
    let width = cols.clamp(30, 100);
    let text_width = width.saturating_sub(2);
    let mut out: Vec<String> = Vec::new();

    let banner = width.min(60);
    out.push(box_top(banner));
    out.push(boxed(
        &format!("{ORANGE}✻{RESET} Welcome to Claude Code!"),
        banner,
    ));
    out.push(boxed("", banner));
    out.push(boxed(
        &format!("{DIM}/help for help, /status for your setup{RESET}"),
        banner,
    ));
    out.push(boxed("", banner));
    let cwd: String = screen
        .cwd
        .chars()
        .rev()
        .take(banner.saturating_sub(12))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    out.push(boxed(&format!("{DIM}cwd: {cwd}{RESET}"), banner));
    out.push(box_bottom(banner));
    out.push(String::new());

    let prompt = text(&screen.scenario, "prompt", "hello");
    for (i, l) in wrap(prompt, text_width.saturating_sub(2)).iter().enumerate() {
        let lead = if i == 0 { "> " } else { "  " };
        out.push(format!(
            "{PROMPT_BG}{lead}{l:<w$}{RESET}",
            w = width.saturating_sub(2)
        ));
    }
    out.push(String::new());

    let reply = text(&screen.scenario, "reply", "Ready when you are.");
    for (i, l) in wrap(reply, text_width.saturating_sub(2)).iter().enumerate() {
        out.push(if i == 0 {
            format!("{GREEN}●{RESET} {l}")
        } else {
            format!("  {l}")
        });
    }
    out.push(String::new());

    let tool = text(&screen.scenario, "tool", "Read(README.md)");
    let result = text(&screen.scenario, "result", "Read 42 lines");
    out.push(format!("{GREEN}●{RESET} {tool}"));
    out.push(format!("  {DIM}⎿  {result}{RESET}"));
    out.push(String::new());
    out.extend(screen.log.iter().cloned());

    match screen.status.as_str() {
        "busy" => {
            let activity = if screen.activity.is_empty() {
                text(&screen.scenario, "activity", "Working")
            } else {
                screen.activity.as_str()
            };
            out.push(format!(
                "{ORANGE}✢ {activity}…{RESET} {DIM}(42s · ↓ 3.1k tokens · esc to interrupt){RESET}"
            ));
            out.push(String::new());
        }
        "waiting" => {
            let ask = text(&screen.scenario, "ask", "Bash command: ls");
            let (kind, detail) = ask.split_once(": ").unwrap_or(("Tool", ask));
            out.push(box_top(width));
            out.push(boxed(kind, width));
            out.push(boxed(&format!("  {detail}"), width));
            out.push(boxed("", width));
            out.push(boxed("Do you want to proceed?", width));
            out.push(boxed(&format!("{CYAN}❯ 1. Yes{RESET}"), width));
            out.push(boxed("  2. Yes, and don't ask again for this command", width));
            out.push(boxed(
                "  3. No, and tell Claude what to do differently (esc)",
                width,
            ));
            out.push(box_bottom(width));
            out.push(String::new());
        }
        _ => {}
    }

    if screen.status != "waiting" {
        out.push(box_top(width));
        out.push(boxed(">", width));
        out.push(box_bottom(width));
        let hint = "? for shortcuts";
        let right = "Context left until auto-compact: 52%";
        let gap = width.saturating_sub(2 + hint.chars().count() + right.chars().count());
        out.push(format!("  {DIM}{hint}{}{right}{RESET}", " ".repeat(gap)));
    }
    let skip = out.len().saturating_sub(rows.saturating_sub(1));
    let mut frame = out[skip..].join("\r\n");
    if screen.status != "waiting" {
        // Park the cursor inside the input box, so typed characters appear there.
        frame.push_str("\x1b[2A\x1b[4G");
    }
    frame
}

fn draw(screen: &Mutex<Screen>) {
    let (cols, rows) = crossterm::terminal::size().map_or((80, 30), |(c, r)| (c as usize, r as usize));
    let frame = render(&screen.lock().unwrap(), cols, rows);
    let mut out = std::io::stdout();
    let _ = write!(out, "\x1b[2J\x1b[H{frame}");
    let _ = out.flush();
}

fn todo(first: char, second: char) -> Vec<String> {
    vec![
        format!("{GREEN}●{RESET} Update Todos"),
        format!("  {DIM}⎿  {first} Add a regression test for this{RESET}"),
        format!("  {DIM}   {second} Run the test suite{RESET}"),
        String::new(),
    ]
}

fn diff_line(number: u32, sign: char, code: &str, width: usize) -> String {
    let bg = if sign == '-' { "[48;5;52m" } else { "[48;5;22m" };
    let body = format!("{number:>5} {sign} {code}");
    format!("    {bg}{body:<w$}{RESET}", w = width.saturating_sub(6))
}

/// One scripted request/response: the typed line, a todo list, a file write with a diff, a test run
/// and a closing summary, each step appearing after a short pause.
fn run_turn(screen: &Mutex<Screen>, typed: &str) {
    let (test_file, test_cmd, test_result) = {
        let s = screen.lock().unwrap();
        let get = |k: &str, d: &str| text(&s.scenario, k, d).to_string();
        (
            get("test_file", "tests/regression.test"),
            get("test_cmd", "npm test"),
            get("test_result", "Tests: 19 passed, 19 total"),
        )
    };
    let width = crossterm::terminal::size().map_or(80, |(c, _)| (c as usize).clamp(30, 100));
    let step = |lines: Vec<String>, activity: &str, pause_ms: u64| {
        {
            let mut s = screen.lock().unwrap();
            s.log.extend(lines);
            s.activity = activity.to_string();
            s.status = "busy".into();
        }
        draw(screen);
        std::thread::sleep(Duration::from_millis(pause_ms));
    };
    let prompt_rows = wrap(typed, width.saturating_sub(4));
    let mut user: Vec<String> = prompt_rows
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let lead = if i == 0 { "> " } else { "  " };
            format!("{PROMPT_BG}{lead}{l:<w$}{RESET}", w = width.saturating_sub(2))
        })
        .collect();
    user.push(String::new());
    step(user, "Thinking", 1200);
    step(
        vec![
            format!("{GREEN}●{RESET} Good call. I'll cover that with a regression test and run the suite."),
            String::new(),
        ],
        "Planning",
        1000,
    );
    step(todo('☐', '☐'), "Writing the test", 1200);
    step(
        vec![
            format!("{GREEN}●{RESET} Write({test_file})"),
            format!("  {DIM}⎿  Wrote 34 lines to {test_file}{RESET}"),
            diff_line(
                1,
                '+',
                "// Regression: the case reported above must keep working",
                width,
            ),
            diff_line(
                2,
                '+',
                "// Arrange a record, run the operation twice, assert one result",
                width,
            ),
            diff_line(3, '+', "// Assert no duplicate side effects are produced", width),
            String::new(),
        ],
        "Running tests",
        1400,
    );
    step(
        vec![
            format!("{GREEN}●{RESET} Bash({test_cmd})"),
            format!("  {DIM}⎿  {test_result}{RESET}"),
            String::new(),
        ],
        "Finishing up",
        900,
    );
    {
        let mut s = screen.lock().unwrap();
        let at = s
            .log
            .iter()
            .rposition(|l| l.contains("Update Todos"))
            .unwrap_or(0);
        let done = todo('☒', '☒');
        s.log.splice(
            at..at + done.len().saturating_sub(1),
            done[..done.len() - 1].iter().cloned(),
        );
        s.log.push(format!(
            "{GREEN}●{RESET} Done. The regression test is in place and the suite passes:"
        ));
        s.log
            .push(format!("  {DIM}-{RESET} {test_file} covers the reported case"));
        s.log.push(format!("  {DIM}-{RESET} No other files were changed"));
        s.log.push(String::new());
        s.activity.clear();
        s.status = "idle".into();
    }
    draw(screen);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let id = args
        .windows(2)
        .find(|w| w[0] == "--resume")
        .map(|w| w[1].clone())
        .unwrap_or_else(|| format!("demo-{}", now_ms()));
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let config_dir = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".claude"));
    let sessions_dir = config_dir.join("sessions");
    let _ = std::fs::create_dir_all(&sessions_dir);
    let pid_file = sessions_dir.join(format!("{}.json", std::process::id()));

    let scenarios: Value = std::fs::read_to_string(home().join("demo-scenarios.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);
    let scenario = scenarios.get(&id).cloned().unwrap_or(Value::Null);
    let status = text(&scenario, "status", "idle").to_string();
    write_pid_file(&pid_file, &id, &status, &cwd);

    if std::env::var_os("DEMO_HEADLESS").is_some() {
        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    }

    let screen = Arc::new(Mutex::new(Screen {
        scenario,
        cwd: cwd.clone(),
        status,
        log: Vec::new(),
        activity: String::new(),
    }));
    draw(&screen);
    let watcher = Arc::clone(&screen);
    std::thread::spawn(move || {
        let mut last = crossterm::terminal::size().ok();
        loop {
            std::thread::sleep(Duration::from_millis(250));
            let now = crossterm::terminal::size().ok();
            if now != last {
                last = now;
                draw(&watcher);
            }
        }
    });

    for line in std::io::stdin().lock().lines() {
        let Ok(raw) = line else { break };
        let typed = raw.trim().to_string();
        if typed.starts_with("/exit") {
            break;
        }
        write_pid_file(&pid_file, &id, "busy", &cwd);
        run_turn(&screen, &typed);
        write_pid_file(&pid_file, &id, "idle", &cwd);
    }
    let _ = std::fs::remove_file(&pid_file);
}
