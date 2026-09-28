# Session Deck user manual

Session Deck manages your Claude Code and GitHub Copilot CLI sessions. It has two front ends that
share the same data:

- **The VS Code extension**: a Session Deck sidebar, plus an "Open Sessions" list in the Explorer.
- **The terminal UI (`sdeck`)**: a full-screen app for any terminal. Sessions keep running in the
  background while you switch between them.

Names, archive flags, pins, folders, the sort order, which folders and projects are collapsed, and
which finished sessions you have seen are shared between the two, in `~/.session-deck/state.json`.
A change in one shows up in the other.

**Contents**

1. [Statuses](#1-statuses)
2. [Terminal UI](#2-terminal-ui)
3. [VS Code extension](#3-vs-code-extension)
4. [Global config](#4-global-config-sessiondeckconfigjson)
5. [Files Session Deck uses](#5-files-session-deck-uses)

---

## 1. Statuses

Both front ends show the same statuses. The terminal UI shows them as symbols; the extension shows
a colored status dot on each session.

| Symbol | Status | Meaning |
|---|---|---|
| `●` green | **Running** | The agent is working. |
| `◐` yellow | **Waiting for you** | The agent asked something (a permission, a choice) and waits for your answer. |
| `◐` yellow | **Finished, not seen yet** | The agent finished a turn you haven't looked at yet. |
| `○` | **Idle** | Running and ready for input, nothing new to see. |
| `⟳` | **Starting** | Just started, not ready yet. |
| `✕` red | **Error** | A sign-in or API error on its screen (e.g. "run /login"), or the agent exited with an error. |
| `■` | **Stopped** | Not running. It can be resumed at any time. |
| `↗` | **Open elsewhere** | Running in another terminal (for example in VS Code). Shown next to its real status. |

- **Seen**: a finished session stops asking for attention once you look at it. In the terminal UI
  that means attaching to it; in the extension, selecting it in the sidebar. Both front ends
  remember this, also after a restart.
- **Waiting for you** is never cleared by looking: it clears once you answer.

---

## 2. Terminal UI

### Starting it

- **Install**: `npm install -g sdeck`, then run `sdeck` from any terminal. Needs Node 22.5 or newer.
- **Run from a clone instead**: `npm run sdeck` from the repository root (after `npm install` and
  `npm run build`), in a real terminal (Windows Terminal, PowerShell, the VS Code terminal).
- **Quit `q` or `Ctrl+C`**: closes Session Deck.
  - Every session running inside it is stopped too; they can be resumed later.

### The screen

- **Header** (top line): the live status dots, how many sessions run in the background, the theme,
  and the version.
- **Filter pills** (second line): how many sessions have each status, and which filters are on.
- **Sessions panel** (left): folders, projects and their sessions.
  - A number `1`–`9` on the left of a top-level row is its jump key.
  - `▾` means expanded, `▸` collapsed.
  - `↑` / `↓` in front of a title means pinned to the top / bottom of its project.
- **Preview panel** (right): the live screen of the selected session, or a summary with its last
  response when it isn't running. For a folder or project row it lists its sessions.
- **Help bar** (bottom line): the most useful keys. It shortens itself on narrow terminals.
- **Terminal title**: `Session Deck · ◐ 2 need you` when sessions are waiting or finished unseen,
  so you can see it from the taskbar or another tab.
- **Layout by width**:
  - 80 columns or more: the panels are side by side.
  - 50 to 79 columns: the sessions panel is above the preview.
  - Under 50 columns: the sessions panel only.

### Moving around

- **Select `↑` `↓` (or `j` `k`)**: moves the selection; the preview follows it.
- **Collapse / go to parent `←` (or `h`)**:
  - On an expanded folder or project: collapses it.
  - On a session: selects its project. On a project inside a folder: selects the folder.
- **Expand / go to child `→` (or `l`)**:
  - On a collapsed folder or project: expands it.
  - On an expanded one: selects its first row.
- **Toggle `Tab`**: collapses or expands the selected folder or project.
- **Jump `1`–`9`**: selects the top-level folder or project with that number.
- **Previous session `` ` `` (backtick)**: goes back to the session you had selected before.
  - If its folder or project is collapsed, it is expanded.
  - If a filter hides it, you're told to clear the filter (`0`).

### Using a session

- **Attach `Enter`**: shows the selected session full-screen so you can work in it.
  - A stopped session is started (resumed) first.
  - On a folder or project row, `Enter` collapses or expands it instead.
  - A session running in another terminal can't be attached here; close it there first.
  - Attaching marks the session as seen.
- **Detach `Ctrl+Q` (while attached)**: back to Session Deck. The session keeps running in the
  background.
  - Whatever finished while you were attached is marked as seen.
- **Type into it `i`**: like attaching, but the sessions panel and preview keep showing — everything
  you type goes straight to the selected session, live, without leaving the list. Useful for sending
  a longer or multi-step reply while still keeping an eye on your other sessions.
  - A stopped session is started first.
  - `Ctrl+Q` stops it, the same key that detaches from a full attach.
  - Marks the session as seen, same as attaching.
- **Start in background `s`**: starts a stopped session without attaching. Its screen shows in the
  preview.
- **Stop `x`**: stops the selected session's agent.
  - It stays listed and can be resumed, unless it was a new session where nothing was sent yet;
    that one disappears.
- **Restart `R`**: stops the agent and starts it again on the same conversation. Useful after
  changing settings or MCP servers.
  - A stopped session is just started.
  - A new Claude session where nothing was sent yet starts over as a new session.
- **New Claude session `n`**: starts a new Claude Code session in the selected project and attaches
  to it.
  - With a session selected, it starts in that session's folder; with a project selected, in the
    project's root.
- **New Copilot session `N`**: the same, with GitHub Copilot CLI.
- **Add project `p`**: asks for a folder (full path, `~` works) and starts a new Claude Code session
  there, for a project that isn't listed yet (new, or moved to another folder).
  - The session joins the folder's git project, or starts one.
  - Projects come from your session history: the project stays listed once you send a prompt.
  - Uses Copilot when Claude is disabled in the config.
- **Send a prompt `o`**: types a one-line prompt into the selected session without attaching.
  - Type the prompt at the bottom and press `Enter` (`Esc` cancels).
  - A stopped session is started first; the prompt is typed as soon as the agent is ready (idle
    and quiet for a second).
  - An idle session gets it right away. A session that's still working gets it once it's done
    (idle), so its current work is never interrupted.
  - Refused while the session is waiting for an answer, because typing would answer its question.
    Attach (`Enter`) to reply.
  - Refused for a session running in another terminal.
- **Copy last response `c`**: copies the session's last response to the clipboard.
  - It uses the terminal's clipboard support (OSC 52), which Windows Terminal has and which also
    works over SSH.
- **Rename `e` (or `F2`)**: renames the selected session, or the selected folder.
  - The current name is filled in; edit it and press `Enter`.
  - A stopped Claude session gets the same title Claude's own `/rename` sets, so the name also
    shows in Claude's `/resume` and in the extension.
  - A Claude session running here gets `/rename <name>` typed into it. It must be idle (or finished)
    for that; otherwise you're asked to wait.
  - A Claude session running in another terminal must be renamed there, with `/rename`.
  - A Copilot session gets a Session Deck name (Copilot has no `/rename`).
  - Projects can't be renamed: they're named after their folder on disk.
- **Mark as unread `u`**: makes the selected session show as finished, not seen yet (`◐`), until you
  look at it again. Shared with the extension.
- **Pin `,` (comma)**: pins the selected session within its project. Each press cycles: pinned to
  the top → pinned to the bottom → not pinned.
  - Pinned sessions stay at the top or bottom of their project whatever the sort order.
  - In the extension, pinned sessions are also never auto-archived.
- **Archive `A`**: archives the selected session (hides it), or unarchives it in the archived view.
  Shared with the extension.
  - An idle session is stopped first.
  - A session that's still working isn't archived; you're asked to stop it (`x`) or wait.
- **Archived view `^`**: switches between the active sessions and the archived ones.
  - The panel title shows "· archived" while you're in it.
- **Delete `d` (on a session)**: moves the session to the trash (`~/.session-deck/trash/`). It
  disappears from Session Deck and from Claude's `/resume`.
  - Only stopped sessions can be deleted; stop it first (`x`).
  - Copilot sessions can't be deleted (Copilot keeps them in its own database); archive them
    instead.
  - Items older than 30 days are removed from the trash when Session Deck starts.
- **Undo delete `Ctrl+Z`**: brings back the last deleted session.
  - Press it again for the one deleted before that.
- **Trash `Z`**: lists the deleted sessions, newest first. Select one and press `Enter` to restore
  it.
  - A session can't be restored if a transcript with the same id is back in its place.

### Folders and order

Projects are grouped automatically by their git root: worktrees and subfolders of the same
repository form one project. Folders let you group projects further, one level deep.

- **New folder `g`**: asks for a name and creates the folder. It's shown above the top-level
  projects.
- **Move to folder `M`**: moves the selected project (or the project of the selected session) into
  a folder. A list opens:
  - **Top level (no folder)**: takes it out of its folder.
  - **A folder name**: moves it there.
  - **+ New folder…**: asks for a name, creates the folder and moves the project into it.
- **Move up / down `K` / `J` (or `Shift+↑` / `Shift+↓`)**: moves the selected folder, or the selected
  project (a session moves its project), one place up or down.
  - Projects never moved keep a fixed alphabetical order by default — moving one keeps it exactly
    where you put it, it won't shuffle as sessions become active. Set `ui.recentProjectsFirst` in
    the [global config](#4-global-config-sessiondeckconfigjson) to sort unmoved projects by most
    recent activity instead, the way earlier versions did.
  - Only in the normal view; in the "active on top" view you're asked to switch back (`t`).
- **Rename folder `e`**: on a folder row, renames it.
- **Delete folder `d`**: on a folder row, deletes the folder after you confirm with `y`. Its projects
  move back to the top level; no session is touched.
- **Sort `S`**: switches how sessions are ordered inside each project:
  - **Most recent first** (default).
  - **Needs attention first**: error, waiting, running, idle, then the rest.
- **View `t`**: switches between the normal view and **active on top**, where folders and projects
  with running or waiting sessions come first, above an `── idle / done` divider.

### Filters

- **Running `!`, Waiting `@`, Idle `#`, Error `&`, Stopped `~`**: each key shows only
  sessions with that status. Press it again to remove it. Several can be on at once.
  - "Waiting" includes finished-not-seen sessions.
  - Folders and projects with nothing matching are hidden.
- **Time `*`**: cycles through all time → today → last 3 days → last 7 days.
- **Clear filters `0`**: removes the status and time filters.
- The panel title shows "· filtered" while a filter is on.

### Search

- **Search `/`**: opens a full-text search across every session's prompts and replies (not just
  titles), live-filtered as you type.
  - Start the query with a status symbol to also filter by status: `!` running, `@` waiting,
    `#` idle, `&` error, `~` stopped.
  - The first search reads every session's content once (shown as "Reading session content…");
    later searches in the same run are instant.
  - `↑` `↓`: moves the highlighted result. `Enter`: selects that session in the tree (expanding its
    folder/project if collapsed) and closes search. `Esc`: cancels.
  - Shared with the extension's own "Search Sessions" command, but read independently.

### View and look

- **Narrow / widen the sessions panel `<` / `>`**: 5% per press, between 15% and 70% of the width.
  Remembered.
- **Hide / show the sessions panel `b` (or `Ctrl+B`)**: gives the preview the whole width.
- **Theme `T`**: cycles dark → light → system (Tokyo Night colors). Remembered.
  - **System** follows your terminal's background color, or Windows' dark/light setting if the
    terminal doesn't report it.
- **Refresh `r`**: reloads the session list from disk.
- **Help `?`**: shows every key. `↑` `↓` scroll it; `Esc`, `?` or `q` close it.
- **Config `C`**: shows and edits the settings from `~/.session-deck/config.json` (see
  [section 4](#4-global-config-sessiondeckconfigjson)). `↑` `↓` selects a setting; `Enter` toggles it
  (`ui.notifications`) or opens a text prompt pre-filled with its current value (everything else) —
  submitting saves straight to the file. `Esc`, `C` or `q` close it.

### Text input, lists and questions

- **Text input** (rename, new folder, prompt):
  - `Enter`: confirms.
  - `Esc` or `Ctrl+C`: cancels.
  - `Backspace`: deletes a character. `Ctrl+U`: clears the text.
  - Pasting works.
- **Lists** (move to folder, trash):
  - `↑` `↓` (or `j` `k`): selects an item.
  - `Enter`: picks it. `Esc` or `q`: closes the list.
- **Questions** (delete folder): `y` confirms; any other key cancels.

### Desktop notifications

A notification appears when a session starts waiting for you, finishes a turn, or hits an error.

- Not for the session you're attached to.
- Clicking the notification selects that session in Session Deck.
- It covers sessions running in other terminals too (for example in VS Code).
- With the extension also running, each event shows once, not twice.

---

## 3. VS Code extension

### The views

- **Sessions** (Session Deck icon in the Activity Bar): the projects of this workspace, their
  sessions, and your folders.
  - With both Claude Code and Copilot sessions, there's a folder per agent on top.
  - Folders (shared with the terminal UI) come first, then top-level projects.
  - A folder shows only if it holds at least one project of this workspace's list.
  - Pinned sessions show 📌.
  - Each project shows its 5 most recent sessions (see `maxSessionsShown`); older ones move to its
    **Archived** folder.
- **Open Sessions** (in the Explorer sidebar, collapsed at first): the sessions you have open in
  terminals right now, across all projects, with their status.

### Keybindings

- **Search Sessions `Ctrl+K S` (`Cmd+K S` on macOS)**: opens the session search.
- **New Session `Ctrl+K G` (`Cmd+K G` on macOS)**: starts a new session. Without a project selected,
  you choose the project and the agent first.

### Toolbar of the Sessions view

- **Search Sessions** (magnifier): fuzzy search across the full content of every session, best
  matches first.
  - Start the search with a symbol to show only sessions with that status: `!` running, `@`
    waiting, `#` finished, `~` error.
- **Add Project to Workspace** (+): lists the projects found in your Claude Code and Copilot
  history, or lets you browse to any folder. The project is added to this workspace's
  `.vscode/session-deck.json`.
- **Edit Project List** (`{}`): opens `.vscode/session-deck.json`.
- **Refresh Sessions** (circular arrow): reloads everything.
- **New Folder…** (in the `…` menu): asks for a name and creates a folder.
- **Sort Sessions…** (in the `…` menu): **Most recent first** or **Needs attention first** (error,
  waiting, running, idle, then the rest). Shared with the terminal UI.

### Session actions (right-click a session)

- **Resume Session in Terminal** (click the session): opens it in a terminal with the real
  `claude` / `copilot` CLI.
  - Each session has its own terminal; clicking it again brings that terminal back instead of
    opening a new one.
  - An archived session is unarchived first.
  - Selecting a session marks its finished status as seen.
- **Resume Session in Terminal (Skip Permissions)**: the same, with the agent's skip-permissions
  mode (`--dangerously-skip-permissions` for Claude, `--allow-all` for Copilot).
  - A confirmation dialog comes first, unless the `sessionDeck.confirmDangerousSkipPermissions`
    setting is off or the project has `dangerouslySkipPermissions: true`.
- **View Transcript**: opens the session's conversation as a read-only document.
- **Rename Session**: renames the session.
  - A stopped Claude session gets the same title Claude's own `/rename` sets, so the name also
    shows in Claude's `/resume` and in the terminal UI.
  - A Claude session running idle in a VS Code terminal gets `/rename <name>` typed into it.
  - A Claude session that's busy: you're asked to rename it once it's idle.
  - A Claude session running in another terminal gets a Session Deck name only; run `/rename`
    there to rename it in Claude too.
  - A Copilot session gets a Session Deck name (Copilot has no `/rename`).
- **Archive Session**: moves it to its project's **Archived** folder. Shared with the terminal UI.
- **Unarchive Session** (on an archived session): moves it back.
- **Pin…**: **Pin to top**, **Pin to bottom** of its project, or **Unpin**. Shared with the terminal
  UI.
  - Pinned sessions don't count toward the project's session limit and are never auto-archived.
- **Mark as Unread**: shows the session's finished status again until you look at it. Shared with
  the terminal UI.
- **Copy Last Response**: copies the agent's last reply to the clipboard.
- **Copy Session Info**: copies the name, agent, project, working folder, session id, last change,
  status and transcript location.
- **Fork Session** (Claude Code only): starts a new session that continues from this one's full
  history, leaving the original untouched.
- **Fork Session (Skip Permissions)**: the same, in skip-permissions mode.

### Project actions (right-click a project)

- **New Session**: starts a new session in the project.
- **New Session (Skip Permissions)**: the same, in skip-permissions mode.
- **Rename Project**: changes its display name in this workspace (`name` in `session-deck.json`).
- **Remove Project**: takes it off this workspace's list, after you confirm. Its sessions are
  untouched and it can be added back with **Add Project**.
- **Hide Project**: keeps it in `session-deck.json` but leaves it out of the tree. To show it again,
  set `"hidden": false` in the file (**Edit Project List**).
- **Move to Folder…**: **Top level (no folder)**, one of your folders, or **New folder…**.
- **Move Up** / **Move Down**: moves the project one place within its folder or the top level.

### Folder actions (right-click a folder)

- **Rename Folder**: renames it.
- **Delete Folder**: deletes it after you confirm. Its projects move back to the top level.
- **Move Up** / **Move Down**: moves the folder one place.

### Collapsing

Folders and projects start expanded. Whatever you collapse is remembered, in the terminal UI too.

### Notifications

A desktop notification appears when a session you opened from VS Code starts waiting for your input
or exits with an error. Clicking it brings that session's terminal to the front.

### Settings

- **`sessionDeck.confirmDangerousSkipPermissions`** (default: on): asks for confirmation before
  resuming or starting a session in skip-permissions mode.

### `.vscode/session-deck.json`

The list of projects this workspace shows. **Add Project**, **Rename Project**, **Hide Project** and
**Remove Project** edit it for you; **Edit Project List** opens it.

```jsonc
{
  "projects": [
    { "root": "C:/Source/my-project", "name": "My Project" },
    { "root": "C:/Source/side-project", "emoji": "🚀", "maxSessionsShown": 10 }
  ]
}
```

- **`root`** (required): the project's git root.
- **`name`**: the name shown in the tree. Default: the folder name.
- **`emoji`**: an emoji shown before the name.
- **`color`**: a colored dot shown before the name instead: `red`, `orange`, `yellow`, `green`,
  `blue`, `purple`, `brown`, `black` or `white`. Ignored when `emoji` is set.
- **`maxSessionsShown`**: how many sessions show before older ones auto-archive. Default: 5.
- **`dangerouslySkipPermissions`**: `true` skips the skip-permissions confirmation for this project.
- **`hidden`**: `true` keeps the entry but hides the project.

In a multi-root workspace, each folder can have its own file; the tree shows all of them together.

---

## 4. Global config (`~/.session-deck/config.json`)

Global settings, mainly for the terminal UI (the extension has no equivalent yet). Press `C` in the
terminal UI to view and edit it directly (see **Config `C`** above), or edit the file by hand — either
way, a missing field keeps that setting's default. Editing through `C` takes effect right away, except
for `trash.retentionDays` (checked only at startup) and any hand-made change to the file (`sdeck` picks
it up next time it starts).

```jsonc
{
  "ui": {
    "maxSessionsListed": 30,
    "notifications": true,
    "notifyStatuses": ["waiting", "done", "error"],
    "recentProjectsFirst": false,
    "newSessionFullScreen": true
  },
  "tools": {
    "claude": { "command": "claude-nightly", "args": ["--model", "opus"] },
    "copilot": { "enabled": false }
  },
  "trash": {
    "retentionDays": 30
  }
}
```

- **`ui.maxSessionsListed`**: how many of the most recent Claude and Copilot sessions the terminal UI
  loads from disk. Default: 30.
- **`ui.notifications`**: turns desktop notifications off entirely. Default: on.
- **`ui.notifyStatuses`**: which statuses `notifications` fires for, space-separated: `running`,
  `waiting`, `done`, `error`. Default: `waiting done error` (not `running`).
- **`ui.recentProjectsFirst`**: sorts top-level projects you haven't manually moved (`K`/`J`) by
  most-recent-activity when `true`, so they shuffle as sessions become active — a fixed alphabetical
  order when `false`. Default: off.
- **`ui.newSessionFullScreen`**: a new session started with `n`/`N` attaches full-screen, as if you'd
  pressed `Enter`, when `true` — or opens it in the preview pane, as if you'd pressed `i`, when
  `false`. Default: on.
- **`tools.claude` / `tools.copilot`**:
  - **`enabled`**: `false` hides that agent entirely — its sessions aren't discovered, and its `n`/`N`
    new-session key just flashes a message instead of starting one. Default: on.
  - **`command`**: replaces the executable Session Deck spawns for that agent (a bare name resolved on
    PATH, or a full path). Skips the default `where.exe` lookup on Windows.
  - **`args`**: extra arguments appended after the ones Session Deck builds itself (`--resume <id>`,
    etc).
- **`trash.retentionDays`**: how many days a deleted session stays restorable in
  `~/.session-deck/trash/` before being purged at startup. Default: 30.

---

## 5. Files Session Deck uses

| Location | What's there |
|---|---|
| `~/.session-deck/state.json` | Shared by both front ends: names, archive flags, pins, seen marks, folders, order, sort, collapsed state, and the terminal UI's theme and panel width. |
| `~/.session-deck/config.json` | Global, hand-edited settings — mainly for the terminal UI. |
| `~/.session-deck/trash/` | Sessions deleted in the terminal UI (restorable for 30 days). |
| `~/.claude/session-deck-status/` | Each session's current status, written by both front ends. |
| `<workspace>/.vscode/session-deck.json` | The extension's project list for that workspace. |
| `~/.claude/projects/` | Claude Code's own transcripts. Session Deck reads them, and writes only a title when you rename a stopped session. |
| `~/.copilot/` | Copilot CLI's own sessions. Session Deck only reads them. |
