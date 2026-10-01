import { randomUUID } from 'crypto';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import {
  acknowledgeSessionStatus,
  appendClaudeRenameRecords,
  clearSessionStatus,
  copilotSessionSearchText,
  CopilotStatusWatcher,
  lastCopilotAssistantResponse,
  listTrash,
  mapWithConcurrency,
  purgeTrash,
  fuzzyMatch,
  readSessionSearchText,
  restoreSession,
  trashClaudeSession,
  ClaudeProcessWatcher,
  clearSessionMetaCache,
  createFolder,
  DeckStore,
  deleteFolder,
  expandHome,
  folderNodeKey,
  freezeSessionOrder,
  humanizeSince,
  markSessionUnseen,
  getDeckConfigPath,
  moveFolder,
  moveProject,
  moveProjectToFolder,
  moveSession,
  normalizeFsPath,
  prependSession,
  projectNodeKey,
  readDeckConfig,
  readLastAssistantResponse,
  renameFolder,
  renameSessionId,
  resolveProjectRoot,
  SessionPin,
  setCollapsed,
  SIDEBAR_PCT_MAX,
  SIDEBAR_PCT_MIN,
  ThemePreference,
  TreePrefs,
  WaitingNotifier,
  writeDeckConfig,
} from '@session-deck/core';
import { ESC, fitAnsi, oneLine } from './ansi';
import { CONFIG_FIELDS } from './configFields';
import { matchesStatusFilter, nextTimeFilter, STATUS_CATEGORIES, StatusCategory, TimeFilter, withinTimeFilter } from './filters';
import { GitStatusTracker } from './gitStatusTracker';
import {
  DISABLE_MOUSE,
  ENABLE_MOUSE,
  findChordKey,
  findDetachKey,
  findPlainKey,
  parseMouseSequence,
  RESET_AGENT_MODES,
  splitKeys,
} from './keys';
import { discoverLocalAgents, LocalAgent } from './agents';
import { computeLayout, Layout, ptySizeFor, Rect } from './layout';
import { AgentType, clearExecutableCache, disposeLive, LiveSession, resizeLive, spawnAgent, typeLine, warmExecutables } from './liveSession';
import { DeckSession, StatusTracker, discoverSessions, displayTitle, refreshLiveTitle } from './sessions';
import { discoverLocalSkills, LocalSkill } from './skills';
import { buildTree, isStarted, projectLabels, TreeOptions, TreeRow } from './tree';
import { extractBackgroundReply, OSC11_QUERY, readOsTheme, Theme, ThemeName } from './theme';
import {
  configOverlay,
  GroupPreview,
  helpOverlay,
  ListRow,
  pickerOverlay,
  quitConfirmOverlay,
  renderConfirmBar,
  renderGroupPreviewPanel,
  renderHeader,
  renderHelpBar,
  renderListPanel,
  renderMessageBar,
  renderPills,
  renderPreviewPanel,
  renderPromptBar,
  searchOverlay,
  SearchResultRow,
  SessionView,
  skillsOverlay,
  SkillsPopupTab,
  themeLabel,
} from './view';

const out = process.stdout;
const VERSION: string = JSON.parse(fs.readFileSync(path.join(__dirname, '..', 'package.json'), 'utf8')).version;
const DEFAULT_SIDEBAR_PCT = 35;
const SIDEBAR_STEP = 5;
const THEME_CYCLE: readonly ThemePreference[] = ['dark', 'light', 'system'];
const THEME_POLL_MS = 5000;
const GIT_STATUS_POLL_MS = 5000;
const DAY_MS = 24 * 60 * 60 * 1000;
/** Caps how many sessions' content search reads concurrently on first use. */
const SEARCH_READ_CONCURRENCY = 8;
const FILTER_KEYS: Record<string, StatusCategory> = { '!': 'running', '@': 'waiting', '#': 'idle', '&': 'error', '~': 'stopped' };

/** Footer text input while open. */
interface Prompt {
  label: string;
  value: string;
  onSubmit(value: string): void;
}

/** Centered list to choose from while open. */
interface Picker {
  title: string;
  items: string[];
  index: number;
  onPick(index: number): void;
}

/** Footer yes/no question while open: `y` confirms, any other key cancels. */
interface Confirm {
  question: string;
  onYes(): void;
}

/** Centered "quit with active sessions" warning while open: ←→/Tab moves the selection, Enter picks it, Esc cancels. Opens with `selected: 'no'`. */
interface QuitConfirm {
  activeCount: number;
  selected: 'yes' | 'no';
}

/** Global search (`/`) while open: live-filtered as `query` changes, full session content fetched lazily on first use. */
interface Search {
  query: string;
  results: DeckSession[];
  index: number;
  /** `undefined` until first needed; keyed by session id. */
  textBySessionId?: Map<string, string>;
  loading: boolean;
}

const PIN_CYCLE: readonly (SessionPin | undefined)[] = [undefined, 'top', 'bottom'];

/**
 * Claude sessions run in background PTYs owned by this process, each mirrored into a headless xterm
 * so the preview pane can redraw any of them instantly. Enter attaches full-screen (raw passthrough),
 * Ctrl+Q detaches with the session still running, Ctrl+K Q detaches and stops it outright (same as `x`
 * from the list). Ctrl+K T swaps attached and interacting for the same session directly, without
 * detaching to the list first.
 */
export class App {
  private sessions: DeckSession[] = [];
  private rows: TreeRow[] = [];
  /** Displayed project order per container (`''` = top level, else folder id), for K/J. */
  private containers = new Map<string, string[]>();
  private sessionContainers = new Map<string, string[]>();
  /** Index into `rows`: any row but a divider (or 0 when there are none). */
  private selected = 0;
  /** Checked with Space, for a batch action (archive/stop/delete/move) applied to all of them at once. */
  private multiSelected = new Set<DeckSession>();
  /** For ` (back to the previous session). */
  private lastSession?: DeckSession;
  private previousSession?: DeckSession;
  /** Lines scrolled back from the live bottom in the preview panel, for the session `current` points to — reset whenever the selection moves to a different session. See `scrollPreview`. */
  private previewScroll = 0;
  /** The session `previewScroll` currently applies to, so it can be reset the moment the selection moves elsewhere (including to no session at all). */
  private scrolledSession?: DeckSession;
  private tree: TreePrefs;
  private sidebarVisible = true;
  private attached: DeckSession | null = null;
  /** Set right after Ctrl+K while attached: the next key decides whether it's `n` (the new-session chord) or an ordinary Ctrl+K meant for the agent. */
  private chordPending = false;
  /** Typing goes straight to this session's PTY, but (unlike `attached`) the list and preview keep rendering normally. `Ctrl+Q` stops it, `Ctrl+K Q` stops it and kills the session, `Ctrl+K T` attaches full-screen instead. */
  private interacting: DeckSession | null = null;
  /** Off by default (and always while attached, see `attach()`) so a click-drag still does the terminal's own text selection. `m` (or `Ctrl+K M` while typing in place) flips it — see `toggleMouseTracking()`. */
  private mouseTracking = false;
  private message = '';
  private messageTimer?: NodeJS.Timeout;
  private renderTimer?: NodeJS.Timeout;
  private prompt: Prompt | null = null;
  private picker: Picker | null = null;
  private confirm: Confirm | null = null;
  private quitConfirm: QuitConfirm | null = null;
  private helpScroll: number | null = null;
  private configSelected: number | null = null;
  /**
   * `w`: local skills (`~/.claude/skills`) and subagents (`~/.claude/agents`) read the moment the
   * popup opens; `skillsPopupTab` (← →) picks which of the two the overlay shows.
   */
  private skillsScroll: number | null = null;
  private skillsPopupTab: SkillsPopupTab = 'skills';
  private skills: LocalSkill[] = [];
  private agents: LocalAgent[] = [];
  private search: Search | null = null;
  private statusFilter = new Set<StatusCategory>();
  private timeFilter: TimeFilter = 'all';
  /** Set from a tag row at the bottom of the list (Enter): narrows the tree to sessions carrying that tag. */
  private tagFilter?: string;
  /** Shown next to the SESSIONS title for a moment after resizing. */
  private resizeNote?: { text: string; until: number };
  private readonly procs = new StatusTracker();
  /** Tails the events log of Copilot sessions running here, writing their status files. */
  private readonly copilotWatcher = new CopilotStatusWatcher(() => undefined);
  /** `git status` per session cwd, for the row's ⇡/⇣/✱ markers and the preview panel's branch line. Off entirely when `ui.gitStatus` is false. */
  private readonly gitStatus = new GitStatusTracker();
  /** `^`: show archived sessions (only) instead of the active ones. */
  private archivedView = false;
  /** Ids moved to the trash this run, newest last, for Ctrl+Z. */
  private deleted: string[] = [];
  private readonly store = new DeckStore();
  /** Settings from `~/.session-deck/config.json` — edited in place by the config popup (`C`), or by hand (restart to pick up a hand-made change). */
  private config = readDeckConfig();
  private sidebarPct: number;
  private themePreference: ThemePreference;
  private systemTheme: ThemeName = 'dark';
  /** Once the terminal has answered an OSC 11 query, its background decides "system" (not the OS setting). */
  private terminalReportsBackground = false;
  private readonly themes: Record<ThemeName, Theme> = { dark: new Theme('dark'), light: new Theme('light') };
  /** Toasts for sessions that need you, except the one you're attached to. The extension notifies too; claims keep it to one toast. */
  /** `notifyChanges()` always passes `config.ui.notifyStatuses`, so this constructor doesn't set a default. */
  private readonly notifier = new WaitingNotifier({
    iconPath: path.join(__dirname, '..', 'resources', 'icon.png'),
    skip: (id) => this.attached?.id === id || this.interacting?.id === id,
  });
  /** Last terminal title written, so it's only rewritten when the waiting count changes. */
  private lastTitle = '';

  constructor() {
    const ui = this.store.getUi();
    this.sidebarPct = ui.sidebarPct ?? DEFAULT_SIDEBAR_PCT;
    this.themePreference = ui.theme ?? 'system';
    this.tree = this.store.getTree();
  }

  async run(): Promise<void> {
    // Turns Claude's live process status into Session Deck's status files ("done" after a turn, etc.),
    // the same way the extension does. Both can run at once: the watcher skips writes already made.
    new ClaudeProcessWatcher(() => undefined).start();
    purgeTrash(Date.now(), this.config.trash.retentionDays * DAY_MS);
    warmExecutables(); // pays the first spawn's `where.exe` lookup now, not on whichever session gets attached first
    await this.discover();
    this.pollProcs();
    setInterval(() => this.pollProcs(), 1000);
    // Names/archive flags changed by the VS Code extension (or by us).
    this.store.watch(() => void this.discover().then(() => this.scheduleRender()));
    void this.refreshSystemTheme();
    setInterval(() => void this.refreshSystemTheme(), THEME_POLL_MS);
    void this.pollGitStatus();
    setInterval(() => void this.pollGitStatus(), GIT_STATUS_POLL_MS);

    out.write(`${ESC}?1049h${ESC}?25l${ESC}2J`);
    process.stdin.setRawMode(true);
    process.stdin.setEncoding('utf8');
    process.stdin.on('data', (data: string) => this.onInput(data));
    out.on('resize', () => {
      if (this.attached) {
        resizeLive(this.attached.live, out.columns, out.rows);
      } else {
        out.write(`${ESC}2J`);
        this.resizeAllToPane();
        this.render();
      }
    });
    process.on('uncaughtException', (err) => {
      out.write(`${RESET_AGENT_MODES}${ESC}?25h${ESC}?1049l`);
      console.error(err);
      process.exit(1);
    });
    this.render();
  }

  // -------------------------------------------------------------------------------------------
  // Theme
  // -------------------------------------------------------------------------------------------

  private get theme(): Theme {
    return this.themes[this.themePreference === 'system' ? this.systemTheme : this.themePreference];
  }

  /** Asks the terminal for its background (answered via stdin, see {@link onInput}); falls back to the OS setting. */
  private async refreshSystemTheme(): Promise<void> {
    if (this.themePreference !== 'system' || this.attached) {
      return;
    }
    out.write(OSC11_QUERY);
    if (!this.terminalReportsBackground) {
      const osTheme = await readOsTheme();
      if (osTheme && !this.terminalReportsBackground) {
        this.setSystemTheme(osTheme);
      }
    }
  }

  private setSystemTheme(theme: ThemeName): void {
    if (theme !== this.systemTheme) {
      this.systemTheme = theme;
      this.scheduleRender();
    }
  }

  // -------------------------------------------------------------------------------------------
  // Sessions
  // -------------------------------------------------------------------------------------------

  private get selectedRow(): TreeRow | undefined {
    return this.rows[this.selected];
  }

  private get current(): DeckSession | undefined {
    const row = this.selectedRow;
    return row?.kind === 'session' ? row.session : undefined;
  }

  /** The project the selection belongs to: the project row itself, or a session's project. */
  private get selectedProject(): { key: string; root: string } | undefined {
    const row = this.selectedRow;
    if (row?.kind === 'project') {
      return { key: row.projectKey, root: row.root };
    }
    return row?.kind === 'session' ? { key: row.session.projectKey, root: row.session.projectRoot } : undefined;
  }

  /** Checked sessions, in `this.sessions` order — stale entries (deleted/vanished elsewhere) are dropped by `rebuildRows`. */
  private get multiSelection(): DeckSession[] {
    return this.sessions.filter((s) => this.multiSelected.has(s));
  }

  /** Space: check/uncheck the selected row for a batch action, then move down like a typical multi-select list. */
  private toggleMultiSelect(s: DeckSession): void {
    if (!this.multiSelected.delete(s)) {
      this.multiSelected.add(s);
    }
    this.move(1);
  }

  /** `A` with a checked batch: archives (or, in the archived view, unarchives) every one of them. */
  private bulkArchive(targets: DeckSession[]): void {
    const archive = !this.archivedView;
    const toUpdate: DeckSession[] = [];
    let skipped = 0;
    for (const s of targets) {
      if (!s.id) {
        skipped++;
        continue;
      }
      if (archive && s.live && !s.live.exited) {
        const status = this.procs.statusOf(s);
        if (status === 'running' || status === 'waiting') {
          skipped++;
          continue;
        }
        this.kill(s);
      }
      toUpdate.push(s);
    }
    this.multiSelected.clear();
    if (toUpdate.length === 0) {
      this.flash(skipped ? `Nothing archived — ${skipped} still working` : 'Nothing to archive');
      return;
    }
    void Promise.all(toUpdate.map((s) => this.store.updateSession(s.id!, { archived: archive }))).then(() => {
      this.rebuildRows();
      this.scheduleRender();
    });
    this.flash(`${archive ? 'Archived' : 'Unarchived'} ${toUpdate.length}${skipped ? ` · ${skipped} skipped (still working)` : ''}`);
  }

  /** `x` with a checked batch: stops every one of them that's running. */
  private bulkStop(targets: DeckSession[]): void {
    let stopped = 0;
    for (const s of targets) {
      if (s.live) {
        this.kill(s);
        stopped++;
      }
    }
    this.multiSelected.clear();
    this.flash(stopped ? `Stopped ${stopped}${stopped < targets.length ? ` · ${targets.length - stopped} not running` : ''}` : 'None of the selected sessions were running');
  }

  /** `d` with a checked batch: moves every deletable one (a stopped Claude session on disk) to the trash. Copilot sessions and running ones are skipped. */
  private bulkDelete(targets: DeckSession[]): void {
    let deleted = 0;
    let skipped = 0;
    for (const s of targets) {
      if (s.agent === 'copilot' || (s.live && !s.live.exited) || this.procs.isElsewhere(s) || !s.id || !s.file || !fs.existsSync(s.file)) {
        skipped++;
        continue;
      }
      try {
        trashClaudeSession(s.file, s.id, displayTitle(s, this.store));
      } catch {
        skipped++;
        continue;
      }
      clearSessionStatus(s.id);
      this.deleted.push(s.id);
      if (s.live) {
        disposeLive(s.live);
      }
      this.sessions = this.sessions.filter((x) => x !== s);
      deleted++;
    }
    this.multiSelected.clear();
    this.rebuildRows();
    this.flash(deleted ? `Moved ${deleted} to the trash${skipped ? ` · ${skipped} skipped` : ''} · Ctrl+Z to undo` : `Nothing deletable in the selection${skipped ? ` (${skipped} skipped)` : ''}`);
  }

  private async discover(): Promise<void> {
    this.sessions = await discoverSessions(this.sessions);
    this.tree = this.store.getTree();
    this.rebuildRows();
  }

  private isVisible(s: DeckSession): boolean {
    return (
      this.isArchived(s) === this.archivedView &&
      matchesStatusFilter(this.procs.categoryOf(s), this.statusFilter) &&
      withinTimeFilter(s.mtime, this.timeFilter) &&
      (!this.tagFilter || this.tagsOf(s).includes(this.tagFilter))
    );
  }

  private tagsOf(s: DeckSession): string[] {
    return (s.id && this.store.getSession(s.id)?.tags) || [];
  }

  /** Every session, minus ones belonging to a project removed with `d` (see `DeckStore.setProjectHidden`). */
  private get unhiddenSessions(): DeckSession[] {
    const hidden = this.store.getHiddenProjects();
    return hidden.length ? this.sessions.filter((s) => !hidden.includes(s.projectKey)) : this.sessions;
  }

  private isArchived(s: DeckSession): boolean {
    return !!s.id && this.store.getSession(s.id)?.archived === true;
  }

  private get filtering(): boolean {
    return this.statusFilter.size > 0 || this.timeFilter !== 'all' || !!this.tagFilter;
  }

  /** Options for `buildTree`, shared by the live render and `cycleActiveSession`'s fully-expanded lookup. */
  private treeBuildOptions(): TreeOptions {
    return {
      include: (s) => this.isVisible(s),
      categoryOf: (s) => this.procs.categoryOf(s),
      isDoneUnseen: (s) => this.procs.statusOf(s) === 'done',
      pinOf: (s) => (s.id ? this.store.getSession(s.id)?.pin : undefined),
      gitOf: (s) => (this.config.ui.gitStatus ? this.gitStatus.get(s.cwd) : undefined),
      filtering: this.filtering,
      recentProjectsFirst: this.config.ui.recentProjectsFirst,
      recentSessionsFirst: this.config.ui.recentSessionsFirst,
      tagsOf: (s) => this.tagsOf(s),
    };
  }

  /** Re-applies the tree, sort and filters, keeping the selected row selected when it's still shown. */
  private rebuildRows(): void {
    const previous = this.selectedRow;
    const previousIndex = this.selected;
    const previousRows = this.rows;
    const built = buildTree(this.unhiddenSessions, this.tree, this.treeBuildOptions());
    this.rows = built.rows;
    this.containers = built.containers;
    this.sessionContainers = built.sessionContainers;
    if (!this.config.ui.recentSessionsFirst) {
      this.applyFreezeSessionOrder(built.sessionContainers);
    }
    if (this.multiSelected.size) {
      for (const s of this.multiSelected) {
        if (!this.sessions.includes(s)) {
          this.multiSelected.delete(s);
        }
      }
    }
    const keep = previous ? this.rows.findIndex((r) => sameRow(r, previous)) : -1;
    this.selected = keep >= 0 ? keep : (this.nearestSurvivingRow(previousRows, previousIndex) ?? Math.max(0, this.rows.findIndex((r) => r.kind === 'session')));
  }

  private applyFreezeSessionOrder(sessionContainers: Map<string, string[]>): void {
    const next = freezeSessionOrder(this.tree, sessionContainers);
    if (next !== this.tree) {
      this.tree = next;
      void this.store.updateTree((tree) => freezeSessionOrder(tree, sessionContainers));
    }
  }

  /** See `prependSession`: puts a just-created session at the top of its project, when `recentSessionsFirst` is off. */
  private applyPrependSession(projectKey: string, sessionId: string): void {
    this.tree = prependSession(this.tree, projectKey, sessionId);
    void this.store.updateTree((tree) => prependSession(tree, projectKey, sessionId));
  }

  /** See `renameSessionId`: keeps a session's manual position when e.g. `/clear` gives it a new id. */
  private applySessionIdChange(projectKey: string, oldId: string, newId: string): void {
    const change = (tree: TreePrefs) => {
      const renamed = renameSessionId(tree, projectKey, oldId, newId);
      return renamed !== tree ? renamed : prependSession(tree, projectKey, newId);
    };
    this.tree = change(this.tree);
    void this.store.updateTree(change);
  }

  /** When the previously selected row is gone (deleted, archived, filtered out), select whatever now sits
   *  where it used to be — the row right after it, else the row right before it — instead of the list's top. */
  private nearestSurvivingRow(previousRows: TreeRow[], previousIndex: number): number | undefined {
    for (let i = previousIndex + 1; i < previousRows.length; i++) {
      const idx = this.rows.findIndex((r) => sameRow(r, previousRows[i]));
      if (idx >= 0) return idx;
    }
    for (let i = previousIndex - 1; i >= 0; i--) {
      const idx = this.rows.findIndex((r) => sameRow(r, previousRows[i]));
      if (idx >= 0) return idx;
    }
    return undefined;
  }

  /** Applies a tree change right away, and to the shared store (on the tree as it is on disk). */
  private changeTree(change: (tree: TreePrefs) => TreePrefs): void {
    this.tree = change(this.tree);
    this.rebuildRows();
    void this.store.updateTree(change);
  }

  private selectWhere(match: (row: TreeRow) => boolean): boolean {
    const i = this.rows.findIndex(match);
    if (i >= 0) {
      this.selected = i;
    }
    return i >= 0;
  }

  /** Refreshes every visible session's `git status`, then repaints — a no-op (no subprocess spawned) while `ui.gitStatus` is off. */
  private async pollGitStatus(): Promise<void> {
    if (!this.config.ui.gitStatus) {
      return;
    }
    await this.gitStatus.poll(this.unhiddenSessions.map((s) => s.cwd));
    this.rebuildRows(); // the project row's badge is computed into `this.rows`, not read live like the preview panel is
    this.scheduleRender();
  }

  private pollProcs(): void {
    this.procs.poll();
    // A brand-new session (or one that ran /clear) only learns its id from Claude's own pid file.
    for (const s of this.sessions) {
      if (s.live && !s.live.exited) {
        const rec = this.procs.forPid(s.live.pid);
        if (rec && rec.sessionId !== s.id) {
          const oldId = s.id;
          s.id = rec.sessionId;
          s.file = undefined;
          if (s.pendingTopOrder) {
            s.pendingTopOrder = false;
            if (!this.config.ui.recentSessionsFirst) {
              this.applyPrependSession(s.projectKey, s.id);
            }
          } else if (oldId) {
            // /clear: same terminal, new session id, and its old title no longer describes the
            // (now empty) conversation — show it as fresh and waiting for rename, same as a brand-new one.
            s.title = s.agent === 'copilot' ? '(new Copilot session)' : '(new session)';
            if (!this.config.ui.recentSessionsFirst) {
              // Keep its manual position instead of falling out of `sessionOrder` and landing at
              // the back of `sortSessionsManual`'s fallback.
              this.applySessionIdChange(s.projectKey, oldId, s.id);
            }
          }
        }
        refreshLiveTitle(s, () => this.scheduleRender());
        this.sendPendingPrompt(s);
      } else if (s.file && this.procs.isElsewhere(s)) {
        // Another terminal keeps writing to it: keep its "5m ago" current.
        try {
          s.mtime = fs.statSync(s.file).mtimeMs;
        } catch {
          // vanished; keep the last known time
        }
      }
    }
    // Statuses (and so filter matches) and relative times move on their own.
    this.rebuildRows();
    this.notifyChanges();
    this.scheduleRender();
  }

  private layout(): Layout {
    return computeLayout(out.columns || 120, out.rows || 30, this.sidebarPct, this.sidebarVisible);
  }

  /** PTY size for background agents: the preview's body. In list-only layout there's no preview, so agents keep their size. */
  private ptySize(): { cols: number; rows: number } | undefined {
    return ptySizeFor(this.layout());
  }

  private start(s: DeckSession): boolean {
    if (!fs.existsSync(s.cwd)) {
      this.flash(`Folder no longer exists: ${s.cwd}`);
      return false;
    }
    const { cols, rows } = this.ptySize() ?? { cols: 80, rows: 24 };
    const sessionId = s.id;
    s.live = spawnAgent(s.agent, sessionId, !!s.isNew, s.cwd, cols, rows, {
      onData: (data) => {
        if (this.attached === s) {
          out.write(data);
        } else if (this.current === s) {
          this.scheduleRender();
        }
      },
      isAttached: () => this.attached === s,
      onExit: (exitCode) => {
        if (s.agent === 'copilot' && sessionId) {
          this.copilotWatcher.stop(sessionId);
        }
        if (this.attached === s) {
          this.detach(`Session exited (code ${exitCode})`);
        } else if (this.interacting === s) {
          this.interacting = null;
          this.flash(`Session exited (code ${exitCode})`);
        } else {
          this.scheduleRender();
        }
      },
    });
    if (s.agent === 'copilot' && sessionId) {
      this.copilotWatcher.start(sessionId);
    }
    s.isNew = false; // from now on it resumes its own id
    return true;
  }

  private kill(s: DeckSession): void {
    if (s.live) {
      disposeLive(s.live);
    }
    if (s.agent === 'copilot' && s.id) {
      this.copilotWatcher.stop(s.id);
    }
    s.live = undefined;
    // Stays listed only if it's resumable: found on disk, or (Claude) its transcript exists by now. A new
    // session with nothing sent has neither.
    if (!s.id || !(s.onDisk || (s.file && fs.existsSync(s.file)))) {
      this.sessions = this.sessions.filter((x) => x !== s);
    }
    this.rebuildRows();
  }

  private resizeAllToPane(): void {
    const size = this.ptySize();
    if (!size) {
      return;
    }
    for (const s of this.sessions) {
      if (s !== this.attached) {
        resizeLive(s.live, size.cols, size.rows);
      }
    }
  }

  private shortPath(p: string): string {
    const home = os.homedir();
    return p.toLowerCase().startsWith(home.toLowerCase()) ? `~${p.slice(home.length)}` : p;
  }

  private viewOf(s: DeckSession): SessionView {
    const status = this.procs.statusOf(s);
    const active = s.live && !s.live.exited && (status === 'running' || status === 'waiting');
    return {
      title: displayTitle(s, this.store),
      status,
      elsewhere: this.procs.isElsewhere(s),
      agent: s.agent,
      timeLabel: active ? 'now' : humanizeSince(s.mtime),
      cwd: this.shortPath(s.cwd),
      id: s.id,
      detail: s.live && !s.live.exited ? s.live.screenError : undefined,
      git: this.config.ui.gitStatus ? this.gitStatus.get(s.cwd) : undefined,
    };
  }

  /** Attaching (and detaching) counts as seeing the session: its "done"/"error" stops asking for attention, in both front ends. */
  private markSeen(s: DeckSession): void {
    if (s.id) {
      void acknowledgeSessionStatus(s.id).then(() => this.pollProcs());
    }
  }

  private notifyChanges(): void {
    if (!this.config.ui.notifications) {
      return;
    }
    const watchedSessions = this.sessions.filter((s) => s.id && ((s.live && !s.live.exited) || this.procs.isElsewhere(s)));
    const projectNames = projectLabels(watchedSessions.map((s) => s.projectRoot));
    const watched = watchedSessions.map((s) => ({
      sessionId: s.id!,
      label: displayTitle(s, this.store),
      project: projectNames.get(s.projectRoot),
    }));
    // Clicking the toast selects that session here (the terminal can't be brought to the front from Node).
    this.notifier.check(
      watched,
      (id) => {
        this.selectWhere((r) => r.kind === 'session' && r.session.id === id);
        this.scheduleRender();
      },
      this.config.ui.notifyStatuses
    );
  }

  /** "Session Deck · ◐ 2 need you" in the terminal's title bar/tab, so it's visible from other windows. */
  private updateTitle(waiting: number): void {
    const title = waiting ? `Session Deck · ◐ ${waiting} need${waiting === 1 ? 's' : ''} you` : 'Session Deck';
    if (title !== this.lastTitle) {
      this.lastTitle = title;
      out.write(`\x1b]0;${title}\x07`);
    }
  }

  // -------------------------------------------------------------------------------------------
  // Attach / detach
  // -------------------------------------------------------------------------------------------

  /** Shared by attach() and startInteracting(): refuses a session open elsewhere, starts a stopped one. `undefined` if it couldn't be readied (already flashed why). */
  private ensureLive(s: DeckSession): LiveSession | undefined {
    if (this.procs.isElsewhere(s)) {
      this.flash('That session is running in another terminal. Close it there first.');
      return undefined;
    }
    if (!s.live || s.live.exited) {
      if (s.live) {
        this.kill(s);
      }
      if (!this.start(s)) {
        return undefined;
      }
    }
    return s.live!;
  }

  private attach(s: DeckSession): void {
    const live = this.ensureLive(s);
    if (!live) {
      return;
    }
    this.attached = s;
    this.markSeen(s);
    // Off for as long as input goes straight to the agent — it never asked for it, and a wheel notch
    // (or the agent's own mouse handling) would otherwise write raw SGR bytes into its screen.
    out.write(DISABLE_MOUSE);
    // Paint the mirrored screen immediately (no waiting for the agent), then let the resize-triggered
    // redraw and live output stream straight through.
    const snapshot = live.serializer.serialize({ scrollback: 0 });
    out.write(`${ESC}?2026h${ESC}0m${ESC}2J${ESC}H${ESC}?25h${snapshot}${ESC}?2026l`);
    resizeLive(live, out.columns || 120, out.rows || 30);
  }

  /** Leaves full-screen attached mode: real terminal modes reset, its own title dropped, `attached` cleared. Shared by `detach()` and `switchToInteracting()` so the two can't drift apart. */
  private teardownAttached(): void {
    this.attached = null;
    // RESET_AGENT_MODES already turns mouse reporting back off (among other things the agent may have
    // changed) — restore it to whatever `mouseTracking` was before attaching, if it was on.
    out.write(`${RESET_AGENT_MODES}${ESC}?25l${this.mouseTracking ? ENABLE_MOUSE : ''}`);
    this.lastTitle = ''; // the agent set its own title while attached
  }

  private detach(note?: string): void {
    const seen = this.attached;
    this.teardownAttached();
    if (seen) {
      this.markSeen(seen); // whatever finished while you were attached, you saw
    }
    this.resizeAllToPane();
    if (note) {
      this.flash(note);
    }
    this.render();
  }

  /** Ctrl+K Q: leaves attached/interacting mode via `leave`, then stops `s` outright — same as `x` from the list. */
  private stopSession(s: DeckSession, leave: () => void): void {
    leave();
    this.kill(s);
    this.flash('Session stopped');
  }

  /** Ctrl+K T while attached: back to typing into `s` from the list/preview, without detaching to the list first. */
  private switchToInteracting(s: DeckSession): void {
    // Confirmed live first: attached must stay put (not go null with nothing to replace it) if `s`
    // turns out to be unreachable (e.g. grabbed by another terminal in the same instant).
    if (!this.ensureLive(s)) {
      return;
    }
    this.teardownAttached();
    this.resizeAllToPane();
    this.startInteracting(s);
  }

  /** Ctrl+K T while interacting: attach full-screen to `s`, without stopping back to the list first. */
  private switchToAttach(s: DeckSession): void {
    if (!this.ensureLive(s)) {
      return;
    }
    this.interacting = null;
    this.attach(s);
  }

  // Only the very next key resolves the chord — findPlainKey searches its whole input, which would
  // otherwise fire on an 'n'/'t' typed anywhere later in the same chunk (e.g. a fast-typed sentence
  // or paste), silently eating a Ctrl+K meant for the agent and the text along with it.
  private isNewSessionChord(data: string): boolean {
    return findPlainKey(data, 'n') === 0 || findPlainKey(data, 'N') === 0;
  }

  private isSwitchChord(data: string): boolean {
    return findPlainKey(data, 't') === 0 || findPlainKey(data, 'T') === 0;
  }

  private isMouseChord(data: string): boolean {
    return findPlainKey(data, 'm') === 0 || findPlainKey(data, 'M') === 0;
  }

  private isStopSessionChord(data: string): boolean {
    return findPlainKey(data, 'q') === 0 || findPlainKey(data, 'Q') === 0;
  }

  /**
   * Ctrl+K's resolving key, if `text` starts with one — shared by both places `onLiveInput` checks
   * for it (the chord's own chunk, and the chunk after, if it arrived split across two). Returns
   * whether it resolved (and already acted); adding a further resolving key only means editing here.
   * `toggleMouse` is only passed while interacting (see `onKey`) — while attached, mouse tracking is
   * always off regardless (see `attach()`), so `m`/`M` there goes to the agent like any other key.
   */
  private resolveChord(text: string, stop: () => void, switchMode: () => void, stopSession: () => void, toggleMouse?: () => void): boolean {
    if (this.isNewSessionChord(text)) {
      stop();
      this.newSession('claude');
      return true;
    }
    if (this.isSwitchChord(text)) {
      switchMode();
      return true;
    }
    if (this.isStopSessionChord(text)) {
      stopSession();
      return true;
    }
    if (toggleMouse && this.isMouseChord(text)) {
      toggleMouse();
      return true;
    }
    return false;
  }

  /**
   * Input while attached (full-screen) or interacting (typed into from the list) with `live`:
   * forwarded to its PTY, except Ctrl+Q (`stop` — detach or stop interacting, see `findDetachKey`) and
   * the Ctrl+K chord, which starts a new session in the same project on `n`/`N`, swaps attached and
   * interacting for the same session on `t`/`T` (`switchMode`), stops the session outright on `q`/`Q`
   * (`stopSession` — same as `x` from the list; there's no separate Ctrl+Alt+Q, since Windows Terminal
   * doesn't reliably report that combination — some layouts compose it into a printable character
   * instead of a Ctrl+Alt-modified Q), or (while interacting only) flips mouse tracking on `m`/`M`
   * (`toggleMouse`). The chord's resolving key is usually typed just after Ctrl+K, quickly enough to
   * arrive in the same input chunk (unlike Ctrl+Q, which needs no second key) — so both
   * `findChordKey`'s match end and the *next* chunk, if this one ends right at the match, are checked
   * for it.
   */
  private onLiveInput(
    live: LiveSession | undefined,
    data: string,
    stop: () => void,
    switchMode: () => void,
    stopSession: () => void,
    toggleMouse?: () => void
  ): void {
    if (this.chordPending) {
      this.chordPending = false;
      if (this.resolveChord(data, stop, switchMode, stopSession, toggleMouse)) {
        return;
      }
      if (live && !live.exited) {
        live.pty.write('\x0b'); // not the chord after all: the withheld Ctrl+K goes to the agent
      }
    }
    const chord = findChordKey(data);
    const detachIdx = findDetachKey(data);
    const idx = chord && (detachIdx < 0 || chord.index < detachIdx) ? chord.index : detachIdx;
    if (idx < 0) {
      if (live && !live.exited) {
        live.pty.write(data);
      }
      return;
    }
    // Forward keys typed before the match; drop the rest (e.g. the matching key-up events).
    if (idx > 0 && live && !live.exited) {
      live.pty.write(data.slice(0, idx));
    }
    if (chord && idx === chord.index) {
      const after = data.slice(chord.end);
      if (this.resolveChord(after, stop, switchMode, stopSession, toggleMouse)) {
        return;
      }
      this.chordPending = true;
    } else {
      stop();
    }
  }

  /**
   * Types straight into `s` without leaving the list/preview screen — its own PTY is already sized to
   * the preview pane (background sessions always are), so nothing needs resizing. `Ctrl+Q` stops
   * interacting, `Ctrl+K Q` stops interacting and kills the session, `Ctrl+K T` attaches full-screen
   * instead.
   */
  private startInteracting(s: DeckSession): void {
    const live = this.ensureLive(s);
    if (!live) {
      return;
    }
    this.interacting = s;
    this.markSeen(s);
    // Unlike attach(), mouse tracking is left as `mouseTracking` currently has it (not forced off): the
    // list/preview screen is still up (typed input is the only thing that goes to the agent), so a
    // wheel notch is still ours to scroll it with when it's on — see onKey's `interacting` branch.
    this.render();
  }

  private stopInteracting(): void {
    this.interacting = null;
    this.render();
  }

  // -------------------------------------------------------------------------------------------
  // Rename
  // -------------------------------------------------------------------------------------------

  /**
   * Sets the same title Claude's own `/rename` does. A running Claude keeps its title in memory and
   * re-writes it into the transcript, so a live session must be renamed through Claude itself. A
   * stopped one gets the records appended that `/rename` would have written. Any Session Deck name
   * override is cleared, so the new title shows in the extension too.
   */
  private renameSession(s: DeckSession, rawName: string): void {
    const name = oneLine(rawName).slice(0, 100);
    if (!name || name === displayTitle(s, this.store)) {
      return;
    }
    if (s.agent === 'copilot') {
      // Copilot has no /rename: a Session Deck name, shared with the extension.
      if (s.id) {
        void this.store.updateSession(s.id, { name }).then(() => this.scheduleRender());
        this.flash(`Renamed to "${name}"`);
      }
      return;
    }
    if (this.procs.isElsewhere(s)) {
      this.flash('That session is running in another terminal. Rename it there with /rename.');
      return;
    }
    const live = s.live;
    if (live && !live.exited) {
      const status = this.procs.statusOf(s);
      if (status !== 'idle' && status !== 'done') {
        this.flash('Session is busy or waiting for you. Rename it once it is idle (○).');
        return;
      }
      typeLine(live, `/rename ${name}`);
    } else if (s.id && s.file) {
      appendClaudeRenameRecords(s.file, s.id, name);
    } else {
      this.flash('Nothing to rename yet.');
      return;
    }
    s.title = name;
    if (s.id && this.store.getSession(s.id)?.name) {
      void this.store.updateSession(s.id, { name: undefined });
    }
    this.flash(`Renamed to "${name}"`);
  }

  // -------------------------------------------------------------------------------------------
  // Rendering
  // -------------------------------------------------------------------------------------------

  private scheduleRender(): void {
    if (this.attached || this.renderTimer) {
      return;
    }
    this.renderTimer = setTimeout(() => {
      this.renderTimer = undefined;
      this.render();
    }, 33);
  }

  private flash(text: string): void {
    this.message = text;
    clearTimeout(this.messageTimer);
    this.messageTimer = setTimeout(() => {
      this.message = '';
      this.scheduleRender();
    }, 4000);
    this.scheduleRender();
  }

  private render(): void {
    if (this.attached) {
      return;
    }
    const current = this.current;
    if (current && current !== this.lastSession) {
      this.previousSession = this.lastSession;
      this.lastSession = current;
    }
    if (current !== this.scrolledSession) {
      this.scrolledSession = current;
      this.previewScroll = 0;
    }
    const t = this.theme;
    const cols = out.columns || 120;
    const height = out.rows || 30;
    const layout = this.layout();

    const counts = Object.fromEntries(STATUS_CATEGORIES.map((c) => [c, 0])) as Record<StatusCategory, number>;
    const unhidden = this.unhiddenSessions;
    const inView = unhidden.filter((s) => this.isArchived(s) === this.archivedView);
    let doneCount = 0;
    for (const s of inView) {
      // A finished-but-unseen session is its own bucket here, not folded into "waiting" the way
      // categoryOf does for sort/filter purposes — these counts must reflect real status.
      if (this.procs.statusOf(s) === 'done') {
        doneCount++;
      } else {
        counts[this.procs.categoryOf(s)]++;
      }
    }
    const liveCount = unhidden.filter((s) => s.live && !s.live.exited).length;
    this.updateTitle(counts.waiting);

    let frame = `${ESC}?2026h${ESC}0m`;
    // Truncated as a safety net: a wrapped top row would push the whole frame down.
    frame += `${ESC}1;1H${fitAnsi(renderHeader(t, cols, counts, doneCount, liveCount, themeLabel(this.themePreference, this.theme.name), VERSION), cols)}`;
    frame += `${ESC}2;1H${fitAnsi(renderPills(t, cols, inView.length, counts, doneCount, this.statusFilter, this.timeFilter, this.tagFilter), cols)}`;

    if (layout.list) {
      const listRows: ListRow[] = this.rows.map((r) =>
        r.kind === 'session'
          ? { kind: 'session', view: this.viewOf(r.session), isLast: r.isLast, depth: r.depth, pin: r.pin, checked: this.multiSelected.has(r.session) }
          : r.kind === 'tag'
            ? { ...r, active: this.tagFilter === r.name }
            : r
      );
      const modes = [
        this.multiSelected.size ? `${this.multiSelected.size} selected` : '',
        this.archivedView ? 'archived' : '',
        this.filtering ? 'filtered' : '',
        this.tree.sort === 'actionable' ? 'actionable' : '',
        this.tree.view === 'active' ? 'active on top' : '',
      ]
        .filter(Boolean)
        .join(' · ');
      const note = this.resizeNote && Date.now() < this.resizeNote.until ? this.resizeNote.text : modes ? `· ${modes}` : '';
      const empty = unhidden.length === 0 ? 'No Claude sessions found.' : 'Nothing matches the filter. Press 0 to clear it.';
      frame += placeLines(layout.list, renderListPanel(t, layout.list, listRows, this.selected, note, empty));
    }
    const group = this.groupPreview();
    if (layout.preview && group) {
      frame += placeLines(layout.preview, renderGroupPreviewPanel(t, layout.preview, group));
    } else if (layout.preview) {
      const s = this.current;
      const content = s
        ? {
            view: this.viewOf(s),
            term: s.live && !s.live.exited ? s.live.term : undefined,
            interacting: this.interacting === s,
            exitCode: s.live?.exitCode,
            lastResponse: this.lastResponseOf(s),
            scrollOffset: this.previewScroll,
          }
        : undefined;
      frame += placeLines(layout.preview, renderPreviewPanel(t, layout.preview, content));
    }
    if (layout.dividerX !== undefined && layout.list) {
      for (let y = 0; y < layout.list.height; y++) {
        frame += `${ESC}${layout.list.y + y + 1};${layout.dividerX + 1}H${t.fg('border')}│${ESC}0m`;
      }
    }

    frame += `${ESC}${height};1H`;
    if (this.prompt) {
      frame += renderPromptBar(t, cols, this.prompt.label, this.prompt.value);
    } else if (this.confirm) {
      frame += renderConfirmBar(t, cols, this.confirm.question);
    } else if (this.message) {
      frame += renderMessageBar(t, cols, this.message);
    } else if (this.interacting) {
      frame += renderMessageBar(t, cols, `Typing into session · Ctrl+Q to stop · Ctrl+K T to attach`);
    } else {
      frame += renderHelpBar(t, cols);
    }

    if (this.helpScroll !== null) {
      const overlay = helpOverlay(t, cols, height, this.helpScroll, VERSION);
      this.helpScroll = Math.min(this.helpScroll, overlay.maxScroll);
      overlay.lines.forEach((line, i) => (frame += `${ESC}${overlay.y + i + 1};${overlay.x + 1}H${line}`));
    }
    if (this.picker) {
      const overlay = pickerOverlay(t, cols, height, this.picker.title, this.picker.items, this.picker.index);
      overlay.lines.forEach((line, i) => (frame += `${ESC}${overlay.y + i + 1};${overlay.x + 1}H${line}`));
    }
    if (this.configSelected !== null) {
      const overlay = configOverlay(t, cols, height, this.config, getDeckConfigPath(), this.configSelected);
      overlay.lines.forEach((line, i) => (frame += `${ESC}${overlay.y + i + 1};${overlay.x + 1}H${line}`));
    }
    if (this.skillsScroll !== null) {
      const overlay = skillsOverlay(t, cols, height, this.skillsPopupTab, this.skills, this.agents, this.skillsScroll);
      this.skillsScroll = Math.min(this.skillsScroll, overlay.maxScroll);
      overlay.lines.forEach((line, i) => (frame += `${ESC}${overlay.y + i + 1};${overlay.x + 1}H${line}`));
    }
    if (this.search) {
      const labels = projectLabels(unhidden.map((s) => s.projectRoot));
      const rows: SearchResultRow[] = this.search.results.map((s) => ({ view: this.viewOf(s), projectLabel: labels.get(s.projectRoot) ?? s.projectRoot }));
      const overlay = searchOverlay(t, cols, height, this.search.query, this.search.loading, rows, this.search.index);
      overlay.lines.forEach((line, i) => (frame += `${ESC}${overlay.y + i + 1};${overlay.x + 1}H${line}`));
    }
    if (this.quitConfirm) {
      const overlay = quitConfirmOverlay(t, cols, height, this.quitConfirm.activeCount, this.quitConfirm.selected);
      overlay.lines.forEach((line, i) => (frame += `${ESC}${overlay.y + i + 1};${overlay.x + 1}H${line}`));
    }
    frame += `${ESC}?2026l`;
    out.write(frame);
  }

  /** Starts loading a stopped session's last reply the first time it's previewed. */
  private lastResponseOf(s: DeckSession): string | null | undefined {
    if (s.lastResponse === undefined && s.agent === 'copilot' && s.id) {
      s.lastResponse = lastCopilotAssistantResponse(s.id) ?? '';
    } else if (s.lastResponse === undefined && s.file) {
      s.lastResponse = null; // loading
      void readLastAssistantResponse(s.file).then((r) => {
        s.lastResponse = r || '';
        this.scheduleRender();
      });
    }
    return s.lastResponse;
  }

  // -------------------------------------------------------------------------------------------
  // Input
  // -------------------------------------------------------------------------------------------

  private onInput(data: string): void {
    if (this.attached) {
      this.onKey(data);
      return;
    }
    // Answers to our background-color query arrive mixed into the input.
    const { theme, rest } = extractBackgroundReply(data);
    if (theme) {
      this.terminalReportsBackground = true;
      this.setSystemTheme(theme);
    }
    const keys = splitKeys(rest);
    for (let i = 0; i < keys.length; i++) {
      if (this.attached) {
        // A key (Enter) just attached a session: the rest of the chunk is typed into it as-is.
        this.onKey(keys.slice(i).join(''));
        return;
      }
      this.onKey(keys[i]);
    }
  }

  private move(delta: number): void {
    let i = this.selected;
    do {
      i += delta;
    } while (this.rows[i] && this.rows[i].kind === 'divider');
    if (this.rows[i]) {
      this.selected = i;
    }
  }

  /** How far back the preview can scroll for the selected session: everything its live PTY's mirror still has in scrollback, 0 with nothing live to scroll into. */
  private previewMaxScroll(): number {
    const live = this.current?.live;
    return live && !live.exited ? live.term.buffer.active.baseY : 0;
  }

  /** `delta` lines further back into scrollback (positive) or toward the live bottom (negative), clamped to what's there. Caller renders. */
  private scrollPreview(delta: number): void {
    this.previewScroll = Math.min(this.previewMaxScroll(), Math.max(0, this.previewScroll + delta));
  }

  /**
   * ↑/↓ (`delta` -1/+1, `move`'s own convention): scrolls the selected session's preview instead of
   * moving the tree selection, whenever it has something live to scroll into. Falls back to `move`
   * otherwise (no session selected, or nothing live yet to scroll), so plain tree navigation is
   * untouched. A mouse wheel notch is `onMouseSequence`'s job, not this one — see `ENABLE_MOUSE`.
   */
  private onArrow(delta: number): void {
    if (this.current && this.previewMaxScroll() > 0) {
      this.scrollPreview(-delta);
    } else {
      this.move(delta);
    }
  }

  /**
   * A wheel notch or click, reported because {@link ENABLE_MOUSE} is on — unlike `onArrow`, it never
   * falls back to moving the tree selection, since scrolling the wheel is never navigation (that
   * ambiguity is exactly why a real arrow-key press and a terminal's wheel-to-arrow-key translation
   * used to be indistinguishable). A plain click is parsed the same way and simply consumed; sdeck has
   * no use for clicks yet.
   *
   * A notch scrolls real terminal scrollback if there is any — but many agents (Claude Code among
   * them) never produce any: they keep their own transcript internally and redraw in place in
   * response to PageUp/PageDown, rather than ever letting old lines scroll off screen. For those,
   * `previewMaxScroll()` always reads 0 no matter how long the conversation, so a notch instead
   * forwards the same PageUp/PageDown a keypress would send straight to the live PTY, exactly as
   * typing one would (either attached, or from the list — see `onLiveInput`) — the agent pages its own
   * history the same way either way.
   */
  private onMouseSequence(data: string): boolean {
    const mouse = parseMouseSequence(data);
    if (!mouse) {
      return false;
    }
    if (mouse.wheel && this.current) {
      if (this.previewMaxScroll() > 0) {
        this.scrollPreview(mouse.wheel === 'up' ? 1 : -1);
      } else {
        const live = this.current.live;
        if (live && !live.exited) {
          live.pty.write(mouse.wheel === 'up' ? '\x1b[5~' : '\x1b[6~');
        }
      }
    }
    return true;
  }

  /** PageUp/PageDown/Home/End: a full preview page, or all the way to the top/bottom of scrollback. No-ops (rather than falling back to `move`) when there's nothing to scroll, since these keys have no tree-navigation meaning of their own. */
  private onPageKey(data: string): boolean {
    if (!this.current || this.previewMaxScroll() <= 0) {
      return false;
    }
    const page = Math.max(1, this.ptySize()?.rows ?? 10);
    switch (data) {
      case '\x1b[5~':
        this.scrollPreview(page);
        return true;
      case '\x1b[6~':
        this.scrollPreview(-page);
        return true;
      case '\x1b[H':
      case '\x1b[1~':
        this.scrollPreview(this.previewMaxScroll());
        return true;
      case '\x1b[F':
      case '\x1b[4~':
        this.scrollPreview(-this.previewMaxScroll());
        return true;
      default:
        return false;
    }
  }

  /** Summary shown in the preview while a folder or project row is selected. */
  private groupPreview(): GroupPreview | undefined {
    const row = this.selectedRow;
    if (row?.kind !== 'folder' && row?.kind !== 'project') {
      return undefined;
    }
    const keys = row.kind === 'project' ? [row.projectKey] : (this.containers.get(row.folderId) ?? []);
    // From the sessions themselves, not the rows: a collapsed group has no session rows.
    const sessions = this.sessions
      .filter((s) => keys.includes(s.projectKey) && this.isVisible(s))
      .sort((x, y) => y.mtime - x.mtime)
      .map((s) => this.viewOf(s));
    const detail = row.kind === 'project' ? this.shortPath(row.root) : `${keys.length} project${keys.length === 1 ? '' : 's'}`;
    const { count, running, waiting } = row;
    return { kind: row.kind, name: row.kind === 'project' ? row.label : row.name, detail, counts: { count, running, waiting }, sessions };
  }

  private onPromptKey(prompt: Prompt, data: string): void {
    if (data === '\r') {
      this.prompt = null;
      prompt.onSubmit(prompt.value);
    } else if (data === '\x1b' || data === '\x03') {
      this.prompt = null;
    } else if (data === '\x7f' || data === '\b') {
      prompt.value = Array.from(prompt.value).slice(0, -1).join('');
    } else if (data === '\x15') {
      prompt.value = ''; // Ctrl+U
    } else if (!data.startsWith('\x1b')) {
      // eslint-disable-next-line no-control-regex -- strips control characters from typed/pasted text
      prompt.value += data.replace(/[\x00-\x1f\x7f]/g, '');
    }
    this.render();
  }

  private onHelpKey(data: string): void {
    if (data === '\x1b' || data === '?' || data === 'q') {
      this.helpScroll = null;
      out.write(`${ESC}2J`);
    } else if (data === '\x1b[A' || data === 'k') {
      this.helpScroll = Math.max(0, (this.helpScroll ?? 0) - 1);
    } else if (data === '\x1b[B' || data === 'j') {
      this.helpScroll = (this.helpScroll ?? 0) + 1;
    }
    this.render();
  }

  private onConfigKey(data: string): void {
    if (data === '\x1b[A' || data === 'k') {
      this.configSelected = Math.max(0, (this.configSelected ?? 0) - 1);
    } else if (data === '\x1b[B' || data === 'j') {
      this.configSelected = Math.min(CONFIG_FIELDS.length - 1, (this.configSelected ?? 0) + 1);
    } else if (data === '\r') {
      this.editConfigField(this.configSelected ?? 0);
    } else if (data === '\x1b' || data === 'C' || data === 'q') {
      this.configSelected = null;
      out.write(`${ESC}2J`);
    }
    this.render();
  }

  private onSkillsKey(data: string): void {
    if (data === '\x1b' || data === 'w' || data === 'q') {
      this.skillsScroll = null;
      out.write(`${ESC}2J`);
    } else if (data === '\x1b[A' || data === 'k') {
      this.skillsScroll = Math.max(0, (this.skillsScroll ?? 0) - 1);
    } else if (data === '\x1b[B' || data === 'j') {
      this.skillsScroll = (this.skillsScroll ?? 0) + 1;
    } else if (data === '\x1b[C' || data === '\x1b[D') {
      this.skillsPopupTab = this.skillsPopupTab === 'skills' ? 'agents' : 'skills';
      this.skillsScroll = 0;
    }
    this.render();
  }

  /** `toggle` fields flip themselves right away; the rest open a prompt pre-filled with their current value. */
  private editConfigField(index: number): void {
    const field = CONFIG_FIELDS[index];
    if (field.kind === 'toggle') {
      this.applyConfigField(index, '');
      return;
    }
    this.prompt = { label: field.label, value: field.editValue(this.config), onSubmit: (value) => this.applyConfigField(index, value) };
  }

  private applyConfigField(index: number, input: string): void {
    const field = CONFIG_FIELDS[index];
    const next = field.apply(this.config, input);
    if (!next) {
      this.flash(`Invalid value for ${field.label}`);
      return;
    }
    this.config = next;
    writeDeckConfig(next);
    clearExecutableCache(); // a tools.*.command edit shouldn't need a restart to take effect
    warmExecutables(); // re-resolve now, not on whichever session gets attached first
    this.rebuildRows(); // e.g. ui.recentProjectsFirst reorders the tree right away
    void this.pollGitStatus(); // turning ui.gitStatus on shows markers right away, not after the next poll
    this.flash(`${field.label} updated`);
  }

  private openSearch(): void {
    this.search = { query: '', results: [], index: 0, loading: false };
  }

  /** Every prompt/reply of every session with a real id, keyed by session id — read once per search, lazily. */
  private async loadSearchText(): Promise<Map<string, string>> {
    const searchable = this.unhiddenSessions.filter((s) => s.id);
    const pairs = await mapWithConcurrency(searchable, SEARCH_READ_CONCURRENCY, async (s): Promise<[string, string]> => [
      s.id!,
      s.agent === 'claude' ? (s.file ? await readSessionSearchText(s.file) : '') : copilotSessionSearchText(s.id!),
    ]);
    return new Map(pairs);
  }

  /** Fetches full session content on first use (once), then re-filters synchronously on every keystroke after that. */
  private onSearchInput(): void {
    const search = this.search;
    if (!search) {
      return;
    }
    if (search.textBySessionId) {
      this.applySearchFilter();
      return;
    }
    if (search.loading) {
      return; // already fetching; applySearchFilter runs once it resolves, against the latest query
    }
    search.loading = true;
    void this.loadSearchText().then((text) => {
      if (this.search !== search) {
        return; // search was closed (or reopened) while this was loading
      }
      search.textBySessionId = text;
      search.loading = false;
      this.applySearchFilter();
      this.render();
    });
  }

  /** A leading `!`/`@`/`#`/`&`/`~` filters by status, same as the main filter pills (see `FILTER_KEYS`). */
  private applySearchFilter(): void {
    const search = this.search;
    if (!search) {
      return;
    }
    const trimmed = search.query.trim();
    if (!trimmed) {
      search.results = [];
      search.index = 0;
      return;
    }
    const statusFilter = FILTER_KEYS[trimmed[0]];
    const query = (statusFilter ? trimmed.slice(1) : trimmed).trim().toLowerCase();
    const textBySessionId = search.textBySessionId;
    search.results = textBySessionId
      ? this.unhiddenSessions
          .filter((s) => s.id && (!statusFilter || this.procs.categoryOf(s) === statusFilter))
          .map((s) => ({ session: s, match: fuzzyMatch(query, textBySessionId.get(s.id!) ?? '') }))
          .filter((x) => x.match.matched)
          .sort((a, b) => a.match.score - b.match.score)
          .map((x) => x.session)
      : [];
    search.index = Math.min(search.index, Math.max(0, search.results.length - 1));
  }

  private onSearchKey(data: string): void {
    const search = this.search;
    if (!search) {
      return;
    }
    if (data === '\x1b' || data === '\x03') {
      this.search = null;
      out.write(`${ESC}2J`);
    } else if (data === '\r') {
      const target = search.results[search.index];
      this.search = null;
      out.write(`${ESC}2J`);
      if (target) {
        this.jumpToSession(target);
      }
    } else if (data === '\x1b[A') {
      search.index = Math.max(0, search.index - 1);
    } else if (data === '\x1b[B') {
      search.index = Math.min(Math.max(0, search.results.length - 1), search.index + 1);
    } else if (data === '\x7f' || data === '\b') {
      search.query = Array.from(search.query).slice(0, -1).join('');
      this.onSearchInput();
    } else if (data === '\x15') {
      search.query = ''; // Ctrl+U
      this.onSearchInput();
    } else if (!data.startsWith('\x1b')) {
      // eslint-disable-next-line no-control-regex -- strips control characters from typed/pasted text
      search.query += data.replace(/[\x00-\x1f\x7f]/g, '');
      this.onSearchInput();
    }
    this.render();
  }

  private resizeSidebar(delta: number): void {
    this.sidebarPct = Math.min(SIDEBAR_PCT_MAX, Math.max(SIDEBAR_PCT_MIN, this.sidebarPct + delta));
    this.resizeNote = { text: `${this.sidebarPct}%`, until: Date.now() + 1500 };
    setTimeout(() => this.scheduleRender(), 1600);
    void this.store.updateUi({ sidebarPct: this.sidebarPct });
    out.write(`${ESC}2J`);
    this.resizeAllToPane();
  }

  private cycleTheme(): void {
    this.themePreference = THEME_CYCLE[(THEME_CYCLE.indexOf(this.themePreference) + 1) % THEME_CYCLE.length];
    void this.store.updateUi({ theme: this.themePreference });
    void this.refreshSystemTheme();
    this.flash(`Theme: ${this.themePreference}`);
  }

  /** `m` from the list, or `Ctrl+K M` while typing in place: flips real mouse reporting on/off (never while attached — see `attach()`). On lets the wheel/click scroll the preview; off (the default) leaves click-drag to the terminal's own text selection. */
  private toggleMouseTracking(): void {
    this.mouseTracking = !this.mouseTracking;
    out.write(this.mouseTracking ? ENABLE_MOUSE : DISABLE_MOUSE);
    this.flash(this.mouseTracking ? 'Mouse scrolling on — click-drag no longer selects text (m to turn off)' : 'Mouse scrolling off — click-drag selects text again (m to turn on)');
    this.render();
  }

  private onKey(data: string): void {
    if (this.prompt) {
      this.onPromptKey(this.prompt, data);
      return;
    }
    if (this.helpScroll !== null) {
      this.onHelpKey(data);
      return;
    }
    if (this.configSelected !== null) {
      this.onConfigKey(data);
      return;
    }
    if (this.skillsScroll !== null) {
      this.onSkillsKey(data);
      return;
    }
    if (this.search) {
      this.onSearchKey(data);
      return;
    }
    if (this.picker) {
      this.onPickerKey(this.picker, data);
      return;
    }
    if (this.confirm) {
      const { onYes } = this.confirm;
      this.confirm = null;
      if (data === 'y' || data === 'Y') {
        onYes();
      }
      this.render();
      return;
    }
    if (this.quitConfirm) {
      this.onQuitConfirmKey(data);
      return;
    }
    const attached = this.attached;
    if (attached) {
      this.onLiveInput(
        attached.live,
        data,
        () => this.detach(),
        () => this.switchToInteracting(attached),
        () => this.stopSession(attached, () => this.detach())
      );
      return;
    }
    const interacting = this.interacting;
    if (interacting) {
      // When mouseTracking is on, a wheel notch scrolls the same preview shown while just browsing,
      // rather than reaching the agent, which would otherwise see it as a plain ↑/↓ (with no mouse mode
      // on, that's all a terminal ever sends for it) and likely treat it as its own prompt-history
      // recall instead of what the user actually did.
      if (this.onMouseSequence(data)) {
        this.render();
        return;
      }
      this.onLiveInput(
        interacting.live,
        data,
        () => this.stopInteracting(),
        () => this.switchToAttach(interacting),
        () => this.stopSession(interacting, () => this.stopInteracting()),
        () => this.toggleMouseTracking()
      );
      return;
    }

    const s = this.current;
    const row = this.selectedRow;
    if (FILTER_KEYS[data]) {
      const category = FILTER_KEYS[data];
      if (!this.statusFilter.delete(category)) {
        this.statusFilter.add(category);
      }
      this.rebuildRows();
      this.render();
      return;
    }
    if (/^[1-9]$/.test(data)) {
      const n = Number(data);
      this.selectWhere((r) => (r.kind === 'folder' || r.kind === 'project') && r.hotkey === n);
      this.render();
      return;
    }
    if (this.onMouseSequence(data)) {
      this.render();
      return;
    }
    if (this.onPageKey(data)) {
      this.render();
      return;
    }
    switch (data) {
      case '\x1b[A':
      case '\x1bOA': // some terminals (and their wheel-to-arrow-key translation) send SS3, not CSI, for ↑
        this.onArrow(-1);
        break;
      case 'k':
        this.move(-1);
        break;
      case '\x1b[B':
      case '\x1bOB': // ↓, same as above
        this.onArrow(1);
        break;
      case 'j':
        this.move(1);
        break;
      case '\x1b[D': // ←
      case 'h':
        this.collapseOrParent();
        break;
      case '\x1b[C': // →
      case 'l':
        this.expandOrChild();
        break;
      case '\t':
        if (row?.kind === 'folder' || row?.kind === 'project') {
          this.toggleCollapsed(row);
        }
        break;
      case '\r':
        if (s) {
          this.attach(s);
          return;
        }
        if (row?.kind === 'folder' || row?.kind === 'project') {
          this.toggleCollapsed(row);
        } else if (row?.kind === 'tag') {
          this.tagFilter = this.tagFilter === row.name ? undefined : row.name;
          this.rebuildRows();
        }
        break;
      case ' ':
        if (row?.kind === 'session') {
          this.toggleMultiSelect(row.session);
        }
        break;
      case '\x1b':
        if (this.multiSelected.size) {
          this.multiSelected.clear();
          this.flash('Selection cleared');
        } else {
          return;
        }
        break;
      case '`':
        this.selectPreviousSession();
        break;
      case ']':
        this.cycleActiveSession(1);
        break;
      case '[':
        this.cycleActiveSession(-1);
        break;
      case 's':
        if (s && this.procs.isElsewhere(s)) {
          this.flash('That session is running in another terminal.');
        } else if (s && (!s.live || s.live.exited)) {
          if (s.live) this.kill(s);
          if (this.start(s)) this.flash('Started in background');
        }
        break;
      case 'i':
        if (s) {
          this.startInteracting(s);
        } else {
          this.flash('Select a session to type into.');
        }
        break;
      case 'n':
        this.newSession('claude');
        return;
      case 'N':
        this.newSession('copilot');
        return;
      case 'p':
        this.openAddProject();
        break;
      case 'o':
        this.openPromptInput();
        break;
      case 'c':
        this.copyLastResponse();
        break;
      case 'R':
        this.restart();
        break;
      case 'A':
        this.toggleArchived();
        break;
      case '^':
        this.archivedView = !this.archivedView;
        this.rebuildRows();
        this.flash(this.archivedView ? 'Archived sessions (A to unarchive, ^ to go back)' : 'Active sessions');
        break;
      case '\x1a': // Ctrl+Z
        this.undoDelete();
        break;
      case 'Z':
        this.openTrashPicker();
        break;
      case 'e':
      case '\x1bOQ': // F2
      case '\x1b[12~': // F2 (some terminals)
        this.rename();
        break;
      case '\x0c': // Ctrl+L
        this.clearContext();
        break;
      case 'x':
        if (this.multiSelected.size) {
          this.bulkStop(this.multiSelection);
        } else if (s && s.live) {
          this.kill(s);
          this.flash('Session stopped');
        }
        break;
      case 'u':
        if (s?.id) {
          void markSessionUnseen(s.id).then(() => this.pollProcs());
          this.flash('Marked as unread');
        } else {
          this.flash('Select a session to mark as unread.');
        }
        break;
      case 'U':
        if (s?.id) {
          void acknowledgeSessionStatus(s.id).then(() => this.pollProcs());
          this.flash('Marked as read');
        } else {
          this.flash('Select a session to mark as read.');
        }
        break;
      case ',':
        this.cyclePin();
        break;
      case 'g':
        this.prompt = { label: 'New folder', value: '', onSubmit: (name) => this.newFolder(name) };
        break;
      case 'M':
        this.openMovePicker();
        break;
      case 'L':
        this.openTagPrompt();
        break;
      case 'K':
      case '\x1b[1;2A': // Shift+↑
        this.reorder(-1);
        break;
      case 'J':
      case '\x1b[1;2B': // Shift+↓
        this.reorder(1);
        break;
      case 'd':
        if (this.multiSelected.size) {
          this.bulkDelete(this.multiSelection);
        } else if (s) {
          this.deleteSession(s);
        } else if (row?.kind === 'folder') {
          const folderId = row.folderId;
          this.confirm = {
            question: `Delete folder "${row.name}"? Its projects move back to the top level.`,
            onYes: () => this.changeTree((t) => deleteFolder(t, folderId)),
          };
        } else if (row?.kind === 'project') {
          this.removeProject(row.projectKey, row.label);
        } else if (row?.kind === 'tag') {
          const name = row.name;
          this.confirm = { question: `Remove tag "${name}" from every project?`, onYes: () => this.removeTagEverywhere(name) };
        }
        break;
      case 'S':
        this.changeTree((t) => ({ ...t, sort: t.sort === 'recent' ? 'actionable' : 'recent' }));
        this.flash(this.tree.sort === 'actionable' ? 'Sessions: needs attention first (error, waiting, running, idle)' : 'Sessions: most recent first');
        break;
      case 't':
        this.changeTree((t) => ({ ...t, view: t.view === 'normal' ? 'active' : 'normal' }));
        this.flash(this.tree.view === 'active' ? 'View: groups with running or waiting sessions on top' : 'View: normal');
        break;
      case '*':
        this.timeFilter = nextTimeFilter(this.timeFilter);
        this.rebuildRows();
        break;
      case '0':
        this.statusFilter.clear();
        this.timeFilter = 'all';
        this.tagFilter = undefined;
        this.rebuildRows();
        break;
      case '<':
        this.resizeSidebar(-SIDEBAR_STEP);
        break;
      case '>':
        this.resizeSidebar(SIDEBAR_STEP);
        break;
      case 'b':
      case '\x02': // Ctrl+B
        this.sidebarVisible = !this.sidebarVisible;
        out.write(`${ESC}2J`);
        this.resizeAllToPane();
        break;
      case 'T':
        this.cycleTheme();
        break;
      case 'm':
        this.toggleMouseTracking();
        break;
      case '?':
        this.helpScroll = 0;
        break;
      case 'C':
        this.configSelected = 0;
        break;
      case 'w':
        this.skills = discoverLocalSkills();
        this.agents = discoverLocalAgents();
        this.skillsPopupTab = 'skills';
        this.skillsScroll = 0;
        break;
      case '/':
        this.openSearch();
        break;
      case 'r':
        clearSessionMetaCache();
        void this.discover().then(() => this.render());
        break;
      case 'q':
      case '\x03':
        this.quit();
        return;
      default:
        return;
    }
    this.render();
  }

  // -------------------------------------------------------------------------------------------
  // Tree actions
  // -------------------------------------------------------------------------------------------

  private toggleCollapsed(row: Extract<TreeRow, { kind: 'folder' | 'project' }>): void {
    const key = row.kind === 'folder' ? folderNodeKey(row.folderId) : projectNodeKey(row.projectKey);
    this.changeTree((t) => setCollapsed(t, key, !row.collapsed));
  }

  /** ←: collapse an expanded group, otherwise go up to the parent row (session → project → folder). */
  private collapseOrParent(): void {
    const row = this.selectedRow;
    if ((row?.kind === 'folder' || row?.kind === 'project') && !row.collapsed) {
      this.toggleCollapsed(row);
      return;
    }
    const depth = row?.kind === 'session' ? row.depth : row?.kind === 'project' ? row.depth : 0;
    for (let i = this.selected - 1; i >= 0 && depth > 0; i--) {
      const r = this.rows[i];
      const d = r.kind === 'project' ? r.depth : r.kind === 'folder' ? 0 : Infinity;
      if (d < depth) {
        this.selected = i;
        return;
      }
    }
  }

  /** →: expand a collapsed group, otherwise step into its first child. */
  private expandOrChild(): void {
    const row = this.selectedRow;
    if (row?.kind !== 'folder' && row?.kind !== 'project') {
      return;
    }
    if (row.collapsed) {
      this.toggleCollapsed(row);
    } else if (this.rows[this.selected + 1] && this.rows[this.selected + 1].kind !== 'divider') {
      this.selected++;
    }
  }

  /** `: back to the session selected before the current one, expanding its groups if they're collapsed. */
  private selectPreviousSession(): void {
    const target = this.previousSession;
    if (!target || !this.sessions.includes(target)) {
      this.flash('No previous session.');
      return;
    }
    this.jumpToSession(target);
  }

  /**
   * `]` / `[`: selects the next / previous started session — running, waiting (which already folds in
   * "finished, not seen yet") or idle — wrapping around and skipping only stopped or erroring ones,
   * plus folders and projects. So switching between the sessions you're actually using, spread across
   * different projects, takes one press instead of walking every row between them.
   * With `ui.expandCollapsedOnActiveJump` on (the default), a target hidden inside a collapsed
   * folder or project is still reached, expanding just that group (via `jumpToSession`); off, only
   * rows already shown are targets, same as `move`.
   */
  private cycleActiveSession(delta: 1 | -1): void {
    const rows = this.config.ui.expandCollapsedOnActiveJump ? buildTree(this.unhiddenSessions, { ...this.tree, collapsed: [] }, this.treeBuildOptions()).rows : this.rows;
    const active: { i: number; session: DeckSession }[] = [];
    rows.forEach((r, i) => {
      if (r.kind === 'session' && isStarted(this.procs.categoryOf(r.session))) {
        active.push({ i, session: r.session });
      }
    });
    if (!active.length) {
      this.flash('No started sessions.');
      return;
    }
    const current = this.selectedRow;
    const at = current ? rows.findIndex((r) => sameRow(r, current)) : -1;
    const target = delta > 0 ? (active.find((a) => a.i > at) ?? active[0]) : ([...active].reverse().find((a) => a.i < at) ?? active[active.length - 1]);
    this.jumpToSession(target.session);
  }

  /** Selects `target` in the tree, expanding its project (and folder) if collapsed. Flashes if a filter still hides it. */
  private jumpToSession(target: DeckSession): void {
    const isTarget = (r: TreeRow) => r.kind === 'session' && r.session === target;
    if (this.selectWhere(isTarget)) {
      return;
    }
    const folder = this.tree.folders.find((f) => f.projects.includes(target.projectKey));
    this.changeTree((t) => {
      let next = setCollapsed(t, projectNodeKey(target.projectKey), false);
      if (folder) {
        next = setCollapsed(next, folderNodeKey(folder.id), false);
      }
      return next;
    });
    if (!this.selectWhere(isTarget)) {
      this.flash('Hidden by the filter. Press 0 to clear it.');
    }
  }

  private newSession(agent: AgentType): void {
    if (this.config.tools[agent].enabled === false) {
      this.flash(`${agent === 'claude' ? 'Claude' : 'Copilot'} is disabled (tools.${agent}.enabled: false in the config).`);
      this.render();
      return;
    }
    const s = this.current;
    const project = this.selectedProject;
    if (!project) {
      this.flash('Select a project or session to start a new session in.');
      this.render();
      return;
    }
    this.startNewSession(agent, s ? s.cwd : project.root, project);
  }

  /** `p`: a new Claude session in any folder, which lists its project once a prompt is sent. */
  private openAddProject(): void {
    this.prompt = { label: 'Project folder', value: '', onSubmit: (input) => void this.addProject(input) };
  }

  private async addProject(input: string): Promise<void> {
    const raw = input.trim().replace(/^"(.*)"$/, '$1');
    if (!raw) {
      return;
    }
    const cwd = path.resolve(expandHome(raw));
    if (!fs.existsSync(cwd) || !fs.statSync(cwd).isDirectory()) {
      this.flash(`Not a folder: ${cwd}`);
      this.render();
      return;
    }
    const agent: AgentType = this.config.tools.claude.enabled === false ? 'copilot' : 'claude';
    if (this.config.tools[agent].enabled === false) {
      this.flash('Claude and Copilot are both disabled in the config.');
      this.render();
      return;
    }
    const { root } = await resolveProjectRoot(cwd);
    const key = normalizeFsPath(root);
    await this.store.setProjectHidden(key, false); // explicitly added here: undoes a previous `d` removal, if any
    this.startNewSession(agent, cwd, { key, root });
  }

  private startNewSession(agent: AgentType, cwd: string, project: { key: string; root: string }): void {
    // Copilot takes a pre-assigned id; Claude reports its own once started.
    const fresh: DeckSession = {
      agent,
      id: agent === 'copilot' ? randomUUID() : null,
      isNew: agent === 'copilot',
      cwd,
      projectRoot: project.root,
      projectKey: project.key,
      title: agent === 'copilot' ? '(new Copilot session)' : '(new session)',
      mtime: Date.now(),
    };
    if (!this.config.ui.recentSessionsFirst) {
      // Renders at the top right away either way (see `sortSessionsManual`'s `pendingTopOrder` band).
      // Copilot already has its id, so its real position can be recorded now; Claude reports its own
      // id later, asynchronously — `pollProcs` records it then, once `s.id` stops being `null`.
      fresh.pendingTopOrder = true;
      if (fresh.id) {
        fresh.pendingTopOrder = false;
        this.applyPrependSession(project.key, fresh.id);
      }
    }
    this.sessions.unshift(fresh);
    this.rebuildRows();
    this.selectWhere((r) => r.kind === 'session' && r.session === fresh);
    if (this.config.ui.newSessionFullScreen) {
      this.attach(fresh);
    } else {
      this.startInteracting(fresh);
    }
  }

  /** `o`: a one-line prompt, sent without attaching. A stopped session is started first and gets it once ready. */
  private openPromptInput(): void {
    const s = this.current;
    if (!s) {
      this.flash('Select a session to send a prompt to.');
      return;
    }
    if (this.procs.isElsewhere(s)) {
      this.flash('That session is running in another terminal. Send it from there.');
      return;
    }
    if (this.procs.statusOf(s) === 'waiting') {
      this.flash('It is waiting for an answer. Attach (Enter) to reply.');
      return;
    }
    this.prompt = { label: `Prompt for ${displayTitle(s, this.store)}`, value: '', onSubmit: (text) => this.sendPrompt(s, text) };
  }

  private sendPrompt(s: DeckSession, rawText: string): void {
    const text = oneLine(rawText);
    if (!text) {
      return;
    }
    if (!s.live || s.live.exited) {
      if (s.live) {
        this.kill(s);
      }
      if (!this.start(s)) {
        return;
      }
    }
    s.pendingPrompt = text;
    this.sendPendingPrompt(s);
    this.flash(s.pendingPrompt ? 'Starting… the prompt is sent once it is ready' : 'Prompt sent');
  }

  /**
   * Types a queued prompt once the agent is ready for input: idle (or just done), and quiet for a
   * moment so a freshly started agent has drawn its input box. Never into a "waiting" prompt.
   */
  private sendPendingPrompt(s: DeckSession): void {
    const live = s.live;
    const status = this.procs.statusOf(s);
    if (!s.pendingPrompt || !live || live.exited || (status !== 'idle' && status !== 'done') || Date.now() - live.lastOutputAt < 1000) {
      return;
    }
    typeLine(live, s.pendingPrompt);
    s.pendingPrompt = undefined;
    this.markSeen(s);
  }

  /** `c`: copies the last response through the terminal (OSC 52), which works in Windows Terminal and over SSH. */
  private copyLastResponse(): void {
    const s = this.current;
    if (!s) {
      this.flash('Select a session to copy its last response.');
      return;
    }
    const copy = (text: string | undefined) => {
      if (!text) {
        this.flash('No response to copy yet.');
        return;
      }
      out.write(`\x1b]52;c;${Buffer.from(text, 'utf8').toString('base64')}\x07`);
      this.flash(`Copied the last response (${text.length} characters)`);
    };
    if (s.agent === 'copilot') {
      copy(s.id ? lastCopilotAssistantResponse(s.id) : undefined);
    } else if (s.file) {
      void readLastAssistantResponse(s.file).then(copy);
    } else {
      copy(undefined);
    }
  }

  /** `R`: a fresh agent process on the same conversation (e.g. to pick up changed settings or MCP servers). */
  private restart(): void {
    const s = this.current;
    if (!s || this.procs.isElsewhere(s)) {
      this.flash(s ? 'That session is running in another terminal.' : 'Select a session to restart.');
      return;
    }
    if (s.live) {
      disposeLive(s.live);
      if (s.agent === 'copilot' && s.id) {
        this.copilotWatcher.stop(s.id);
      }
      s.live = undefined;
    }
    if (s.agent === 'claude' && s.id && !(s.file && fs.existsSync(s.file))) {
      s.id = null; // nothing was sent yet: there's no conversation to resume, start a new one
    }
    if (this.start(s)) {
      this.flash('Restarted');
    }
  }

  /** `A`: archive (hide) or, in the archived view, unarchive. Shared with the extension. */
  private toggleArchived(): void {
    if (this.multiSelected.size) {
      this.bulkArchive(this.multiSelection);
      return;
    }
    const s = this.current;
    if (!s?.id) {
      this.flash('Select a session to archive.');
      return;
    }
    const archive = !this.isArchived(s);
    if (archive && s.live && !s.live.exited) {
      const status = this.procs.statusOf(s);
      if (status === 'running' || status === 'waiting') {
        this.flash('It is still working. Stop it (x) or wait until it is idle to archive it.');
        return;
      }
      this.kill(s);
    }
    void this.store.updateSession(s.id, { archived: archive }).then(() => {
      this.rebuildRows();
      this.scheduleRender();
    });
    this.flash(archive ? 'Archived (^ shows archived sessions)' : 'Unarchived');
  }

  /**
   * `d` on a project: removes it from the list. Nothing on disk is touched, and any still-running
   * session keeps running in the background — a new session for it (here or from outside Session Deck)
   * won't bring it back on its own; add the project again (`p`) to see it.
   */
  private removeProject(key: string, label: string): void {
    this.confirm = {
      question: `Remove project "${label}" from the list? (p to add it back)`,
      onYes: () => {
        void this.store.setProjectHidden(key, true).then(() => {
          this.rebuildRows();
          this.scheduleRender();
        });
      },
    };
  }

  /** `d` on a session: moves its transcript to the trash (Ctrl+Z or Z to bring it back). */
  private deleteSession(s: DeckSession): void {
    if (s.agent === 'copilot') {
      this.flash('Copilot sessions live in its own database: archive them (A) instead.');
      return;
    }
    if ((s.live && !s.live.exited) || this.procs.isElsewhere(s)) {
      this.flash('Stop the session before deleting it.');
      return;
    }
    if (!s.id || !s.file || !fs.existsSync(s.file)) {
      this.flash('Nothing on disk to delete.');
      return;
    }
    try {
      trashClaudeSession(s.file, s.id, displayTitle(s, this.store));
    } catch (err) {
      this.flash(`Could not move it to the trash: ${(err as Error).message}`);
      return;
    }
    clearSessionStatus(s.id);
    this.deleted.push(s.id);
    if (s.live) {
      disposeLive(s.live);
    }
    this.sessions = this.sessions.filter((x) => x !== s);
    this.rebuildRows();
    this.flash('Moved to the trash · Ctrl+Z to undo · Z to see the trash');
  }

  private undoDelete(): void {
    const id = this.deleted.pop();
    if (!id) {
      this.flash('Nothing to undo. Z shows the trash.');
      return;
    }
    this.restoreFromTrash(id);
  }

  private restoreFromTrash(sessionId: string): void {
    try {
      const entry = restoreSession(sessionId);
      this.deleted = this.deleted.filter((d) => d !== sessionId);
      clearSessionMetaCache();
      void this.discover().then(() => {
        this.selectWhere((r) => r.kind === 'session' && r.session.id === sessionId);
        this.scheduleRender();
      });
      this.flash(`Restored "${entry.title}"`);
    } catch (err) {
      this.flash((err as Error).message);
    }
  }

  /** `Z`: the trash, newest first; Enter restores. Items older than 30 days are removed on startup. */
  private openTrashPicker(): void {
    const entries = listTrash();
    if (entries.length === 0) {
      this.flash('The trash is empty.');
      return;
    }
    this.picker = {
      title: 'Trash · Enter restores',
      items: entries.map((e) => `${e.title} · ${humanizeSince(e.trashedAt)}`),
      index: 0,
      onPick: (i) => this.restoreFromTrash(entries[i].sessionId),
    };
  }

  private rename(): void {
    const row = this.selectedRow;
    const s = this.current;
    if (s) {
      const title = displayTitle(s, this.store);
      this.prompt = { label: 'Rename', value: title === '(new session)' ? '' : title, onSubmit: (value) => this.renameSession(s, value) };
    } else if (row?.kind === 'folder') {
      const folderId = row.folderId;
      this.prompt = {
        label: 'Rename folder',
        value: row.name,
        onSubmit: (value) => value.trim() && this.changeTree((t) => renameFolder(t, folderId, value)),
      };
    } else if (row?.kind === 'project') {
      this.flash('Projects are named after their folder on disk.');
    }
  }

  /** Ctrl+L: sends `/clear` into the live session, without attaching. Only when there's nothing to lose by it — idle, done (an unseen reply), or error (e.g. a stuck sign-in screen) — never mid-turn or mid-prompt. */
  private clearContext(): void {
    const s = this.current;
    if (!s) {
      this.flash('Select a session to clear.');
      return;
    }
    if (this.procs.isElsewhere(s)) {
      this.flash('That session is running in another terminal. Clear it there with /clear.');
      return;
    }
    const live = s.live;
    if (!live || live.exited) {
      this.flash('Select a running session to clear.');
      return;
    }
    const status = this.procs.statusOf(s);
    if (status !== 'idle' && status !== 'done' && status !== 'error') {
      this.flash('Session is busy or waiting for you. Clear once it is idle (○), done, or errored.');
      return;
    }
    typeLine(live, '/clear');
    if (this.config.ui.newSessionFullScreen) {
      this.attach(s);
    } else {
      this.startInteracting(s);
      this.flash('Cleared');
    }
  }

  private cyclePin(): void {
    const s = this.current;
    if (!s?.id) {
      this.flash(s ? 'Send something first: a new session has nothing to pin yet.' : 'Select a session to pin.');
      return;
    }
    const pin = PIN_CYCLE[(PIN_CYCLE.indexOf(this.store.getSession(s.id)?.pin) + 1) % PIN_CYCLE.length];
    void this.store.updateSession(s.id, { pin }).then(() => {
      this.rebuildRows();
      this.scheduleRender();
    });
    this.flash(pin ? `Pinned to the ${pin} of its project` : 'Unpinned');
  }

  /** Creates the folder, moving `projectKeys` into it when given. The id is made once, so the local and on-disk trees agree. */
  private newFolder(rawName: string, projectKeys?: string[]): void {
    const name = oneLine(rawName);
    if (!name) {
      return;
    }
    const { tree, folderId } = createFolder(this.tree, name);
    const folder = tree.folders.find((f) => f.id === folderId)!;
    this.changeTree((t) => {
      const withFolder = { ...t, folders: [...t.folders, folder] };
      return projectKeys?.length ? projectKeys.reduce((acc, key) => moveProjectToFolder(acc, key, folderId), withFolder) : withFolder;
    });
    if (!projectKeys?.length) {
      this.selectWhere((r) => r.kind === 'folder' && r.folderId === folderId);
    }
    this.flash(
      projectKeys?.length
        ? `Moved ${projectKeys.length > 1 ? `${projectKeys.length} projects` : 'project'} to "${name}"`
        : `Folder "${name}" created. Press M on a project to move it in.`
    );
  }

  private openMovePicker(): void {
    if (this.multiSelected.size) {
      this.openBulkMovePicker(this.multiSelection);
      return;
    }
    const project = this.selectedProject;
    if (!project) {
      this.flash('Select a project (or one of its sessions) to move it to a folder.');
      return;
    }
    const folders = this.tree.folders;
    const current = folders.findIndex((f) => f.projects.includes(project.key));
    const label = path.basename(project.root) || project.root;
    this.picker = {
      title: `Move ${label} to`,
      items: ['Top level (no folder)', ...folders.map((f) => f.name), '+ New folder…'],
      index: current + 1,
      onPick: (i) => {
        if (i === 0) {
          this.changeTree((t) => moveProjectToFolder(t, project.key, null));
        } else if (i <= folders.length) {
          const folderId = folders[i - 1].id;
          this.changeTree((t) => moveProjectToFolder(t, project.key, folderId));
        } else {
          this.prompt = { label: 'New folder', value: '', onSubmit: (name) => this.newFolder(name, [project.key]) };
        }
      },
    };
  }

  /** `M` with a checked batch: moves every distinct project among the selected sessions to one target folder. */
  private openBulkMovePicker(targets: DeckSession[]): void {
    const keys = [...new Set(targets.map((s) => s.projectKey))];
    const folders = this.tree.folders;
    const label = `${keys.length} project${keys.length === 1 ? '' : 's'}`;
    this.picker = {
      title: `Move ${label} to`,
      index: 0,
      items: ['Top level (no folder)', ...folders.map((f) => f.name), '+ New folder…'],
      onPick: (i) => {
        this.multiSelected.clear();
        if (i === 0) {
          this.changeTree((t) => keys.reduce((acc, key) => moveProjectToFolder(acc, key, null), t));
        } else if (i <= folders.length) {
          const folderId = folders[i - 1].id;
          this.changeTree((t) => keys.reduce((acc, key) => moveProjectToFolder(acc, key, folderId), t));
        } else {
          this.prompt = { label: 'New folder', value: '', onSubmit: (name) => this.newFolder(name, keys) };
        }
      },
    };
  }

  /** `L`: edit the selected session's own tags, or (with a checked batch) add tag(s) to every checked session. */
  private openTagPrompt(): void {
    if (this.multiSelected.size) {
      const targets = this.multiSelection.filter((s) => s.id);
      this.prompt = {
        label: `Add tag(s) to ${targets.length} session${targets.length === 1 ? '' : 's'}`,
        value: '',
        onSubmit: (value) => this.addTagsToSessions(targets, value),
      };
      return;
    }
    const s = this.current;
    if (!s?.id) {
      this.flash(s ? 'Send something first: a new session has nothing to tag yet.' : 'Select a session to tag.');
      return;
    }
    this.prompt = {
      label: `Tags for ${displayTitle(s, this.store)} (comma-separated)`,
      value: this.tagsOf(s).join(', '),
      onSubmit: (value) => this.setSessionTags(s.id!, value),
    };
  }

  private parseTagsInput(raw: string): string[] {
    return [...new Set(raw.split(',').map((t) => t.trim()).filter(Boolean))];
  }

  private setSessionTags(sessionId: string, raw: string): void {
    const tags = this.parseTagsInput(raw);
    void this.store.updateSession(sessionId, { tags }).then(() => {
      this.rebuildRows();
      this.scheduleRender();
    });
    this.flash(tags.length ? `Tags: ${tags.join(', ')}` : 'Tags cleared');
  }

  /** `L` with a checked batch: unions the entered tag(s) into every checked session's existing tags (never replaces them). */
  private addTagsToSessions(targets: DeckSession[], raw: string): void {
    const added = this.parseTagsInput(raw);
    this.multiSelected.clear();
    if (!added.length) {
      this.rebuildRows();
      this.flash('Selection cleared');
      return;
    }
    void Promise.all(
      targets.map((s) => this.store.updateSession(s.id!, { tags: [...new Set([...this.tagsOf(s), ...added])] }))
    ).then(() => {
      this.rebuildRows();
      this.scheduleRender();
    });
    this.flash(`Added "${added.join(', ')}" to ${targets.length} session${targets.length === 1 ? '' : 's'}`);
  }

  /** `d` on a tag row (bottom of the list): drops it from every session that carries it. */
  private removeTagEverywhere(name: string): void {
    const targets = this.sessions.filter((s) => this.tagsOf(s).includes(name));
    if (this.tagFilter === name) {
      this.tagFilter = undefined;
    }
    void Promise.all(targets.map((s) => this.store.updateSession(s.id!, { tags: this.tagsOf(s).filter((t) => t !== name) }))).then(() => {
      this.rebuildRows();
      this.scheduleRender();
    });
    this.flash(`Removed tag "${name}" from ${targets.length} session${targets.length === 1 ? '' : 's'}`);
  }

  /** K/J: moves the selected folder, the selected project, or the selected session (within its project), up or down. */
  private reorder(delta: number): void {
    const row = this.selectedRow;
    if (this.tree.view === 'active') {
      this.flash('Switch to the normal view (t) to reorder.');
      return;
    }
    if (row?.kind === 'folder') {
      const folderId = row.folderId;
      this.changeTree((t) => moveFolder(t, folderId, delta));
      return;
    }
    if (row?.kind === 'session') {
      const s = row.session;
      if (!s.id) {
        return;
      }
      const displayed = this.sessionContainers.get(s.projectKey) ?? [];
      this.changeTree((t) => moveSession(t, s.projectKey, s.id!, delta, displayed));
      return;
    }
    const project = this.selectedProject;
    if (!project) {
      return;
    }
    const container = this.tree.folders.find((f) => f.projects.includes(project.key))?.id ?? '';
    const displayed = this.containers.get(container) ?? [];
    this.changeTree((t) => moveProject(t, project.key, delta, displayed));
  }

  private onPickerKey(picker: Picker, data: string): void {
    if (data === '\x1b[A' || data === 'k') {
      picker.index = Math.max(0, picker.index - 1);
    } else if (data === '\x1b[B' || data === 'j') {
      picker.index = Math.min(picker.items.length - 1, picker.index + 1);
    } else if (data === '\r') {
      this.picker = null;
      out.write(`${ESC}2J`);
      picker.onPick(picker.index);
    } else if (data === '\x1b' || data === 'q' || data === '\x03') {
      this.picker = null;
      out.write(`${ESC}2J`);
    }
    this.render();
  }

  private quit(): void {
    const activeCount = this.sessions.filter((s) => {
      if (!s.live || s.live.exited) {
        return false;
      }
      const status = this.procs.statusOf(s);
      return status === 'running' || status === 'waiting';
    }).length;
    if (activeCount === 0) {
      this.quitNow();
      return;
    }
    this.quitConfirm = { activeCount, selected: 'no' };
    this.render();
  }

  private onQuitConfirmKey(data: string): void {
    const qc = this.quitConfirm;
    if (!qc) {
      return;
    }
    if (data === '\x1b[C' || data === '\x1b[D' || data === '\t') {
      qc.selected = qc.selected === 'yes' ? 'no' : 'yes';
    } else if ((data === '\r' && qc.selected === 'yes') || data === 'y' || data === 'Y') {
      this.quitConfirm = null;
      this.quitNow();
      return;
    } else if (data === '\r' || data === 'n' || data === 'N' || data === '\x1b' || data === 'q' || data === '\x03') {
      this.quitConfirm = null;
    }
    this.render();
  }

  private quitNow(): void {
    this.copilotWatcher.dispose();
    for (const s of this.sessions) {
      if (s.live && !s.live.exited) {
        try {
          s.live.pty.kill();
        } catch {
          // ignore
        }
      }
    }
    out.write(`${RESET_AGENT_MODES}${ESC}?25h${ESC}?1049l`);
    process.stdin.setRawMode(false);
    process.exit(0);
  }
}

/** Cursor-positioned lines filling `rect` (each line must already be exactly `rect.width` wide). */
function placeLines(rect: Rect, lines: string[]): string {
  return lines
    .slice(0, rect.height)
    .map((line, i) => `${ESC}${rect.y + i + 1};${rect.x + 1}H${line}`)
    .join('');
}

/** Same logical row across rebuilds (rows are recreated each time). */
function sameRow(a: TreeRow, b: TreeRow): boolean {
  if (a.kind === 'session' && b.kind === 'session') {
    return a.session === b.session;
  }
  if (a.kind === 'project' && b.kind === 'project') {
    return a.projectKey === b.projectKey;
  }
  if (a.kind === 'tag' && b.kind === 'tag') {
    return a.name === b.name;
  }
  return a.kind === 'folder' && b.kind === 'folder' && a.folderId === b.folderId;
}
