//! Lists the `claude` processes running right now (every account) with their live status.
//! Honors `SDECK_USER_HOME`.

use sdeck_core::status::account::discover_accounts;
use sdeck_core::status::claude_process_watcher::{
    classify_claude_process_status, list_live_claude_processes,
};

fn main() {
    let accounts = discover_accounts();
    let live = list_live_claude_processes(&accounts);
    if live.is_empty() {
        println!("no live claude processes");
    }
    for p in live {
        let mapped = classify_claude_process_status(&p.status).map_or("-", |s| s.as_str());
        println!(
            "{:>7}  {}  {:<8} -> {:<8} {:<28} {}",
            p.pid,
            p.session_id,
            p.status,
            mapped,
            p.account.email.as_deref().unwrap_or("-"),
            p.cwd.as_deref().unwrap_or("")
        );
    }
}
