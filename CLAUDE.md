# Session Deck (`sdeck`)

Rust workspace (edition 2021): a terminal UI that runs and monitors Claude Code and Copilot CLI sessions.
Windows is the primary target (no tmux/WSL); it also builds on Linux. The TUI is a port of a TypeScript
app (`packages/cli`), so doc comments say "Port of `foo.ts`" / `App.method`.

## Commands

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check            # rustfmt.toml: max_width = 110
cargo build --release -p sdeck -p sdeck-demo && target/release/sdeck-demo   # UI on fake data
```

On Linux, 7 `sdeck` tests fail because they assume Windows (`app::restore::*`, two
`live_session::*` launch-resolution tests, one `tree::*` test). Clippy and fmt are clean. Don't treat
those 7 as regressions.

## Crates

- `crates/sdeck-core`: library, **no terminal deps**. Discovery, status, persisted state, control API.
- `crates/sdeck`: the TUI (`lib.rs` holds everything; `main.rs` is a thin shell).
- `crates/fake-agent`: test-only stand-in for `claude` used by PTY tests.
- `crates/sdeck-demo`: builds a sandbox of fake repos/sessions under `target/demo`, then runs `sdeck` in it.

## Where things live

`sdeck-core/src/`
- `discovery/`: finds past sessions on disk (`claude_storage`, `copilot_storage`, `claude_trace`) and
  git info (`git_project` groups worktrees by git root, `git_status`).
- `status/`: live status. `session_status` (Running/Waiting/Done/Error), `claude_process_watcher`
  (pid files), `claude_transcript_tailer`, `copilot_status_watcher`, `statusline`/`session_usage`/
  `usage_display` (context, 5h/7d quota, spend), `waiting_notifier` + `hooks` (user `onRunning`/
  `onWaiting`/`onDone`/`onError` shell hooks, env `SDECK_*`), `alert_log`, `account` (multiple Claude
  accounts), `watch` (debounced file watcher).
- `store/`: persisted state. `deck_config` (`config.json`), `deck_store` (`state.json`), `tree_prefs`,
  `running_set` (`running.json`), `trash`, `atomic` (write-temp-then-rename with Windows retry).
- `ctl/`: read-only `sdeck ctl` API: loopback TCP, one JSON object per line, token auth,
  endpoint file `ctl.json`.
- `agent_catalog` (agent registry, `tools.<agent>.*`; Claude/Copilot have rich adapters, others are
  "basic"), `commands/session_id.rs` (session-id validation), `paths.rs`, `format`, `fuzzy_match`.

`sdeck/src/`
- `app.rs`: `App` state and the main loop. Feature handlers are in `app/*.rs`, one per feature (`attach`,
  `lifecycle`, `search`, `tags`, `archive`, `delete`, `bulk`, `new_session`, `overlays`, `ctl`, ...). Each
  file's `//!` header names the keys and the TS functions it ports.
- `event.rs` (`AppEvent`), `sessions.rs` (session model, discovery merge, status tracker), `tree.rs` (rows
  of the sessions panel), `live_session.rs` (agent in a background PTY mirrored into a `vt100` screen),
  `keys.rs`/`keybindings.rs` (input encoding and rebinding), `config_fields.rs` (config popup rows),
  `commands.rs` (`:` palette), `view/` (rendering), `term_widget.rs`, `theme.rs`, `layout.rs`.

Largest files, read selectively: `app.rs`, `tree.rs`, `view/overlays.rs`, `store/deck_store.rs`,
`view/list_panel.rs`.

## Architecture notes

- **One thread owns app state.** Input, probes, PTY readers, file watchers and the ctl server run on
  background threads and send `AppEvent`s over one channel; only the main loop mutates `App`.
  Never share `App` state across threads.
- **Status flow:** agent writes pid/status files (`~/.claude/session-deck-status`) and transcripts;
  watchers in `status/` feed the status tracker in `sdeck/src/sessions.rs`; a 1 s process poll
  (`app/lifecycle.rs`) adopts new session ids, sends queued prompts and fires toasts/hooks.
- **State dir:** `~/.session-deck/` (`config.json`, `state.json`, `running.json`, `ctl.json`, `trash/`).
  `running.json` is separate from `state.json` on purpose so writing it doesn't trigger `StoreChanged`.
- **Attach / interact:** attach hands the real terminal to the agent PTY until the detach key
  (`Ctrl+Q`, `ui.detachKey`); interact types into a session while the UI keeps rendering. Input bytes
  are re-encoded as VT sequences (`keys.rs`) so they forward to the PTY unchanged.
- `ctl` is read-only by design. Nothing in it may start, type into or stop a session.
- Session ids are interpolated into commands and paths: always validate with `commands/session_id.rs`.

## Conventions

- Keep `sdeck-core` free of terminal/TUI dependencies.
- Views are pure functions from app state to ratatui lines; each line is exactly the panel width.
- Write state files with `store::atomic::write_atomic`.
- Tests isolate the filesystem with env overrides: `SDECK_USER_HOME`, `SESSION_DECK_HOME`,
  `SESSION_DECK_STATUS_DIR` (see `paths.rs`). Tests that set them must hold the `test_env` lock.
- Shared TUI test helpers: `sdeck/src/test_support.rs`, `app/test_fixture.rs`.
- Releases are built by cargo-dist (`dist-workspace.toml`, `.github/workflows/release.yml`, generated:
  don't hand-edit).
