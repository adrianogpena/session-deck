<h1 align="center">sdeck</h1>

<p align="center"><strong>Session Deck's terminal UI — background Claude Code and GitHub Copilot CLI sessions, with a live preview and attach/detach.</strong></p>

<p align="center">
  <img alt="Node" src="https://img.shields.io/badge/Node-%3E%3D22.5.0-339933?logo=node.js&logoColor=white">
  <img alt="Platform" src="https://img.shields.io/badge/platform-Windows_%7C_macOS_%7C_Linux-lightgrey">
</p>

`sdeck` turns `~/.claude/projects` and the Copilot CLI's own session history into a full-screen
session manager: every session, grouped by project, with a live preview and one keystroke to attach.
Sessions keep running in the background while you switch between them. No tmux, no WSL — native on
Windows.

## Install

```bash
npm install -g sdeck
sdeck
```

Requires Node 22.5 or newer, and [Claude Code](https://code.claude.com) and/or
[GitHub Copilot CLI](https://github.com/github/copilot-cli) installed and on your `PATH` — `sdeck`
launches the real CLI, it doesn't reimplement either one.

## Features

- **Every session, one screen** — grouped automatically by git project, with manual folders on top.
- **Live preview** of the selected session's real screen, without attaching to it.
- **Attach / detach** (`Enter` / `Ctrl+Q`) — the session keeps running in the background while detached.
- **Full-text search** (`/`) across every session's prompts and replies, not just titles.
- **Status at a glance** — running, waiting for you, finished, idle, error or stopped, with desktop
  notifications the moment a session needs input.
- **Folders, pins, sort and filters**, plus an "active on top" view.
- **Rename, archive, delete with undo, fork** (Claude Code), and a one-line prompt sent without attaching.
- **Editable settings popup** (`C`) for `~/.session-deck/config.json` — no restart needed for most changes.
- **Tokyo Night theme**, dark / light / system.
- Shares state with the [Session Deck VS Code extension](https://marketplace.visualstudio.com/items?itemName=adrianogpena.session-deck)
  via `~/.session-deck/state.json` — rename or archive a session in one, see it in the other.

## Documentation

Every key, screen and file `sdeck` uses is in the
[user manual](https://github.com/adrianogpena/session-deck/blob/main/docs/user-manual.md).

## Development

This package lives in `packages/cli` of the [Session Deck monorepo](https://github.com/adrianogpena/session-deck).
From the repo root:

```bash
npm install
npm run build --workspace=@session-deck/core
npm run build --workspace=sdeck   # tsc, then esbuild bundles it into a single out/main.js
npm run sdeck                      # or: node packages/cli/out/main.js
```

`npm test` runs the unit test suite; `npm run lint` runs ESLint. See the root README for the repo layout.

## Credits

Built on ideas from:

- **[agent-deck](https://github.com/asheshgoplani/agent-deck)** — the "mission control" list-of-sessions UX.
- **[Agent Session Manager](https://github.com/izll/agent-session-manager)** — global history search across agents.
