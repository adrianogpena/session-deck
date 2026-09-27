# Changelog

All notable changes to Session Deck are documented in this file.

## [Unreleased]

### Added

- Folders for projects, manual order for folders and projects (Move Up / Move Down), pinned sessions, and a sort order for sessions (most recent, or needs attention first). All of it is shared with the Session Deck terminal UI.
- "Mark as Unread" on sessions: a finished session shows its status again until you look at it.
- Session names, archive flags and collapsed folders/projects are stored in `~/.session-deck/state.json`, shared with the terminal UI. Existing names and archive flags are copied over on first start.

### Changed

- Which finished sessions you have already looked at is remembered across restarts, and shared with the terminal UI (it used to reset when VS Code restarted).
- With the terminal UI running too, each desktop notification is shown once, not by both.
- Projects start expanded unless collapsed (the collapsed state is now remembered, in both front ends).
- Claude sessions are titled with Claude Code's own title (`/rename` or its AI-generated title) instead of the first prompt.

## [0.1.0] - 2026-09-23

Initial release.

### Added

- One sidebar for Claude Code and GitHub Copilot CLI sessions, curated per-workspace via `.vscode/session-deck.json`.
- Resume any session in a real terminal running the actual `claude`/`copilot` CLI — one terminal per session, reused on click.
- Live status (running / waiting / done / error) for both agents, with no setup or hooks required.
- Desktop notifications when a session needs input.
- Fuzzy search across every session's full content, with a status-prefix filter (`!running`, `@waiting`, `#done`, `~error`).
- Fork a session (Claude Code) to branch a new session off an existing one's full history.
- Start brand-new sessions for any project from the tree.
- Sessions auto-archive once a project's active list fills up — nothing is lost.
- Rename, hide, or remove projects and sessions; per-project overrides for emoji, color, session cap, and permission prompts.
- View a transcript, or copy a session's last response or full details to the clipboard.
- Multi-root workspace support.
- "Open Sessions" panel in the built-in Explorer sidebar — a flat, cross-project list of currently open sessions.
- Works natively on Windows, macOS, and Linux — no WSL, no tmux.
