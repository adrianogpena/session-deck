# Session Deck

`sdeck`: a terminal UI that manages Claude Code and GitHub Copilot CLI sessions. Sessions run in
background PTYs with a live preview and attach/detach; native on Windows (no tmux, no WSL). Sessions
from several logged-in Claude accounts are listed together.

Every command and key is explained in the [user manual](docs/user-manual.md).

## Build and run

Requires the stable Rust toolchain (`x86_64-pc-windows-gnu` on Windows, set locally with
`rustup override set stable-x86_64-pc-windows-gnu`; CI releases build with MSVC).

```bash
cargo build --release        # target/release/sdeck.exe
target/release/sdeck.exe --version
```

Run `sdeck` from a real terminal (Windows Terminal, PowerShell). Ctrl+Q detaches from an attached
session; Ctrl+K q detaches and stops it.

## Development

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

- `crates/sdeck-core`: discovery, status, the shared state store. No terminal dependencies.
- `crates/sdeck`: the TUI binary.
- `crates/fake-agent`: test-only stand-in for `claude`.
