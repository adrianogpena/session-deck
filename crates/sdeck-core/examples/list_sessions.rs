//! Lists the Claude sessions found under every account's `projects/` dir, newest first, with each
//! session's account email, project root and git state. Honors `SDECK_USER_HOME`.

use sdeck_core::discovery::claude_storage::discover_claude_sessions;
use sdeck_core::discovery::git_project::resolve_project_root;
use sdeck_core::discovery::git_status::read_git_status;
use sdeck_core::format::{humanize_since, now_ms};
use sdeck_core::status::account::discover_accounts;
use sdeck_core::store::deck_config::{deck_config_path, read_deck_config};

fn main() {
    let accounts = discover_accounts();
    for a in &accounts {
        println!(
            "account: {} ({})",
            a.email.as_deref().unwrap_or("not logged in"),
            a.config_dir.display()
        );
    }
    let max = read_deck_config(&deck_config_path()).ui.max_sessions_listed;
    let now = now_ms();
    for s in discover_claude_sessions(&accounts, max as usize, |cwd| {
        Some(resolve_project_root(cwd).root)
    }) {
        let root = resolve_project_root(&s.cwd);
        let git = read_git_status(&s.cwd)
            .map(|g| {
                format!(
                    " [{} +{}/-{} ~{}]",
                    g.branch.as_deref().unwrap_or("detached"),
                    g.ahead,
                    g.behind,
                    g.dirty
                )
            })
            .unwrap_or_default();
        println!(
            "{}  {:<16} {:<28} {}  {}{}",
            s.id,
            humanize_since(s.mtime_ms, now),
            s.account.email.as_deref().unwrap_or("-"),
            root.root,
            s.title,
            git
        );
    }
}
