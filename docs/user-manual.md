# Session Deck user manual

Session Deck (`sdeck`) manages your Claude Code and GitHub Copilot CLI sessions from a full-screen
terminal app. Sessions keep running in the background while you switch between them.

Names, archive flags, pins, folders, the sort order, which folders and projects are collapsed, and
which finished sessions you have seen are stored in `~/.session-deck/state.json`.

**Contents**

1. [Statuses](#1-statuses)
2. [Terminal UI](#2-terminal-ui)
3. [Multiple Claude accounts](#3-multiple-claude-accounts)
4. [Global config](#4-global-config-sessiondeckconfigjson)
5. [Files Session Deck uses](#5-files-session-deck-uses)

---

## 1. Statuses

The terminal UI shows each status as a symbol.
a colored status dot on each session.

| Symbol | Status | Meaning |
|---|---|---|
| `●` red | **Running** | The agent is working. |
| `◐` yellow | **Waiting for you** | The agent asked something (a permission, a choice) and waits for your answer. |
| `●` green | **Finished, not seen yet** | The agent finished a turn you haven't looked at yet. |
| `○` | **Idle** | Running and ready for input, nothing new to see. |
| `⟳` | **Starting** | Just started, not ready yet. |
| `✕` red | **Error** | A sign-in or API error on its screen (e.g. "run /login"), or the agent exited with an error. |
| `■` | **Stopped** | Not running. It can be resumed at any time. |
| `↗` | **Open elsewhere** | Running in another terminal. Shown next to its real status. |
- **Seen**: a finished session stops asking for attention once you look at it. In the terminal UI
  that means attaching to it. Session Deck remembers this, also after a restart.
- **Waiting for you** is never cleared by looking: it clears once you answer.

---

## 2. Terminal UI

### Starting it

- **Install**: build with `cargo build --release` and run `target/release/sdeck.exe` from any terminal (put it on your PATH to start it as `sdeck`).
- **Run from a clone**: `cargo run --release` from the repository root, in a real terminal (Windows Terminal, PowerShell, the VS Code terminal).
- **Quit `q` or `Ctrl+C`**: closes Session Deck.
  - Every session running inside it is stopped too; they can be resumed later.
  - If any session is still running or waiting, a confirmation popup opens first (Yes/No, defaults to No; ←→/Tab to move, Enter to pick, Esc cancels).

### The screen

- **Header** (top line): the live status dots, how many sessions run in the background, the theme,
  and the version.
- **Filter pills** (second line): how many sessions have each status, and which filters are on.
- **Sessions panel** (left): folders, projects and their sessions.
  - A number `1`–`9` on the left of a top-level row is its jump key.
  - `▾` means expanded, `▸` collapsed.
  - `↑` / `↓` in front of a title means pinned to the top / bottom of its project.
  - `⇡` / `⇣` / `✱` after a project's count means it's ahead of / behind its upstream, or has
    uncommitted changes — one badge for the whole project (see `ui.gitStatus` in
    [section 4](#4-global-config-sessiondeckconfigjson)), not repeated per session.
- **Preview panel** (right): the live screen of the selected session, or a summary with its last
  response when it isn't running. For a folder or project row it lists its sessions. Its header
  line also spells out that session's own branch and git detail (`⎇main ⇡2 ⇣0 ✱3`).
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
- **Jump between started sessions `[` `]`**: selects the previous / next session that's running,
  waiting, or idle — any session actually started here — skipping stopped and erroring ones, folders
  and projects, and wrapping around from the last back to the first. Useful when the sessions you're
  using are spread across different projects — one press gets you there instead of walking every row
  in between.
  - Reaches into a collapsed folder or project to select one hidden there, expanding it, same as
    `` ` ``. Set `ui.expandCollapsedOnActiveJump` to `false` in the
    [global config](#4-global-config-sessiondeckconfigjson) to only ever jump between sessions
    already shown instead, like `j`/`k`.

### Using a session

- **Attach `Enter`**: shows the selected session full-screen so you can work in it.
  - A stopped session is started (resumed) first.
  - On a folder or project row, `Enter` collapses or expands it instead.
  - A session running in another terminal can't be attached here; close it there first.
  - Attaching marks the session as seen.
- **Detach `Ctrl+Q` (while attached)**: back to Session Deck. The session keeps running in the
  background.
  - Whatever finished while you were attached is marked as seen.
- **Detach and stop `Ctrl+K` then `q` (while attached)**: back to Session Deck, and stops the
  session's agent too — the same as detaching and then pressing `x`.
- **New session `Ctrl+K` then `n` (while attached)**: detaches and starts a new Claude Code session
  in the same project (the same folder, if the attached session is in a subfolder of it) — without
  going back to the list first.
  - Any other key after `Ctrl+K` is sent through to the agent as an ordinary `Ctrl+K` keystroke.
- **Interact `i`**: like attaching, but the sessions panel and preview keep showing — everything
  you type goes straight to the selected session, live, without leaving the list. Useful for sending
  a longer or multi-step reply while still keeping an eye on your other sessions.
  - A stopped session is started first.
  - `Ctrl+Q` stops interacting, the same key that detaches from a full attach; `Ctrl+K` then `q` also
    stops the session's agent.
  - Marks the session as seen, same as attaching.
- **Start in background `s`**: starts a stopped session without attaching. Its screen shows in the
  preview.
- **Stop `x`**: stops the selected session's agent.
  - It stays listed and can be resumed, unless it was a new session where nothing was sent yet;
    that one disappears.
  - With a checked batch (`Space`): stops every checked session that's running, instead of just the
    selected one.
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
    shows in Claude's `/resume`.
  - A Claude session running here gets `/rename <name>` typed into it. It must be idle (or finished)
    for that; otherwise you're asked to wait.
  - A Claude session running in another terminal must be renamed there, with `/rename`.
  - A Copilot session gets a Session Deck name (Copilot has no `/rename`).
  - Projects can't be renamed: they're named after their folder on disk.
- **Mark as unread `u`**: makes the selected session show as finished, not seen yet (`●` green), until
  you look at it again.
- **Mark as read `U`**: the opposite of `u` — clears the "finished, not seen" mark on the selected
  session right away, without having to attach to it.
- **Pin `,` (comma)**: pins the selected session within its project. Each press cycles: pinned to
  the top → pinned to the bottom → not pinned.
  - Pinned sessions stay at the top or bottom of their project whatever the sort order.
- **Archive `A`**: archives the selected session (hides it), or unarchives it in the archived view.
 
  - An idle session is stopped first.
  - A session that's still working isn't archived; you're asked to stop it (`x`) or wait.
  - With a checked batch (`Space`): archives (or unarchives) every checked session, skipping ones
    still working.
- **Archived view `^`**: switches between the active sessions and the archived ones.
  - The panel title shows "· archived" while you're in it.
- **Delete `d` (on a session)**: moves the session to the trash (`~/.session-deck/trash/`). It
  disappears from Session Deck and from Claude's `/resume`.
  - Only stopped sessions can be deleted; stop it first (`x`).
  - Copilot sessions can't be deleted (Copilot keeps them in its own database); archive them
    instead.
  - Items older than 30 days are removed from the trash when Session Deck starts.
  - With a checked batch (`Space`): moves every deletable one to the trash, skipping Copilot
    sessions and ones still running.
- **Undo delete `Ctrl+Z`**: brings back the last deleted session.
  - Press it again for the one deleted before that.
- **Trash `Z`**: lists the deleted sessions, newest first. Select one and press `Enter` to restore
  it.
  - A session can't be restored if a transcript with the same id is back in its place.

### Multi-select

- **Check `Space`** (on a session row): checks it for a batch action, then moves the selection down
  like a normal list. Press it again on a checked session to uncheck it.
  - The header shows how many are checked.
  - `Esc` clears the whole batch first, before it does anything else.
- With sessions checked, **Stop `x`**, **Archive `A`**, **Delete `d`**, **Move to folder `M`** and
  **Tag `L`** act on the whole batch instead of just the selected session — see each key's own entry
  above (and [Tags](#tags) for `L`).
  - Each batch action clears the checks afterward and reports how many it acted on and how many it
    skipped.

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
  - With a checked batch (`Space`): moves every distinct project among the checked sessions to the
    folder you pick, in one go.
- **Move up / down `K` / `J` (or `Shift+↑` / `Shift+↓`)**: moves the selected folder, project, or
  session one place up or down. On a session row, it moves that session within its project, not the
  project itself.
  - Projects never moved keep a fixed alphabetical order by default — moving one keeps it exactly
    where you put it, it won't shuffle as sessions become active. Set `ui.recentProjectsFirst` in
    the [global config](#4-global-config-sessiondeckconfigjson) to sort unmoved projects by most
    recent activity instead, the way earlier versions did.
  - Sessions sort by most recent activity by default, shuffling as they become active. Set
    `ui.recentSessionsFirst` to `false` in the [global config](#4-global-config-sessiondeckconfigjson)
    to freeze that order: the next time each project's sessions are shown, whatever order they're
    currently in is locked in, and from then on a session only moves when you move it with `K`/`J`
    (an existing session Session Deck hasn't shown before is appended, most recent first, the first
    time it's seen — but a session you just started with `n`/`N` always lands at the top). Turning
    the setting off doesn't undo any shuffling that already happened before you turned it off.
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
- **Clear filters `0`**: removes the status, time and tag filters.
- The panel title shows "· filtered" while a filter is on.

### Tags

Free-form labels on individual sessions — orthogonal to folders, so you can group sessions across
different projects without moving anything.

- **Tag `L`**: opens a text prompt for the selected session's own tags (comma-separated), pre-filled
  with its current ones. Submitting replaces the list; clearing the text removes all its tags.
  - With a checked batch (`Space`): adds the tag(s) you type to every checked session, on top of
    whatever tags each one already has.
  - Tags belong to the session, not its project — tagging one session doesn't tag the rest of the
    project's sessions.
- **The `TAGS` section**, at the bottom of the sessions panel: one row per tag in use and how many
  sessions currently carry it. Shown only once at least one session has a tag.
  - **Filter to it `Enter`**: narrows the whole tree to sessions with that tag, wherever they live.
    Press `Enter` on it again to clear the filter (`0` also clears it, along with the status and time
    filters). The active tag also shows as a pill on the filter row.
  - **Remove everywhere `d`**: after confirming, strips that tag from every session that has it.

### Search

- **Search `/`**: opens a full-text search across every session's prompts and replies (not just
  titles), live-filtered as you type.
  - Start the query with a status symbol to also filter by status: `!` running, `@` waiting,
    `#` idle, `&` error, `~` stopped.
  - The first search reads every session's content once (shown as "Reading session content…");
    later searches in the same run are instant.
  - `↑` `↓`: moves the highlighted result. `Enter`: selects that session in the tree (expanding its
    folder/project if collapsed) and closes search. `Esc`: cancels.

### View and look

- **Narrow / widen the sessions panel `<` / `>`**: 5% per press, between 15% and 70% of the width.
  Remembered.
- **Hide / show the sessions panel `b`** (or `Ctrl+K B` while Interacting): gives
  the preview the whole width.
- **Mouse scrolling `m`** (or `Ctrl+K M` while Interacting): toggles real mouse
  reporting. Off by default (not remembered across restarts), so click-drag still does your terminal's
  own text selection, letting you copy from the preview. Turn it on to scroll the preview with the
  wheel or a click-drag; turn it back off to select and copy again. Always off while attached
  (`Enter`) — the agent gets the terminal's mouse events there, not sdeck.
- **Theme `T`**: cycles dark → light → system (Tokyo Night colors). Remembered.
  - **System** follows your terminal's background color, or Windows' dark/light setting if the
    terminal doesn't report it.
- **Refresh `r`**: reloads the session list from disk.
- **Help `?`**: shows every key. `↑` `↓` scroll it; `Esc`, `?` or `q` close it.
- **Config `C`**: shows and edits the settings from `~/.session-deck/config.json` (see
  [section 4](#4-global-config-sessiondeckconfigjson)). `↑` `↓` selects a setting; `Enter` toggles it
  (`ui.notifications`) or opens a text prompt pre-filled with its current value (everything else) —
  submitting saves straight to the file. `Esc`, `C` or `q` close it.
- **Skills / agents `w`**: read-only, two tabs (`← →` to switch):
  - **Skills**: every personal skill directly under `skills/` of the active account's config folder (`~/.claude` for the default account) (Claude Code's own skills
    directory — not `sdeck`'s), grouped by its effective state: `on` (visible + auto-triggerable),
    `name-only` (name visible, no description), `user-invocable-only` (fully hidden from context,
    still in `/`), or `off` (removed entirely, even from `/`). That state is `skillOverrides[name]`
    from Claude Code's `settings.json` in that same folder. With no override it's `user-invocable-only` when
    the skill's frontmatter has `disable-model-invocation: true`, otherwise `on`.
  - **Agents**: every personal subagent directly under `agents/` of that folder, identified by
    name/description from the one `*.agent.md` file inside each subagent's folder. Sorted
    alphabetically — subagents have no `skillOverrides`-style visibility state to group by.

  `↑` `↓` scroll the current tab; `Esc`, `w` or `q` close it.

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

A notification appears when a session starts waiting for you, finishes a turn, or hits an error
(see `ui.notifyStatuses` to change which statuses fire one).

- Titled with the project's name (`Session Deck` when that isn't known), with the Session Deck icon.
- The message is the status followed by the session's title, e.g. `Waiting: <title>`.
- Not for the session you're attached to.
- Clicking the notification selects that session in Session Deck.
- It covers sessions running in other terminals too.

---

## 3. Multiple Claude accounts

Sessions from every logged-in Claude account are listed together.

- **Accounts**: `~/.claude` is the default account. Every sibling folder `~/.claude-<name>` whose
  `.claude.json` has an `oauthAccount` email is another one.
- **Adding an account**: `F4` → "+ Add account…", type the folder name, pick what to share. With a
  project selected, a new Claude then opens as the new account: run `/login` there. It shows in `F4`
  with its email once logged in. (Or run `CLAUDE_CONFIG_DIR=~/.claude-<name> claude` yourself.)
  With `ui.showUsage` on, the usage `statusLine` is added to the new folder.
- **`F4`**: picks the account new sessions launch as. The default account is launched without
  `CLAUDE_CONFIG_DIR`; the others with it set to their folder.
- **`accounts.shareProjects`** (default on): a project used by several accounts shows as one row.
  Off: one row per account.
- **`accounts.showAllSessions`** (default on): list every account's sessions. Off: only the active
  account's.
- **`accounts.showOwner`** (default off): another account's session line also shows its owner's name.
  Off: the session's title is only dimmed.
- Pins, folders and order are kept per folder path, not per account.

---

## 4. Global config (`~/.session-deck/config.json`)

Global settings, mainly for the terminal UI. Press `C` in the
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
    "recentSessionsFirst": true,
    "newSessionFullScreen": true,
    "gitStatus": true,
    "expandCollapsedOnActiveJump": true
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
- **`ui.recentSessionsFirst`**: sorts each project's sessions by most-recent-activity when `true`, so
  they shuffle as they become active — or keeps a fixed order when `false`, changing only when you move
  a session yourself (`K`/`J` on a session row). Default: on.
- **`ui.newSessionFullScreen`**: a new session started with `n`/`N` opens **Attached** (full-screen, as
  if you'd pressed `Enter`) when `true` — or **Interacting** (list and preview still showing, as if
  you'd pressed `i`) when `false`. Default: on.
- **`ui.gitStatus`**: `false` turns off the `⇡`/`⇣`/`✱` project badges and the preview panel's branch
  line entirely — no `git status` is run at all. Default: on.
- **`ui.expandCollapsedOnActiveJump`**: `[`/`]` (jump to the previous/next started session — running,
  waiting or idle) expands a collapsed folder or project to reach one hidden there when `true` — or
  skips it, only ever landing on a session already shown, when `false`. Default: on.
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
| `~/.session-deck/state.json` | Names, archive flags, pins, seen marks, folders, order, sort, collapsed state, and the terminal UI's theme and panel width. Also holds session tags, which only the terminal UI manages and filters by today. |
| `~/.session-deck/config.json` | Global, hand-edited settings — mainly for the terminal UI. |
| `~/.session-deck/trash/` | Sessions deleted in the terminal UI (restorable for 30 days). |
| `~/.claude/session-deck-status/` | Each session's current status. |
| `~/.claude/projects/` | Claude Code's own transcripts. Session Deck reads them, and writes only a title when you rename a stopped session. |
| `~/.copilot/` | Copilot CLI's own sessions. Session Deck only reads them. |
