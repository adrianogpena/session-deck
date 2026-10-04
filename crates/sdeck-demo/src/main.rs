//! Builds a sandbox with fake projects and sessions, then runs `sdeck` inside it, for README
//! screenshots and trying the UI without touching real data.
//!
//! `sdeck-demo [--no-launch] [DIR]` (default `target/demo`) wipes DIR, writes a fake home (git repos, Claude
//! transcripts, status files, state and config) and starts `sdeck` with `SDECK_USER_HOME` and
//! `SESSION_DECK_HOME` pointing at it. Sessions started from the UI run `demo-agent` instead of
//! `claude`.

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};

struct Project {
    name: &'static str,
    /// Local commits not on the remote, and files left uncommitted.
    ahead: u32,
    dirty: u32,
}

const PROJECTS: [Project; 6] = [
    Project {
        name: "atlas-api",
        ahead: 2,
        dirty: 3,
    },
    Project {
        name: "billing-service",
        ahead: 0,
        dirty: 0,
    },
    Project {
        name: "web-dashboard",
        ahead: 0,
        dirty: 1,
    },
    Project {
        name: "mobile-app",
        ahead: 1,
        dirty: 0,
    },
    Project {
        name: "infra-terraform",
        ahead: 0,
        dirty: 0,
    },
    Project {
        name: "docs-site",
        ahead: 0,
        dirty: 0,
    },
];

/// Folders as (name, member projects).
const FOLDERS: [(&str, &[&str]); 1] = [("Backend", &["atlas-api", "billing-service"])];

struct Session {
    id: &'static str,
    project: &'static str,
    /// `default` or `work` (a second Claude account).
    account: &'static str,
    title: &'static str,
    prompt: &'static str,
    reply: &'static str,
    age_minutes: u64,
    /// What the agent screen shows once the session is started from the UI: `idle`, `busy` or
    /// `waiting`.
    live: &'static str,
    /// Already running in "another terminal", so it shows as live right away.
    elsewhere: bool,
    /// `done` or `error`: finished since you last looked.
    unseen: Option<&'static str>,
    pin: Option<&'static str>,
    tags: &'static [&'static str],
    /// The question a `waiting` agent screen shows, as `Kind: detail`.
    ask: &'static str,
    /// The last tool call on the agent screen and its result line.
    tool: &'static str,
    result: &'static str,
}

const fn s(
    id: &'static str,
    project: &'static str,
    title: &'static str,
    prompt: &'static str,
    reply: &'static str,
    age_minutes: u64,
) -> Session {
    Session {
        id,
        project,
        account: "default",
        title,
        prompt,
        reply,
        age_minutes,
        live: "idle",
        elsewhere: false,
        unseen: None,
        pin: None,
        tags: &[],
        ask: "",
        tool: "Read(README.md)",
        result: "Read 42 lines",
    }
}

fn sessions() -> Vec<Session> {
    vec![
        Session {
            live: "busy",
            elsewhere: true,
            tags: &["perf"],
            tool: "Update(src/Atlas.Api/Orders/OrdersQuery.cs)",
            result: "Updated with 12 additions and 9 removals",
            ..s(
                "a1000000-0000-4000-8000-000000000001",
                "atlas-api",
                "Speed up the orders query",
                "The /orders endpoint takes 2s with 50k rows. Find out why and fix it.",
                "I found a missing index on orders.customer_id and an N+1 in the serializer. Adding the index and batching the lookups now.",
                1,
            )
        },
        Session {
            live: "waiting",
            elsewhere: true,
            ask: "Bash command: dotnet ef database update",
            tool: "Update(src/Atlas.Api/Orders/RefundController.cs)",
            result: "Updated with 38 additions and 2 removals",
            ..s(
                "a1000000-0000-4000-8000-000000000002",
                "atlas-api",
                "Add refund endpoint",
                "Add POST /orders/{id}/refund with partial refunds and an audit entry.",
                "The endpoint and the audit entry are written. I need to apply the migration before the integration tests can run.",
                4,
            )
        },
        Session {
            unseen: Some("done"),
            pin: Some("top"),
            tags: &["review"],
            ..s(
                "a1000000-0000-4000-8000-000000000003",
                "atlas-api",
                "Fix flaky OrderServiceTests",
                "OrderServiceTests.ConcurrentCheckout fails about one run in ten. Why?",
                "The test shared a static clock between cases. Each test now gets its own, and 200 consecutive runs pass.",
                22,
            )
        },
        s(
            "a1000000-0000-4000-8000-000000000004",
            "atlas-api",
            "Upgrade to .NET 9",
            "Upgrade the solution to .NET 9 and fix whatever breaks.",
            "Upgraded all 7 projects. Two obsolete APIs were replaced; the build and all 412 tests pass.",
            3 * 60 + 10,
        ),
        Session {
            live: "busy",
            tags: &["perf"],
            tool: "Bash(dotnet test --filter InvoiceRendering)",
            result: "Passed: 18, Failed: 1",
            ..s(
                "b2000000-0000-4000-8000-000000000001",
                "billing-service",
                "Invoice PDF rendering",
                "Invoices render blank when the customer name has an emoji. Fix it.",
                "The font fallback did not include emoji glyphs. I'm adding a fallback font and a regression test.",
                9,
            )
        },
        Session {
            unseen: Some("error"),
            ..s(
                "b2000000-0000-4000-8000-000000000002",
                "billing-service",
                "Reconcile Stripe payouts",
                "Write a job that reconciles Stripe payouts against our ledger every night.",
                "I started the job skeleton but the Stripe call returned an error I could not resolve.",
                35,
            )
        },
        s(
            "b2000000-0000-4000-8000-000000000003",
            "billing-service",
            "Tax rate lookup",
            "Where do we compute VAT for EU customers?",
            "VAT is computed in TaxCalculator.ForOrder; rates come from the tax_rates table, cached for an hour.",
            26 * 60,
        ),
        Session {
            live: "waiting",
            ask: "Edit file: src/components/PriceChart.tsx",
            tool: "Read(src/components/PriceChart.tsx)",
            result: "Read 214 lines",
            tags: &["ui"],
            ..s(
                "c3000000-0000-4000-8000-000000000001",
                "web-dashboard",
                "Dark mode for charts",
                "The charts ignore the dark theme. Make them follow it.",
                "PriceChart and VolumeChart use fixed colors. I can switch both to the theme tokens.",
                14,
            )
        },
        Session {
            unseen: Some("done"),
            tags: &["ui"],
            ..s(
                "c3000000-0000-4000-8000-000000000002",
                "web-dashboard",
                "Accessible date picker",
                "Make the date picker keyboard accessible and announce changes to screen readers.",
                "Arrow keys, Home/End and PageUp/PageDown now work, and the selected date is announced through an aria-live region.",
                48,
            )
        },
        s(
            "c3000000-0000-4000-8000-000000000003",
            "web-dashboard",
            "Bundle size audit",
            "Why did the main bundle grow by 300 kB this month?",
            "Most of it is a second copy of moment.js pulled in by two dependencies. Aliasing them to one copy saves 280 kB.",
            2 * 24 * 60,
        ),
        Session {
            account: "work",
            tags: &["release"],
            ..s(
                "d4000000-0000-4000-8000-000000000001",
                "mobile-app",
                "Crash on cold start (Android)",
                "Crash reports show a NullPointerException on cold start for Android 12. Investigate.",
                "The push token was read before the app context existed. Moving the read behind the lifecycle callback fixes it.",
                5 * 60,
            )
        },
        Session {
            account: "work",
            pin: Some("bottom"),
            ..s(
                "d4000000-0000-4000-8000-000000000002",
                "mobile-app",
                "Offline sync design",
                "Sketch how offline edits should sync and resolve conflicts.",
                "Proposal: a local operation log, replayed on reconnect, with last-writer-wins per field and a conflict list for the rest.",
                3 * 24 * 60,
            )
        },
        s(
            "e5000000-0000-4000-8000-000000000001",
            "infra-terraform",
            "Split the state file",
            "Plan how to split the monolithic Terraform state per environment.",
            "Three states (network, data, apps) with remote state data sources between them. The migration is 11 terraform state mv commands.",
            6 * 24 * 60,
        ),
        s(
            "f6000000-0000-4000-8000-000000000001",
            "docs-site",
            "Rewrite the quickstart",
            "Rewrite the quickstart so a new user reaches a working example in five minutes.",
            "The new quickstart has four steps and one runnable example. I removed the section on legacy configuration.",
            9 * 24 * 60,
        ),
    ]
}

struct Sandbox {
    root: PathBuf,
    home: PathBuf,
}

impl Sandbox {
    fn config_dir(&self, account: &str) -> PathBuf {
        match account {
            "work" => self.home.join(".claude-work"),
            _ => self.home.join(".claude"),
        }
    }

    fn project_dir(&self, name: &str) -> PathBuf {
        self.root.join("projects").join(name)
    }
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=Demo",
            "-c",
            "user.email=demo@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    assert!(ok, "git {args:?} failed in {}", dir.display());
}

fn commit_file(dir: &Path, name: &str, body: &str) {
    fs::write(dir.join(name), body).unwrap();
    git(dir, &["add", name]);
    git(dir, &["commit", "-m", &format!("Add {name}")]);
}

fn make_repo(sb: &Sandbox, p: &Project) {
    let dir = sb.project_dir(p.name);
    fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-b", "main"]);
    commit_file(&dir, "README.md", &format!("# {}\n", p.name));
    let remote = sb.root.join("remotes").join(format!("{}.git", p.name));
    fs::create_dir_all(&remote).unwrap();
    git(&remote, &["init", "--bare", "-b", "main"]);
    git(&dir, &["remote", "add", "origin", &remote.display().to_string()]);
    git(&dir, &["push", "-u", "origin", "main"]);
    for i in 0..p.ahead {
        commit_file(&dir, &format!("change-{i}.txt"), "x\n");
    }
    for i in 0..p.dirty {
        fs::write(dir.join(format!("wip-{i}.txt")), "wip\n").unwrap();
    }
}

fn set_age(file: &Path, minutes: u64) {
    let when = SystemTime::now() - Duration::from_secs(minutes * 60);
    let f = OpenOptions::new().write(true).open(file).unwrap();
    f.set_modified(when).unwrap();
}

fn encode_dir_name(path: &Path) -> String {
    path.display()
        .to_string()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn write_transcript(sb: &Sandbox, se: &Session) {
    let cwd = sb.project_dir(se.project);
    let dir = sb
        .config_dir(se.account)
        .join("projects")
        .join(encode_dir_name(&cwd));
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join(format!("{}.jsonl", se.id));
    let cwd = cwd.display().to_string();
    let records = [
        json!({"type": "user", "cwd": cwd, "sessionId": se.id, "message": {"role": "user", "content": se.prompt}}),
        json!({"type": "assistant", "cwd": cwd, "sessionId": se.id, "message": {"role": "assistant", "content": [{"type": "text", "text": se.reply}]}}),
        json!({"type": "ai-title", "aiTitle": se.title, "sessionId": se.id}),
    ];
    let body: String = records.iter().map(|r| format!("{r}\n")).collect();
    fs::write(&file, body).unwrap();
    set_age(&file, se.age_minutes);
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn project_key(sb: &Sandbox, name: &str) -> String {
    let key = sb.project_dir(name).display().to_string();
    if cfg!(windows) {
        key.to_lowercase()
    } else {
        key
    }
}

fn write_state(sb: &Sandbox, all: &[Session]) {
    let mut prefs = Map::new();
    for se in all {
        let mut o = Map::new();
        if let Some(pin) = se.pin {
            o.insert("pin".into(), json!(pin));
        }
        if !se.tags.is_empty() {
            o.insert("tags".into(), json!(se.tags));
        }
        if !o.is_empty() {
            prefs.insert(se.id.into(), Value::Object(o));
        }
    }
    let folders: Vec<Value> = FOLDERS
        .iter()
        .map(|(name, members)| {
            let keys: Vec<String> = members.iter().map(|m| project_key(sb, m)).collect();
            json!({"id": name.to_lowercase(), "name": name, "projects": keys})
        })
        .collect();
    let state =
        json!({"version": 1, "sessions": prefs, "ui": {"theme": "dark"}, "tree": {"folders": folders}});
    let dir = sb.home.join(".session-deck");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("state.json"),
        serde_json::to_string_pretty(&state).unwrap(),
    )
    .unwrap();
}

fn write_status_files(sb: &Sandbox, all: &[Session]) {
    let dir = sb.home.join(".claude").join("session-deck-status");
    fs::create_dir_all(&dir).unwrap();
    for se in all {
        if let Some(status) = se.unseen {
            let body = json!({"status": status, "updatedAt": now_ms()});
            fs::write(dir.join(format!("{}.json", se.id)), body.to_string()).unwrap();
        }
    }
}

fn write_usage(sb: &Sandbox, all: &[Session]) {
    let dir = sb.home.join(".claude").join("session-deck-status");
    let now = now_ms() as i64;
    let contexts = [34, 62, 18, 81, 45, 27, 9, 56, 22, 13, 67, 40, 8, 15];
    let accounts = [
        ("default", "alex@personal.example", 12.0, 46.0),
        ("work", "alex@work.example", 31.0, 22.0),
    ];
    for (i, se) in all.iter().enumerate() {
        let (_, email, five, seven) = accounts.iter().find(|a| a.0 == se.account).unwrap();
        let body = json!({
            "contextPercent": contexts[i % contexts.len()],
            "startupContextPercent": 7,
            "fiveHourPercent": five,
            "fiveHourResetsAt": now + 2 * 3600 * 1000 + 10 * 60 * 1000,
            "sevenDayPercent": seven,
            "sevenDayResetsAt": now + 3 * 24 * 3600 * 1000,
            "updatedAt": now,
            "accountEmail": email,
        });
        fs::write(dir.join(format!("{}.usage.json", se.id)), body.to_string()).unwrap();
    }
    // Cumulative 7d percent per day for this week; the last value matches today's reading.
    let today = chrono::Local::now().date_naive();
    let monday = today
        - chrono::Days::new(u64::from(
            chrono::Datelike::weekday(&today).num_days_from_monday(),
        ));
    let mut history = Map::new();
    for (_, email, _, seven) in accounts {
        let days = (today - monday).num_days();
        let mut per_day = Map::new();
        for d in 0..=days {
            let date = (monday + chrono::Days::new(d as u64))
                .format("%Y-%m-%d")
                .to_string();
            let share = if days == 0 {
                1.0
            } else {
                (d as f64 + 1.0) / (days as f64 + 1.0)
            };
            let uneven = share * share.sqrt();
            per_day.insert(date, json!((seven * uneven).round()));
        }
        history.insert(email.into(), Value::Object(per_day));
    }
    fs::write(
        dir.join("seven-day-history.json"),
        Value::Object(history).to_string(),
    )
    .unwrap();
}

/// Test file, command and result line for the follow-up request typed into a session.
fn project_test(project: &str) -> (&'static str, &'static str, &'static str) {
    match project {
        "atlas-api" | "billing-service" => (
            "tests/RegressionTests.cs",
            "dotnet test",
            "Passed! - Failed: 0, Passed: 19, Skipped: 0, Total: 19",
        ),
        "web-dashboard" => (
            "src/__tests__/regression.test.tsx",
            "npm test",
            "Tests: 24 passed, 24 total",
        ),
        "mobile-app" => (
            "app/src/test/RegressionTest.kt",
            "./gradlew test",
            "BUILD SUCCESSFUL in 41s",
        ),
        "infra-terraform" => (
            "tests/regression.tftest.hcl",
            "terraform test",
            "Success! 6 passed, 0 failed.",
        ),
        _ => (
            "tests/regression.test.js",
            "npm test",
            "Tests: 11 passed, 11 total",
        ),
    }
}

fn write_scenarios(sb: &Sandbox, all: &[Session]) {
    let mut m = Map::new();
    for se in all {
        let (test_file, test_cmd, test_result) = project_test(se.project);
        m.insert(
            se.id.into(),
            json!({"test_file": test_file, "test_cmd": test_cmd, "test_result": test_result, "status": se.live, "prompt": se.prompt, "reply": se.reply, "ask": se.ask, "activity": se.title, "tool": se.tool, "result": se.result}),
        );
    }
    fs::write(sb.home.join("demo-scenarios.json"), Value::Object(m).to_string()).unwrap();
}

fn write_config_and_accounts(sb: &Sandbox, agent: &Path) {
    let config = json!({
        "ui": {"notifications": false, "showUsage": true},
        "tools": {"claude": {"command": agent.display().to_string()}, "copilot": {"enabled": false}},
    });
    let dir = sb.home.join(".session-deck");
    fs::write(
        dir.join("config.json"),
        serde_json::to_string_pretty(&config).unwrap(),
    )
    .unwrap();
    let login = |email: &str| json!({"oauthAccount": {"emailAddress": email}}).to_string();
    fs::write(sb.home.join(".claude.json"), login("alex@personal.example")).unwrap();
    let work = sb.config_dir("work");
    fs::create_dir_all(&work).unwrap();
    fs::write(work.join(".claude.json"), login("alex@work.example")).unwrap();
}

fn sibling(name: &str) -> PathBuf {
    let exe = std::env::current_exe().expect("own path");
    exe.with_file_name(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

fn spawn_elsewhere(sb: &Sandbox, agent: &Path, se: &Session) -> Child {
    let mut cmd = Command::new(agent);
    cmd.args(["--resume", se.id])
        .current_dir(sb.project_dir(se.project))
        .env("SDECK_USER_HOME", &sb.home)
        .env("DEMO_HEADLESS", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if se.account == "work" {
        cmd.env("CLAUDE_CONFIG_DIR", sb.config_dir("work"));
    }
    cmd.spawn().expect("start demo-agent")
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let no_launch = args.iter().any(|a| a == "--no-launch");
    args.retain(|a| a != "--no-launch");
    let arg = args.first().cloned().unwrap_or_else(|| "target/demo".into());
    let root = std::path::absolute(&arg).expect("absolute path");
    let agent = sibling("demo-agent");
    let sdeck = sibling("sdeck");
    for exe in [&agent, &sdeck] {
        assert!(
            exe.exists(),
            "{} not found: run `cargo build -p sdeck -p sdeck-demo` first",
            exe.display()
        );
    }

    let _ = fs::remove_dir_all(&root);
    let sb = Sandbox {
        home: root.join("home"),
        root,
    };
    fs::create_dir_all(&sb.home).unwrap();
    let all = sessions();
    for p in &PROJECTS {
        make_repo(&sb, p);
    }
    for se in &all {
        write_transcript(&sb, se);
    }
    write_state(&sb, &all);
    write_status_files(&sb, &all);
    write_usage(&sb, &all);
    write_scenarios(&sb, &all);
    write_config_and_accounts(&sb, &agent);
    if no_launch {
        println!("demo home: {}", sb.home.display());
        return;
    }

    let mut children: Vec<Child> = all
        .iter()
        .filter(|se| se.elsewhere)
        .map(|se| spawn_elsewhere(&sb, &agent, se))
        .collect();
    std::thread::sleep(Duration::from_millis(500));

    let status = Command::new(&sdeck)
        .env("SDECK_USER_HOME", &sb.home)
        .env("SESSION_DECK_HOME", sb.home.join(".session-deck"))
        .env_remove("CLAUDE_CONFIG_DIR")
        .status();
    for c in &mut children {
        let _ = c.kill();
    }
    if let Err(e) = status {
        eprintln!("could not start sdeck: {e}");
        std::process::exit(1);
    }
}
