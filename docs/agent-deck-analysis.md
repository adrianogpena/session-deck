# agent-deck analysis and Session Deck action plan

Brainstorm document, 2026-09-26. Nothing here is implemented yet except where marked **(done)**.

Source: [asheshgoplani/agent-deck](https://github.com/asheshgoplani/agent-deck) `main`, v1.16.19. Read from the
README, `docs/status-detection.md`, `skills/agent-deck/references/tui-reference.md`, `internal/ui/*`,
`internal/session/*`, `internal/statedb/statedb.go` and `internal/tmux/pty.go`. `home.go` (25.7k lines) was sampled, not read in full.

Goal: take the good ideas without copying the project, and without tmux or WSL.

---

## 1. What agent-deck is

| Aspect | agent-deck |
|---|---|
| Language / TUI | Go, Bubble Tea, Lip Gloss, `charmbracelet/x/vt` (terminal emulator), `creack/pty` |
| Platforms | macOS, Linux, WSL (no native Windows) |
| Session hosting | Every session is a tmux session on a private socket (`tmux -L agent-deck`) |
| Preview | `tmux capture-pane` every 2 s |
| Send input | `tmux send-keys` |
| Attach | Runs a real `tmux attach` in a PTY and proxies it. It watches stdin for Ctrl+Q as `0x11`, `ESC[27;5;81~` or `ESC[81;5u`. |
| Embedded pane (opt-in) | tmux client in a PTY, rendered into the right pane via Charm VT |
| Storage | SQLite `~/.local/share/agent-deck/<profile>/state.db` (tables `instances`, `groups`, `cost_events`, …) |
| Config | TOML `~/.config/agent-deck/config.toml` (`[ui]`, `[hotkeys]`, `[groups."path"]`, `[tools.*]`, `theme`, …) |

**Relation to Session Deck:** the prototype already uses the same model: a background PTY per session, a
terminal-emulator mirror, attach/detach with Ctrl+Q in several encodings, and an embedded-pane option later. The one
real difference is **who keeps sessions alive**. agent-deck has the tmux server; Session Deck has the `sdeck` process.

---

## 2. Ideas worth adopting

### 2.1 Session tree

agent-deck groups are **manual, nested, slash-path folders** (`work/devops`), not derived from the project path. A
default "My Sessions" group holds everything else. Session Deck groups **automatically by git project**, which is a
strength worth keeping.

**Proposal: hybrid.** Keep automatic project groups, and add optional manual folders that contain projects:

```
▾ Work                     (8)  ● 1  ◐ 1
  ▾ miles-core             (5)  ● 1
    ├─ ● VWDE-24811 action plan              claude   5m ago
    └─ ○ Quote calc refactor                 claude   2d ago
  ▸ miles-ria              (3)  ◐ 1
▾ Personal
  ▾ session-deck           (4)
    ├─ ● VSCode extension to terminal CLI    claude   just now
    └─ ○ Extension app icons                 copilot  3d ago
```

Details to take:

- **Group row:** `▾ Name (count) ● N ◐ N`. The count includes sub-groups, and the running and waiting counts show
  only when above zero. You see "something in Work needs me" even with the group collapsed.
- **Session row:** tree connectors `├─` / `└─` / `│ `, a status glyph, the title, the agent name in its brand color,
  and a relative time (`just now`, `5m ago`, `2h 10m ago`, `3d 4h ago`).
- **Status without color:** titles are **bold when running or waiting** and underlined on error.
- **Pins:** pinned-top and pinned-bottom bands; `,` cycles off, top, bottom.
- **Sort modes:** creation (manual order) or "actionable" (error > waiting > running > idle, then most recently used).
- **View modes (`t`):** normal; "active on top" (groups with activity first, then an `idle / done` divider);
  "populated on top".
- **Filter pills** under the header: `All ● 3 ◐ 1 ○ 12 ✕ 0`. `!` `@` `#` `&` toggle a status, `0` clears, and `*`
  cycles the time filter (today / 3 days / 7 days / all).
- **Navigation:** `1`–`9` jump to a top-level group; `` ` `` goes to the previous session; `Alt+←` / `Alt+→` walk the
  recently used list; `Space` opens a Vimium-style jump mode.
- **Persistence:** collapsed and expanded state survives restarts.
- **Sub-sessions:** one level of nesting under a parent session (`Shift+→` / `Shift+←`). This is optional for us;
  it's useful for forks.

### 2.2 Colors (Tokyo Night)

Exact values from `internal/ui/styles.go`:

| Role | Dark | Light |
|---|---|---|
| Background | `#1a1b26` | `#d5d6db` |
| Surface (header, dialogs) | `#24283b` | `#e9e9ec` |
| Border | `#414868` | `#9699a3` |
| Text | `#c0caf5` | `#343b58` |
| Dim text / comments | `#787fa0` | `#6a6d7c` |
| Accent (selection, title, Copilot) | `#7aa2f7` | `#34548a` |
| Cyan (group names, panel titles) | `#7dcfff` | `#166775` |
| Purple (dialogs, key hints) | `#bb9af7` | `#7847bd` |
| Green, running | `#9ece6a` | `#485e30` |
| Yellow, waiting | `#e0af68` | `#8f5e15` |
| Orange (Claude) | `#ff9e64` | `#965027` |
| Red, error | `#f7768e` | `#8c4351` |

**Status glyphs:**

| Glyph | Meaning | Style |
|---|---|---|
| `●` | running | Green, bold |
| `◐` | waiting for you | Yellow, bold |
| `○` | idle | Dim |
| `✕` | error | Red, bold |
| `■` | stopped / archived | Dim |
| `⟳` | starting | Yellow |

These replace the prototype's current `▶ ? ● … ✕ ⧉ ·` mix.

**Other styling:**

- **Selection:** a full-width accent bar with dark text; connectors, glyph and tool label on the row flip to match.
- **Group names:** bold Cyan.
- **Dialogs:** rounded Purple border on the Surface background.
- **Search match:** background-color text on Yellow.
- **Theme:** `dark` / `light` / `system`. The system option follows the OS setting live; on Windows, read the
  registry value `AppsUseLightTheme`.
- **VS Code extension:** keep VS Code theme colors, mapped to the same meanings (`charts.green`, `charts.yellow`,
  `charts.red`, `charts.orange`), so glyphs and meanings match across both front ends.

### 2.3 Layout

- **Top to bottom:** header bar (Surface background, logo, title, counts), filter pills, a sessions panel and a
  preview panel, and an adaptive help bar at the bottom.
- **Split:** 35 % sessions / 65 % preview by default. `<` and `>` resize it in 5 % steps, and the value is saved.
- **Breakpoints:** 80 columns or more puts the panels side by side; 50–79 stacks them; under 50 shows only the list.
- **Preview header:** bold title, status badge, `📁 path`, `⏱ active now` or a relative time, and small labels for
  the agent and the group.
- **Help overlay (`?`):** scrollable, with sections (Quick start, Navigation, Sessions, Groups, Search & filter, Other).
- **Empty states:** "No sessions in <group>" plus hints such as "Press n to create a session, g to create a group".
- **Delete:** confirms with `y`/`n`, then allows 30 s of `Ctrl+Z` undo. Deleting a group moves its sessions to the
  default group.

### 2.4 Status model

- **Waiting = stopped and not yet seen; idle = stopped and seen.** Attaching marks a session as seen, and `u` marks it
  unseen again. Session Deck's `acknowledgeSessionStatus` already covers half of this; the TUI must call it when you
  attach.
- **agent-deck's layers:** hooks written into Claude's `settings.json` (`UserPromptSubmit`, `Stop`,
  `PermissionRequest`, `Notification`), plus regexes over the visible screen text.
- **Session Deck keeps its zero-setup sources:** Claude's own `~/.claude/sessions/<pid>.json` and Copilot's events
  log. For Copilot this is better than agent-deck, which only reads Copilot's screen.
- **Add a screen-reading fallback over the headless xterm buffer**, mainly for errors, which the status files don't
  report:
  - running: spinner line `✳ Word… (…)`, `esc to interrupt`, a Braille spinner in the window title (from xterm's
    title-change events)
  - waiting: `Allow once`, `Enter to select`, the trust prompt
  - error: `API Error: 401`, `Please run /login`

---

## 3. What works without tmux

| Feature | Verdict | How |
|---|---|---|
| Tree, groups, pins, sort/view modes, filters, jumps | ✅ Portable | Pure data plus renderer |
| Palette, glyphs, light/dark/system | ✅ Portable | Truecolor escape codes in the TUI, theme colors in the extension |
| Preview, attach/detach, Ctrl+Q | ✅ **(done)** in the prototype | headless xterm plus `@xterm/addon-serialize` |
| Rename | ✅ **(done)** in the prototype | Claude's `custom-title` + `agent-name` records, or `/rename` when running |
| AI titles | ✅ **(done)** in both | Claude's `ai-title` records |
| One-line prompt without attaching (`o`) | ✅ Portable | `pty.write(text)`, then `\r` |
| Quick-approve (`a`) | ⚠️ Risky | Only when the screen shows a permission menu, with explicit confirmation |
| Screen-reading status and errors | ✅ Portable | Regexes over the headless buffer |
| Desktop notifications | ✅ Portable | Windows toast (e.g. `node-notifier`) |
| Fork, archive, delete with undo | ✅ Portable | Fork command exists already; undo is a delayed delete |
| Global search | ✅ Exists | "Search Sessions"; a full-text index later |
| Cost dashboard | ✅ Portable | Parse usage from transcripts |
| MCP / skills / plugins managers | ✅ Portable, big | Config file editing |
| Git worktrees | ✅ Portable | Setup scripts in PowerShell instead of `sh` |
| Web UI | ✅ Portable, later | node-pty with xterm.js in the browser |
| **Sessions survive closing sdeck** | ❌ Needs a daemon | A background `sdeck` process that owns the PTYs and serves them over a Windows named pipe; the TUI and the extension both connect |
| Several viewers of one session | ❌ Needs the daemon | The daemon sends output to all clients |
| Conductor (agents driving agents, Telegram/Slack) | ❌ Needs the daemon | Out of scope for now |
| Import tmux sessions, tmux status bar, MCP socket pool over Unix sockets | ⛔ Skip | tmux- or Unix-specific |
| Docker sandbox, remote over SSH | ⛔ Skip for now | Separate products |

---

## 4. Action plan

### Phase 0: decisions

See [section 5](#5-open-decisions).

### Phase 1: shared foundation

- Monorepo with npm workspaces: `packages/core`, `packages/vscode`, `packages/cli`.
- Move the vscode-free modules into `core`: discovery, status, command builders, fuzzy match, concurrency, archive
  policy.
- **Shared state store** in `~/.session-deck/`: groups, pins, collapsed state, archive, names and settings. These live
  in VS Code's `globalState` today (`src/config/state.ts`), which the CLI cannot read, so they need a one-time
  migration.
- Port the prototype to TypeScript in `packages/cli`, split into store, PTY sessions, status, renderer and input.

### Phase 2: look and feel (TUI)

- Tokyo Night palette with dark / light / system.
- `●◐○✕■⟳` glyphs, accent selection bar, bold active titles.
- Header, filter pills, 35/65 split with `<` `>`, adaptive help bar, `?` overlay.
- Row format with connectors, agent color and relative time.

### Phase 3: tree model (both front ends)

- Hybrid groups: automatic projects plus manual folders.
- `g` create folder, `M` move, `K` / `J` reorder, `,` pin.
- Group counts with `●` and `◐`, remembered collapse state, `1`–`9` jumps, `` ` `` previous session.
- Sort modes and the "active on top" view.

### Phase 4: status and attention

- Seen/unseen model (waiting vs idle), with `u` to mark unseen.
- Screen-reading fallback for errors (401, `/login`).
- Toast notifications; waiting count in the terminal title.

### Phase 5: session actions

- `o` one-line prompt, `c` copy last response, `R` restart.
- Archive / unarchive, delete with undo.
- Copilot sessions in the TUI (spawn `copilot --resume`).
- Extension "Rename" writes Claude's `/rename` records instead of VS Code-only storage.

### Phase 6: bigger items (each optional)

- **Daemon:** sessions survive closing sdeck, and several viewers can watch one session.
- **Embedded interactive pane** (variant B).
- Cost dashboard, worktree workflow, MCP / skills manager, web UI.

---

## 5. Open decisions

1. **Grouping:** hybrid (automatic projects plus manual folders, *recommended*), manual only like agent-deck, or
   automatic only?
2. **Daemon priority:** is "sessions end when sdeck closes" acceptable for v1 (daemon in Phase 6), or is it a
   must-have (daemon in Phase 1)?
3. **Theme:** adopt Tokyo Night as-is, or use it as the base for a Session Deck palette of your own?
4. **State store:** JSON (simple, readable by hand) or SQLite via `node:sqlite` (safer when the TUI and the
   extension write at the same time)?

---

## Appendix: agent-deck keybindings (reference)

| Area | Keys |
|---|---|
| Move | `j`/`k`, `↑`/`↓`, `Ctrl+u`/`Ctrl+d`, `PgUp`/`PgDn`, `Home`/`End`, `gg` |
| Tree | `h`/`←` collapse or go to parent, `l`/`→`/`Tab` toggle, `Enter` attach or toggle, `1`–`9` root group, `Space` jump mode |
| History | `` ` `` previous session, `Alt+←`/`Alt+→` recently used list |
| Sessions | `n` new, `N` quick create, `r` rename, `R` restart, `d` delete, `D` close, `Ctrl+Z` undo, `A` archive, `M` move, `,` pin, `K`/`J` reorder, `f`/`F` fork |
| Interact | `o` one-line prompt, `a` quick approve, `u` mark unread, `x` send output to another session |
| Copy | `c` last response, `C` session info, `V` visible pane, `Y` code block |
| View | `v` preview mode, `O` preview orientation, `<`/`>` resize, `t` group view mode |
| Search / filter | `/` fuzzy, `G` global, `!` `@` `#` `&` `%` `^` status filters, `0` clear, `*` time filter |
| Managers | `m` MCP, `s` skills, `L` plugins, `$` costs, `w` watchers |
| Global | `?` help, `S` settings, `Ctrl+R` reload, `Ctrl+Q` detach, `q` / `Ctrl+C` quit |
