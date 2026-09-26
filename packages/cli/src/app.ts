import * as fs from 'fs';
import { clearSessionMetaCache, DeckStore, readLastAssistantResponse } from '@session-deck/core';
import { ESC, fit, oneLine, renderTerm, wrap } from './ansi';
import { findDetachKey, RESET_AGENT_MODES } from './keys';
import { disposeLive, resizeLive, spawnClaude } from './liveSession';
import {
  appendRenameRecords,
  buildRows,
  ClaudeProcs,
  DeckSession,
  discoverSessions,
  displayTitle,
  refreshLiveTitle,
  SessionStatus,
  SidebarRow,
} from './sessions';

const out = process.stdout;

const STATUS_BADGE: Record<SessionStatus, string> = {
  running: `${ESC}32m▶${ESC}0m`,
  waiting: `${ESC}33;1m?${ESC}0m`,
  idle: `${ESC}36m●${ESC}0m`,
  starting: `${ESC}36m…${ESC}0m`,
  exited: `${ESC}31m✕${ESC}0m`,
  elsewhere: `${ESC}35m⧉${ESC}0m`,
  stopped: `${ESC}90m·${ESC}0m`,
};

const HINTS = '↑↓ select  Enter attach  s start bg  n new here  e rename  x kill  b sidebar  r refresh  q quit   (Ctrl+Q detaches)';

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
  /** Index into `rows`, always a session row. */
  private selected = 0;
  private sidebarVisible = true;
  private attached: DeckSession | null = null;
  private message = '';
  private messageTimer?: NodeJS.Timeout;
  private renderTimer?: NodeJS.Timeout;
  private prompt: Prompt | null = null;
  private readonly procs = new ClaudeProcs();
  private readonly store = new DeckStore();

  async run(): Promise<void> {
    await this.discover();
    this.pollProcs();
    setInterval(() => this.pollProcs(), 1000);
    // Names/archive flags changed by the VS Code extension (or by us).
    this.store.watch(() => void this.discover().then(() => this.scheduleRender()));

    out.write(`${ESC}?1049h${ESC}?25l${ESC}2J`);
    process.stdin.setRawMode(true);
    process.stdin.setEncoding('utf8');
    process.stdin.on('data', (data: string) => this.onKey(data));
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

  private rebuildRows(): void {
    const current = this.current;
    this.rows = buildRows(this.sessions);
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
      }
    }
    this.scheduleRender();
  }

  private paneSize(): { cols: number; rows: number } {
    const cols = out.columns || 120;
    const height = out.rows || 30;
    const sw = this.sidebarVisible ? sidebarWidth(cols) + 1 : 0;
    return { cols: Math.max(20, cols - sw), rows: Math.max(5, height - 3) };
  }

  private start(s: DeckSession): boolean {
    if (!fs.existsSync(s.cwd)) {
      this.flash(`Folder no longer exists: ${s.cwd}`);
      return false;
    }
    const { cols, rows } = this.paneSize();
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
    // A session with an id stays listed (still resumable from disk); one that never got an id is gone.
    if (!s.id) {
      this.sessions = this.sessions.filter((x) => x !== s);
      this.rebuildRows();
    }
  }

  private resizeAllToPane(): void {
    const { cols, rows } = this.paneSize();
    for (const s of this.sessions) {
      if (s !== this.attached) {
        resizeLive(s.live, cols, rows);
      }
    }
  }

  // -------------------------------------------------------------------------------------------
  // Attach / detach
  // -------------------------------------------------------------------------------------------

  private attach(s: DeckSession): void {
    if (this.procs.statusOf(s) === 'elsewhere') {
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
    const status = this.procs.statusOf(s);
    if (status === 'elsewhere') {
      this.flash('That session is running in another terminal. Rename it there with /rename.');
      return;
    }
    const live = s.live;
    if (live && !live.exited) {
      if (status !== 'idle') {
        this.flash('Session is busy or waiting for you. Rename it once it is idle (●).');
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
    const cols = out.columns || 120;
    const height = out.rows || 30;
    const pane = this.paneSize();
    const bodyHeight = height - 2;
    const current = this.current;

    let frame = `${ESC}?2026h${ESC}H${ESC}0m`;
    const liveCount = this.sessions.filter((s) => s.live && !s.live.exited).length;
    frame += `${ESC}1;7m${fit(` Session Deck — ${liveCount} live in background`, cols)}${ESC}0m`;

    const side = this.sidebarVisible ? this.renderSidebar(sidebarWidth(cols), bodyHeight) : [];
    const title = current
      ? `${STATUS_BADGE[this.procs.statusOf(current)]} ${ESC}1m${fit(displayTitle(current, this.store), pane.cols - 2)}${ESC}0m`
      : fit('', pane.cols);
    const preview = this.renderPreview(current, pane.cols, pane.rows);

    for (let y = 0; y < bodyHeight; y++) {
      frame += `${ESC}${y + 2};1H`;
      if (this.sidebarVisible) {
        frame += `${side[y]}${ESC}90m│${ESC}0m`;
      }
      frame += y === 0 ? title : preview[y - 1] || ' '.repeat(pane.cols);
    }

    if (this.prompt) {
      const label = this.prompt.label;
      frame += `${ESC}${height};1H${ESC}1m ${label}: ${ESC}0m${fit(`${this.prompt.value}█`, Math.max(1, cols - label.length - 3))}`;
    } else {
      frame += `${ESC}${height};1H${ESC}7m${fit(` ${this.message || HINTS}`, cols)}${ESC}0m`;
    }
    frame += `${ESC}?2026l`;
    out.write(frame);
  }

  private renderSidebar(width: number, height: number): string[] {
    const lines: string[] = [];
    // keep selection visible
    const start = Math.max(0, Math.min(this.selected - Math.floor(height / 2), this.rows.length - height));
    for (let i = 0; i < height; i++) {
      const r = this.rows[start + i];
      if (!r) {
        lines.push(' '.repeat(width));
      } else if (r.kind === 'group') {
        lines.push(`${ESC}1;34m${fit(`▾ ${r.label}`, width)}${ESC}0m`);
      } else {
        const text = fit(`   ${displayTitle(r.session, this.store)}`, width - 2);
        const badge = STATUS_BADGE[this.procs.statusOf(r.session)];
        lines.push(start + i === this.selected ? ` ${badge}${ESC}7m${text}${ESC}0m ` : ` ${badge}${text} `);
      }
    }
    return lines;
  }

  private renderPreview(s: DeckSession | undefined, width: number, height: number): string[] {
    if (!s) {
      return Array.from({ length: height }, () => ' '.repeat(width));
    }
    if (s.live && !s.live.exited) {
      return renderTerm(s.live.term, width, height);
    }
    const status = this.procs.statusOf(s);
    const info = [
      '',
      `  ${displayTitle(s, this.store)}`,
      `  ${s.cwd}`,
      `  ${s.id || '(new)'}`,
      '',
      status === 'elsewhere'
        ? '  Running in another terminal. Close it there to open it here.'
        : status === 'exited'
          ? `  Exited (code ${s.live?.exitCode}). Enter = restart, x = clear.`
          : '  Not running.  Enter = start + attach    s = start in background',
      '',
      '  Last response:',
    ];
    if (s.lastResponse === undefined && s.file) {
      s.lastResponse = null; // loading
      void readLastAssistantResponse(s.file).then((r) => {
        s.lastResponse = r || '';
        this.scheduleRender();
      });
    }
    const lines = info.map((l) => fit(l, width));
    for (const l of s.lastResponse ? wrap(s.lastResponse, width - 4) : []) {
      lines.push(`${ESC}90m${fit(`  ${l}`, width)}${ESC}0m`);
    }
    while (lines.length < height) {
      lines.push(' '.repeat(width));
    }
    return lines.slice(0, height);
  }

  // -------------------------------------------------------------------------------------------
  // Input
  // -------------------------------------------------------------------------------------------

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
      prompt.value += data.replace(/[\x00-\x1f\x7f]/g, ''); // typed or pasted text
    }
    this.render();
  }

  private onKey(data: string): void {
    if (this.prompt) {
      this.onPromptKey(this.prompt, data);
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
        if (s && this.procs.statusOf(s) === 'elsewhere') {
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
      case 'b':
      case '\x02': // Ctrl+B
        this.sidebarVisible = !this.sidebarVisible;
        out.write(`${ESC}2J`);
        this.resizeAllToPane();
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

function sidebarWidth(cols: number): number {
  return Math.min(44, Math.max(24, Math.floor(cols * 0.3)));
}
