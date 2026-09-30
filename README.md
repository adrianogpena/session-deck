# Session Deck

A session manager for AI coding agents (Claude Code, GitHub Copilot CLI), with two front ends that
share the same core and state:

| Package | What it is |
|---|---|
| [`packages/vscode`](packages/vscode) | The VS Code extension ([README](packages/vscode/README.md), published to the Marketplace as `adrianogpena.session-deck`) |
| [`packages/cli`](packages/cli) | `sdeck`, the terminal UI: background sessions with a live preview and attach/detach, native on Windows (no tmux, no WSL). Published to npm as [`sdeck`](https://www.npmjs.com/package/sdeck). Early stage. |
| [`packages/core`](packages/core) | Everything vscode-free both use: session discovery, status, CLI commands, and the shared state store |

Names and archive flags set in either front end are stored in `~/.session-deck/state.json`, so both
see the same thing.

## Install

- **Extension:** [Session Deck on the VS Code Marketplace](https://marketplace.visualstudio.com/items?itemName=adrianogpena.session-deck).
- **Terminal UI:** `npm install -g sdeck`, then run `sdeck` from any terminal. Needs Node 22.5+.

## Development

Requires Node 22.5+ (`node:sqlite`).

```bash
npm install
npm run build          # all packages: core, then the extension, then the CLI
npm test               # core unit tests
npm run lint
```

- **Extension:** press `F5` to launch an Extension Development Host. `npm run package:vscode` builds a `.vsix`.
- **Terminal UI:** `npm run sdeck` from a real terminal (Windows Terminal, PowerShell). Ctrl+Q detaches from
  an attached session; Ctrl+K q detaches and stops it.

Every command and key is explained in the [user manual](docs/user-manual.md). Plans and design notes are in [`docs/`](docs).
