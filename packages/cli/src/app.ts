import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import {
  clearSessionMetaCache,
  DeckStore,
  humanizeSince,
  readLastAssistantResponse,
  SIDEBAR_PCT_MAX,
  SIDEBAR_PCT_MIN,
  ThemePreference,
} from '@session-deck/core';
import { ESC, fitAnsi, oneLine } from './ansi';
import { matchesStatusFilter, nextTimeFilter, STATUS_CATEGORIES, StatusCategory, TimeFilter, withinTimeFilter } from './filters';
import { findDetachKey, RESET_AGENT_MODES, splitKeys } from './keys';
import { computeLayout, Layout, ptySizeFor, Rect } from './layout';
import { disposeLive, resizeLive, spawnClaude } from './liveSession';
import { appendRenameRecords, buildRows, ClaudeProcs, DeckSession, discoverSessions, displayTitle, refreshLiveTitle, SidebarRow } from './sessions';
import { extractBackgroundReply, OSC11_QUERY, readOsTheme, Theme, ThemeName } from './theme';
import {
  helpOverlay,
  ListRow,
  renderHeader,
  renderHelpBar,
  renderListPanel,
  renderMessageBar,
  renderPills,
  renderPreviewPanel,
  renderPromptBar,
  SessionView,
  themeLabel,
} from './view';

const out = process.stdout;
const VERSION: string = JSON.parse(fs.readFileSync(path.join(__dirname, '..', 'package.json'), 'utf8')).version;
const DEFAULT_SIDEBAR_PCT = 35;
const SIDEBAR_STEP = 5;
const THEME_CYCLE: readonly ThemePreference[] = ['dark', 'light', 'system'];
const THEME_POLL_MS = 5000;
const FILTER_KEYS: Record<string, StatusCategory> = { '!': 'running', '@': 'waiting', '#': 'idle', '&': 'error', '~': 'stopped' };

/** Footer text input while open. */
interface Prompt {
  label: string;
  value: string;
  onSubmit(value: string): void;
}

/**
 * Claude sessions run in background PTYs owned by this process, each mirrored into a headless xterm
 * so the preview pane can redraw any of them instantly. Enter attaches full-screen (raw passthrough),
 * Ctrl+Q detaches with the session still running.
 */
export class App {
  private sessions: DeckSession[] = [];
  private rows: SidebarRow[] = [];
  /** Index into `rows`, always a session row (or 0 when there are none). */
  private selected = 0;
  private sidebarVisible = true;
  private attached: DeckSession | null = null;
  private message = '';
  private messageTimer?: NodeJS.Timeout;
  private renderTimer?: NodeJS.Timeout;
  private prompt: Prompt | null = null;
  private helpScroll: number | null = null;
  private statusFilter = new Set<StatusCategory>();
  private timeFilter: TimeFilter = 'all';
  /** Shown next to the SESSIONS title for a moment after resizing. */
  private resizeNote?: { text: string; until: number };
  private readonly procs = new ClaudeProcs();
  private readonly store = new DeckStore();
  private sidebarPct: number;
  private themePreference: ThemePreference;
  private systemTheme: ThemeName = 'dark';
  /** Once the terminal has answered an OSC 11 query, its background decides "system" (not the OS setting). */
  private terminalReportsBackground = false;
  private readonly themes: Record<ThemeName, Theme> = { dark: new Theme('dark'), light: new Theme('light') };

  constructor() {
    const ui = this.store.getUi();
    this.sidebarPct = ui.sidebarPct ?? DEFAULT_SIDEBAR_PCT;
    this.themePreference = ui.theme ?? 'system';
  }

  async run(): Promise<void> {
    await this.discover();
    this.pollProcs();
    setInterval(() => this.pollProcs(), 1000);
    // Names/archive flags changed by the VS Code extension (or by us).
    this.store.watch(() => void this.discover().then(() => this.scheduleRender()));
    void this.refreshSystemTheme();
    setInterval(() => void this.refreshSystemTheme(), THEME_POLL_MS);

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

  private get current(): DeckSession | undefined {
    const row = this.rows[this.selected];
    return row?.kind === 'session' ? row.session : undefined;
  }

  private async discover(): Promise<void> {
    this.sessions = await discoverSessions(this.sessions, this.store);
    this.rebuildRows();
  }

  /** Re-applies grouping and filters, keeping the selected session selected when it's still shown. */
  private rebuildRows(): void {
    const current = this.current;
    this.rows = buildRows(
      this.sessions,
      (s) => matchesStatusFilter(this.procs.categoryOf(s), this.statusFilter) && withinTimeFilter(s.mtime, this.timeFilter),
      (s) => this.procs.categoryOf(s)
    );
    const keep = this.rows.findIndex((r) => r.kind === 'session' && r.session === current);
    this.selected = keep >= 0 ? keep : Math.max(0, this.rows.findIndex((r) => r.kind === 'session'));
  }

  private pollProcs(): void {
    this.procs.poll();
    // A brand-new session (or one that ran /clear) only learns its id from Claude's own pid file.
    for (const s of this.sessions) {
      if (s.live && !s.live.exited) {
        const rec = this.procs.forPid(s.live.pid);
        if (rec && rec.sessionId !== s.id) {
          s.id = rec.sessionId;
          s.file = undefined;
        }
        refreshLiveTitle(s, () => this.scheduleRender());
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
    s.live = spawnClaude(s.id, s.cwd, cols, rows, {
      onData: (data) => {
        if (this.attached === s) {
          out.write(data);
        } else if (this.current === s) {
          this.scheduleRender();
        }
      },
      isAttached: () => this.attached === s,
      onExit: (exitCode) => {
        if (this.attached === s) {
          this.detach(`Session exited (code ${exitCode})`);
        } else {
          this.scheduleRender();
        }
      },
    });
    return true;
  }

  private kill(s: DeckSession): void {
    if (s.live) {
      disposeLive(s.live);
    }
    s.live = undefined;
    // Stays listed only if it's resumable, i.e. its transcript exists (a new session with nothing sent has none).
    if (!s.id || !s.file || !fs.existsSync(s.file)) {
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

  private viewOf(s: DeckSession): SessionView {
    const status = this.procs.statusOf(s);
    const active = s.live && !s.live.exited && (status === 'running' || status === 'waiting');
    const home = os.homedir();
    return {
      title: displayTitle(s, this.store),
      status,
      elsewhere: this.procs.isElsewhere(s),
      agent: 'claude',
      timeLabel: active ? 'now' : humanizeSince(s.mtime),
      cwd: s.cwd.toLowerCase().startsWith(home.toLowerCase()) ? `~${s.cwd.slice(home.length)}` : s.cwd,
      id: s.id,
    };
  }

  // -------------------------------------------------------------------------------------------
  // Attach / detach
  // -------------------------------------------------------------------------------------------

  private attach(s: DeckSession): void {
    if (this.procs.isElsewhere(s)) {
      this.flash('That session is running in another terminal. Close it there first.');
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
    const live = s.live!;
    this.attached = s;
    // Paint the mirrored screen immediately (no waiting for the agent), then let the resize-triggered
    // redraw and live output stream straight through.
    const snapshot = live.serializer.serialize({ scrollback: 0 });
    out.write(`${ESC}?2026h${ESC}0m${ESC}2J${ESC}H${ESC}?25h${snapshot}${ESC}?2026l`);
    resizeLive(live, out.columns || 120, out.rows || 30);
  }

  private detach(note?: string): void {
    this.attached = null;
    out.write(`${RESET_AGENT_MODES}${ESC}?25l${ESC}]0;Session Deck\x07`);
    this.resizeAllToPane();
    if (note) {
      this.flash(note);
    }
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
    if (this.procs.isElsewhere(s)) {
      this.flash('That session is running in another terminal. Rename it there with /rename.');
      return;
    }
    const live = s.live;
    if (live && !live.exited) {
      if (this.procs.statusOf(s) !== 'idle') {
        this.flash('Session is busy or waiting for you. Rename it once it is idle (○).');
        return;
      }
      // Text and Enter as separate writes, so the input box doesn't treat the Enter as part of a paste.
      live.pty.write(`/rename ${name}`);
      setTimeout(() => !live.exited && live.pty.write('\r'), 150);
    } else if (s.id && s.file) {
      appendRenameRecords(s.file, s.id, name);
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
    const t = this.theme;
    const cols = out.columns || 120;
    const height = out.rows || 30;
    const layout = this.layout();

    const counts = Object.fromEntries(STATUS_CATEGORIES.map((c) => [c, 0])) as Record<StatusCategory, number>;
    for (const s of this.sessions) {
      counts[this.procs.categoryOf(s)]++;
    }
    const liveCount = this.sessions.filter((s) => s.live && !s.live.exited).length;

    let frame = `${ESC}?2026h${ESC}0m`;
    // Truncated as a safety net: a wrapped top row would push the whole frame down.
    frame += `${ESC}1;1H${fitAnsi(renderHeader(t, cols, counts, liveCount, themeLabel(this.themePreference, this.theme.name), VERSION), cols)}`;
    frame += `${ESC}2;1H${fitAnsi(renderPills(t, cols, this.sessions.length, counts, this.statusFilter, this.timeFilter), cols)}`;

    if (layout.list) {
      const listRows: ListRow[] = this.rows.map((r) => (r.kind === 'group' ? r : { kind: 'session', view: this.viewOf(r.session), isLast: r.isLast }));
      const filtered = this.statusFilter.size > 0 || this.timeFilter !== 'all';
      const note = this.resizeNote && Date.now() < this.resizeNote.until ? this.resizeNote.text : filtered ? '· filtered' : '';
      const empty = this.sessions.length === 0 ? 'No Claude sessions found.' : 'Nothing matches the filter. Press 0 to clear it.';
      frame += placeLines(layout.list, renderListPanel(t, layout.list, listRows, this.selected, note, empty));
    }
    if (layout.preview) {
      const s = this.current;
      const content = s
        ? {
            view: this.viewOf(s),
            term: s.live && !s.live.exited ? s.live.term : undefined,
            exitCode: s.live?.exitCode,
            lastResponse: this.lastResponseOf(s),
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
    } else if (this.message) {
      frame += renderMessageBar(t, cols, this.message);
    } else {
      frame += renderHelpBar(t, cols);
    }

    if (this.helpScroll !== null) {
      const overlay = helpOverlay(t, cols, height, this.helpScroll, VERSION);
      this.helpScroll = Math.min(this.helpScroll, overlay.maxScroll);
      overlay.lines.forEach((line, i) => (frame += `${ESC}${overlay.y + i + 1};${overlay.x + 1}H${line}`));
    }
    frame += `${ESC}?2026l`;
    out.write(frame);
  }

  /** Starts loading a stopped session's last reply the first time it's previewed. */
  private lastResponseOf(s: DeckSession): string | null | undefined {
    if (s.lastResponse === undefined && s.file) {
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
    } while (this.rows[i] && this.rows[i].kind !== 'session');
    if (this.rows[i]) {
      this.selected = i;
    }
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

  private onKey(data: string): void {
    if (this.prompt) {
      this.onPromptKey(this.prompt, data);
      return;
    }
    if (this.helpScroll !== null) {
      this.onHelpKey(data);
      return;
    }
    const attached = this.attached;
    if (attached) {
      const live = attached.live;
      const idx = findDetachKey(data);
      if (idx >= 0) {
        // Forward keys typed before Ctrl+Q; drop the rest (e.g. the matching key-up events).
        if (idx > 0 && live && !live.exited) {
          live.pty.write(data.slice(0, idx));
        }
        this.detach();
      } else if (live && !live.exited) {
        live.pty.write(data);
      }
      return;
    }

    const s = this.current;
    if (FILTER_KEYS[data]) {
      const category = FILTER_KEYS[data];
      if (!this.statusFilter.delete(category)) {
        this.statusFilter.add(category);
      }
      this.rebuildRows();
      this.render();
      return;
    }
    switch (data) {
      case '\x1b[A':
      case 'k':
        this.move(-1);
        break;
      case '\x1b[B':
      case 'j':
        this.move(1);
        break;
      case '\r':
        if (s) {
          this.attach(s);
        }
        return;
      case 's':
        if (s && this.procs.isElsewhere(s)) {
          this.flash('That session is running in another terminal.');
        } else if (s && (!s.live || s.live.exited)) {
          if (s.live) this.kill(s);
          if (this.start(s)) this.flash('Started in background');
        }
        break;
      case 'n':
        if (s) {
          const fresh: DeckSession = { id: null, cwd: s.cwd, title: '(new session)', mtime: Date.now() };
          this.sessions.unshift(fresh);
          this.rebuildRows();
          this.selected = this.rows.findIndex((r) => r.kind === 'session' && r.session === fresh);
          this.attach(fresh);
          return;
        }
        break;
      case 'e':
      case '\x1bOQ': // F2
      case '\x1b[12~': // F2 (some terminals)
        if (s) {
          const title = displayTitle(s, this.store);
          this.prompt = { label: 'Rename', value: title === '(new session)' ? '' : title, onSubmit: (value) => this.renameSession(s, value) };
        }
        break;
      case 'x':
        if (s && s.live) {
          this.kill(s);
          this.flash('Session stopped');
        }
        break;
      case '*':
        this.timeFilter = nextTimeFilter(this.timeFilter);
        this.rebuildRows();
        break;
      case '0':
        this.statusFilter.clear();
        this.timeFilter = 'all';
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
      case '?':
        this.helpScroll = 0;
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

  private quit(): void {
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
