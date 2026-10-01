import type { Terminal } from '@xterm/headless';
import type { AlertEntry, DailySpend, DeckConfig, GitStatus, TailedTurn, TraceStep, UsageMetric } from '@session-deck/core';
import { agentDisplayName, humanizeSince, renderUsageBar, usageSeverity, USAGE_METRIC_LABELS } from '@session-deck/core';
import { fit, fitAnsi, fitTail, renderTerm, textWidth, wrap } from './ansi';
import { CONFIG_FIELDS } from './configFields';
import { STATUS_CATEGORIES, StatusCategory, TimeFilter } from './filters';
import { PANEL_HEADER_ROWS, Rect } from './layout';
import type { SessionStatus } from './sessions';
import type { LocalAgent } from './agents';
import type { LocalSkill, SkillState } from './skills';
import { Role, Theme, ThemeName } from './theme';

const RESET = '\x1b[0m';
const BOLD = '\x1b[1m';
const UNDERLINE = '\x1b[4m';
const REVERSE = '\x1b[7m';

const GLYPHS: Record<SessionStatus, { char: string; role: Role; bold: boolean }> = {
  running: { char: '●', role: 'red', bold: true },
  waiting: { char: '◐', role: 'yellow', bold: true },
  // Finished, not seen yet: waiting for a look. Same green as the VS Code extension's "Done" decoration.
  done: { char: '●', role: 'green', bold: true },
  idle: { char: '○', role: 'textDim', bold: false },
  starting: { char: '⟳', role: 'yellow', bold: false },
  error: { char: '✕', role: 'red', bold: true },
  exited: { char: '✕', role: 'red', bold: true },
  stopped: { char: '■', role: 'textDim', bold: false },
};

const CATEGORY_GLYPHS: Record<StatusCategory, { char: string; role: Role }> = {
  running: { char: '●', role: 'red' },
  waiting: { char: '◐', role: 'yellow' },
  idle: { char: '○', role: 'textDim' },
  error: { char: '✕', role: 'red' },
  stopped: { char: '■', role: 'textDim' },
};

const AGENT_ROLE: Record<string, Role> = { claude: 'orange', copilot: 'accent' };

/** Everything a row or the preview header shows about one session. */
export interface SessionView {
  title: string;
  status: SessionStatus;
  /** Open in another terminal rather than here. */
  elsewhere: boolean;
  agent: string;
  timeLabel: string;
  cwd: string;
  id: string | null;
  /** Replaces the plain status text, e.g. the screen error ("sign-in failed · run /login"). */
  detail?: string;
  /** `undefined` when `ui.gitStatus` is off, or the cwd isn't a git repo. */
  git?: GitStatus;
}

interface GroupCounts {
  count: number;
  running: number;
  waiting: number;
}

export type ListRow =
  | ({ kind: 'folder'; name: string; collapsed: boolean; hotkey?: number } & GroupCounts)
  | ({ kind: 'project'; label: string; collapsed: boolean; depth: number; hotkey?: number; git?: GitStatus } & GroupCounts)
  | { kind: 'session'; view: SessionView; isLast: boolean; depth: number; pin?: 'top' | 'bottom'; checked?: boolean }
  | { kind: 'divider'; label: string }
  | { kind: 'tag'; name: string; count: number; active: boolean }
  | { kind: 'usage'; metric: UsageMetric; percent?: number; updatedAt?: number; resetLabel?: string }
  | { kind: 'usageBudget'; days: DailySpend[] };

function glyph(t: Theme, status: SessionStatus): string {
  const g = GLYPHS[status];
  return `${g.bold ? BOLD : ''}${t.fg(g.role)}${g.char}${RESET}`;
}

function blank(width: number): string {
  return ' '.repeat(Math.max(0, width));
}

// ---------------------------------------------------------------------------------------------
// Top: header bar and filter pills
// ---------------------------------------------------------------------------------------------

export function renderHeader(
  t: Theme,
  cols: number,
  counts: Record<StatusCategory, number>,
  doneCount: number,
  liveCount: number,
  themeLabel: string,
  version: string,
): string {
  const dot = (char: string, active: boolean, role: Role) => `${t.fg(active ? role : 'textDim')}${char}`;
  const logo = `${t.fg('border')}⟨${dot('●', counts.running > 0, 'red')}${t.fg('border')}│${dot('◐', counts.waiting > 0, 'yellow')}${t.fg('border')}│${dot('●', doneCount > 0, 'green')}${t.fg('border')}│${dot('○', counts.idle > 0, 'text')}${t.fg('border')}⟩`;
  const leftPlain = ' ⟨●│◐│●│○⟩ Session Deck';
  const right = `${liveCount} live · ${themeLabel} · v${version} `;
  const gap = cols - textWidth(leftPlain) - textWidth(right);
  const bar = t.bg('surface');
  if (gap < 1) {
    return `${bar} ${logo} ${bar}${BOLD}${t.fg('accent')}${fit('Session Deck', Math.max(0, cols - 9))}${RESET}`;
  }
  return `${bar} ${logo}${bar} ${BOLD}${t.fg('accent')}Session Deck${RESET}${bar}${blank(gap)}${t.fg('textDim')}${right}${RESET}`;
}

const TIME_LABEL: Record<TimeFilter, string> = { all: 'all time', today: 'today', '3d': '3 days', '7d': '7 days' };

export function renderPills(
  t: Theme,
  cols: number,
  total: number,
  counts: Record<StatusCategory, number>,
  doneCount: number,
  statusFilter: ReadonlySet<StatusCategory>,
  timeFilter: TimeFilter,
  tagFilter?: string
): string {
  const pill = (label: string, active: boolean, labelStyle: string) =>
    active ? `${t.bg('accent')}${t.fg('bg')}${BOLD} ${label} ${RESET}` : ` ${labelStyle}${label}${RESET} `;
  let out = ' ' + pill(`All ${total}`, statusFilter.size === 0, t.fg('text'));
  let plainWidth = 1 + textWidth(`All ${total}`) + 2;
  for (const category of STATUS_CATEGORIES) {
    const g = CATEGORY_GLYPHS[category];
    const label = `${g.char} ${counts[category]}`;
    out += pill(label, statusFilter.has(category), t.fg(g.role));
    plainWidth += textWidth(label) + 2;
    // "done" (finished, not yet seen) folds into the "waiting" filter category, so its pill rides along right after it.
    if (category === 'waiting') {
      const doneLabel = `● ${doneCount}`;
      out += pill(doneLabel, statusFilter.has('waiting'), t.fg('green'));
      plainWidth += textWidth(doneLabel) + 2;
    }
  }
  const time = TIME_LABEL[timeFilter];
  out += `${t.fg('border')}│${RESET}` + pill(time, timeFilter !== 'all', t.fg('purple'));
  plainWidth += 1 + textWidth(time) + 2;
  if (tagFilter) {
    const label = `# ${tagFilter}`;
    out += `${t.fg('border')}│${RESET}` + pill(label, true, t.fg('cyan'));
    plainWidth += 1 + textWidth(label) + 2;
  }
  const hint = '! @ # & ~ filter · * time · 0 clear ';
  const gap = cols - plainWidth - textWidth(hint);
  if (gap >= 2) {
    out += blank(gap) + `${t.fg('textDim')}${hint}${RESET}`;
  } else {
    out += blank(cols - plainWidth);
  }
  return out;
}

// ---------------------------------------------------------------------------------------------
// Panels
// ---------------------------------------------------------------------------------------------

function panelHeader(t: Theme, width: number, title: string, note: string): string[] {
  const noteText = note ? ` ${note}` : '';
  const titleLine = `${BOLD}${t.fg('cyan')}${fit(title, Math.min(width, textWidth(title)))}${RESET}${t.fg('textDim')}${fit(noteText, Math.max(0, width - textWidth(title)))}${RESET}`;
  return [titleLine, `${t.fg('border')}${'─'.repeat(width)}${RESET}`];
}

/** ⇡/⇣ ahead/behind upstream, ✱ dirty — one badge for the whole project, not per session (they'd all share the same repo state). */
function gitGlyphs(g: GitStatus): string {
  return `${g.ahead > 0 ? '⇡' : ''}${g.behind > 0 ? '⇣' : ''}${g.dirty > 0 ? '✱' : ''}`;
}

/** Folder and project rows: `1▾ name (n) ●r ◐w ⇡⇣✱`. The leading column shows the 1–9 jump key on top-level rows. */
function renderGroupRow(t: Theme, width: number, row: Extract<ListRow, { kind: 'folder' | 'project' }>, selected: boolean): string {
  const depth = row.kind === 'project' ? row.depth : 0;
  const hotkey = row.hotkey === undefined ? ' ' : String(row.hotkey);
  const arrow = row.collapsed ? '▸' : '▾';
  const lead = `${hotkey}${'  '.repeat(depth)}${arrow} `;
  const name = row.kind === 'folder' ? row.name : row.label;
  const gitText = row.kind === 'project' && row.git ? gitGlyphs(row.git) : '';
  const counts = `${row.running ? ` ●${row.running}` : ''}${row.waiting ? ` ◐${row.waiting}` : ''}${gitText ? ` ${gitText}` : ''}`;
  const suffix = ` (${row.count})${counts}`;
  const label = fit(name, Math.max(1, width - textWidth(lead) - textWidth(suffix))).trimEnd();
  const pad = blank(width - textWidth(lead) - textWidth(label) - textWidth(suffix));
  if (selected) {
    const sel = `${t.bg('accent')}${t.fg('bg')}`;
    return `${sel}${lead}${BOLD}${label}${RESET}${sel}${suffix}${pad}${RESET}`;
  }
  const nameRole: Role = row.kind === 'folder' ? 'purple' : 'cyan';
  const running = row.running ? ` ${t.fg('red')}●${row.running}` : '';
  const waiting = row.waiting ? ` ${t.fg('yellow')}◐${row.waiting}` : '';
  const git = gitText ? ` ${t.fg('yellow')}${gitText}` : '';
  return (
    `${t.fg('textDim')}${hotkey}${t.fg('text')}${'  '.repeat(depth)}${arrow} ` +
    `${BOLD}${t.fg(nameRole)}${label}${RESET}${t.fg('text')} (${row.count})${running}${waiting}${git}${RESET}${pad}`
  );
}

function renderSessionRow(t: Theme, width: number, row: Extract<ListRow, { kind: 'session' }>, selected: boolean, showCheckbox: boolean): string {
  const v = row.view;
  const indent = '  '.repeat(row.depth);
  const connector = row.isLast ? '└─' : '├─';
  const checkboxPlain = showCheckbox ? (row.checked ? '✔ ' : '· ') : '';
  // Narrow glyphs only: emoji are one column wide in some terminals and two in others.
  const pinMark = row.pin === 'top' ? '↑ ' : row.pin === 'bottom' ? '↓ ' : '';
  const markers = `${pinMark}${v.elsewhere ? '↗ ' : ''}`;
  const leftPlain = `${checkboxPlain}${indent}${connector} ${GLYPHS[v.status].char} ${markers}`;
  const agentText = ` ${v.agent}`;
  const availWidth = Math.max(1, width - textWidth(leftPlain));
  const maxTitleWidth = Math.max(1, availWidth - textWidth(agentText));
  // Truncated to fit, but not padded: the agent name sits right after the title, not at the row's edge.
  const title = fit(v.title, Math.min(textWidth(v.title), maxTitleWidth));
  const pad = blank(Math.max(0, width - textWidth(leftPlain) - textWidth(title) - textWidth(agentText)));
  const active = v.status === 'running' || v.status === 'waiting';

  if (selected) {
    const sel = `${t.bg('accent')}${t.fg('bg')}`;
    return `${sel}${leftPlain}${BOLD}${title}${RESET}${sel}${agentText}${pad}${RESET}`;
  }
  const checkbox = showCheckbox ? (row.checked ? `${t.fg('accent')}${BOLD}✔${RESET} ` : `${t.fg('textDim')}·${RESET} `) : '';
  const titleStyle = `${active ? BOLD : ''}${v.status === 'exited' ? UNDERLINE : ''}${t.fg('text')}`;
  return (
    `${checkbox}${indent}${t.fg('border')}${connector}${RESET} ${glyph(t, v.status)} ${pinMark ? `${t.fg('accent')}${pinMark}` : ''}${v.elsewhere ? `${t.fg('purple')}↗ ` : ''}${RESET}` +
    `${titleStyle}${title}${RESET}${t.fg('textDim')}${agentText}${RESET}${pad}`
  );
}

function renderDividerRow(t: Theme, width: number, label: string): string {
  const text = fit(` ${label} `, Math.max(0, width - 2));
  return `${t.fg('border')}──${t.fg('textDim')}${text}${t.fg('border')}${'─'.repeat(Math.max(0, width - 2 - textWidth(text)))}${RESET}`;
}

/** `# name (n)`, at the bottom of the list. Enter filters to it (again clears); d removes it from every project. */
function renderTagRow(t: Theme, width: number, row: Extract<ListRow, { kind: 'tag' }>, selected: boolean): string {
  const lead = '  ';
  const label = `# ${row.name}`;
  const suffix = ` (${row.count})`;
  const text = fit(label, Math.max(1, width - textWidth(lead) - textWidth(suffix))).trimEnd();
  const pad = blank(width - textWidth(lead) - textWidth(text) - textWidth(suffix));
  if (selected) {
    const sel = `${t.bg('accent')}${t.fg('bg')}`;
    return `${sel}${lead}${BOLD}${text}${RESET}${sel}${suffix}${pad}${RESET}`;
  }
  const style = row.active ? `${BOLD}${t.fg('cyan')}` : t.fg('cyan');
  return `${t.fg('textDim')}${lead}${style}${text}${RESET}${t.fg('textDim')}${suffix}${RESET}${pad}`;
}

/** Fixed-width so Context/5h/7d bars line up with each other and with the "Week" budget row below them. */
const USAGE_LABEL_WIDTH = 7;

const USAGE_SEVERITY_ROLE: Record<ReturnType<typeof usageSeverity>, Role> = { ok: 'green', warning: 'yellow', critical: 'red' };

/** `5h  ███████░░░ 73% (8:30 PM)`, or `7d  ███████░░░ 73% (Mon 8:30 PM)`, or `—` when this session has no usage data recorded yet. */
function renderUsageRow(t: Theme, width: number, row: Extract<ListRow, { kind: 'usage' }>): string {
  const lead = '  ';
  const label = fit(USAGE_METRIC_LABELS[row.metric], USAGE_LABEL_WIDTH);
  const resetSuffix = row.resetLabel ? ` (${row.resetLabel})` : '';
  const text = row.percent === undefined ? `${label} —` : `${label} ${renderUsageBar(row.percent)} ${row.percent}%${resetSuffix}`;
  const fitted = fit(text, Math.max(1, width - textWidth(lead)));
  const pad = blank(width - textWidth(lead) - textWidth(fitted));
  if (row.percent === undefined) {
    return `${t.fg('textDim')}${lead}${fitted}${RESET}${pad}`;
  }
  const role = USAGE_SEVERITY_ROLE[usageSeverity(row.metric, row.percent)];
  return (
    `${t.fg('textDim')}${lead}${t.fg('text')}${label} ${t.fg(role)}${renderUsageBar(row.percent)}${RESET} ${t.fg('text')}${row.percent}%` +
    `${t.fg('textDim')}${resetSuffix}${RESET}${pad}`
  );
}

/** This week's day-by-day spend against the 7d quota (Monday through today, weekends included), below the 7d row. */
function renderUsageBudgetRow(t: Theme, width: number, row: Extract<ListRow, { kind: 'usageBudget' }>): string {
  const lead = '  ';
  const label = fit('Week', USAGE_LABEL_WIDTH);
  const text =
    row.days.length === 0 ? `${label} —` : `${label} ${row.days.map((d) => `${d.label}${Math.round(d.percent)}%`).join(' ')}`;
  const fitted = fit(text, Math.max(1, width - textWidth(lead)));
  const pad = blank(width - textWidth(lead) - textWidth(fitted));
  return `${t.fg('textDim')}${lead}${t.fg('text')}${fitted}${RESET}${pad}`;
}

/** Lines for the sessions panel, exactly `rect.width` columns each, `rect.height` lines. */
export function renderListPanel(t: Theme, rect: Rect, rows: ListRow[], selected: number, note: string, emptyMessage: string): string[] {
  const lines = panelHeader(t, rect.width, 'SESSIONS', note);
  const height = rect.height - PANEL_HEADER_ROWS;
  if (rows.length === 0) {
    lines.push(blank(rect.width), `${t.fg('textDim')}${fit(`  ${emptyMessage}`, rect.width)}${RESET}`);
  }
  // Once anything is checked, every session row reserves the checkbox column so they stay aligned.
  const showCheckbox = rows.some((r) => r.kind === 'session' && r.checked);
  // Keep the selection in view, roughly centered.
  const start = Math.max(0, Math.min(selected - Math.floor(height / 2), rows.length - height));
  for (let i = 0; i < height && lines.length < rect.height; i++) {
    const row = rows[start + i];
    const isSelected = start + i === selected;
    if (!row) {
      lines.push(blank(rect.width));
    } else if (row.kind === 'session') {
      lines.push(renderSessionRow(t, rect.width, row, isSelected, showCheckbox));
    } else if (row.kind === 'divider') {
      lines.push(renderDividerRow(t, rect.width, row.label));
    } else if (row.kind === 'tag') {
      lines.push(renderTagRow(t, rect.width, row, isSelected));
    } else if (row.kind === 'usage') {
      lines.push(renderUsageRow(t, rect.width, row));
    } else if (row.kind === 'usageBudget') {
      lines.push(renderUsageBudgetRow(t, rect.width, row));
    } else {
      lines.push(renderGroupRow(t, rect.width, row, isSelected));
    }
  }
  while (lines.length < rect.height) {
    lines.push(blank(rect.width));
  }
  return lines;
}

/** What the preview shows when a folder or project row is selected. */
export interface GroupPreview {
  kind: 'folder' | 'project';
  name: string;
  /** Project path, or "3 projects" for a folder. */
  detail: string;
  counts: GroupCounts;
  sessions: SessionView[];
}

export function renderGroupPreviewPanel(t: Theme, rect: Rect, group: GroupPreview): string[] {
  const nameRole: Role = group.kind === 'folder' ? 'purple' : 'cyan';
  const heading = `${group.kind === 'folder' ? 'Folder' : 'Project'} · ${group.name}`;
  const titleLine = `${BOLD}${t.fg(nameRole)}${fit(heading, rect.width)}${RESET}`;
  const { count, running, waiting } = group.counts;
  const plural = count === 1 ? '' : 's';
  const meta = ` ${count} session${plural}${running ? ` · ${running} running` : ''}${waiting ? ` · ${waiting} waiting` : ''} · ${group.detail} `;
  const metaFit = fit(meta, Math.max(0, rect.width - 2)).trimEnd();
  const metaLine = `${t.fg('border')}─${t.fg('textDim')}${metaFit}${t.fg('border')}${'─'.repeat(Math.max(0, rect.width - 1 - textWidth(metaFit)))}${RESET}`;
  const key = (k: string) => `${BOLD}${t.fg('accent')}${k}${RESET}${t.fg('textDim')}`;
  const hints =
    group.kind === 'folder'
      ? `${key('Enter')} collapse/expand · ${key('K J')} reorder · ${key('e')} rename · ${key('d')} delete`
      : `${key('Enter')} collapse/expand · ${key('M')} move to folder · ${key('K J')} reorder · ${key('n')} new session · ${key('d')} remove`;
  const body = [blank(rect.width), fitAnsi(`  ${t.fg('textDim')}${hints}${RESET}`, rect.width), blank(rect.width)];
  for (const s of group.sessions) {
    const right = ` ${s.timeLabel} `;
    const title = fit(s.title, Math.max(1, rect.width - 4 - textWidth(right)));
    body.push(`  ${glyph(t, s.status)} ${t.fg('text')}${title}${t.fg('textDim')}${right}${RESET}`);
  }
  const bodyHeight = rect.height - PANEL_HEADER_ROWS;
  while (body.length < bodyHeight) {
    body.push(blank(rect.width));
  }
  return [titleLine, metaLine, ...body.slice(0, bodyHeight)];
}

const STATUS_TEXT: Record<SessionStatus, string> = {
  running: 'running',
  waiting: 'waiting for you',
  done: 'finished · not seen yet',
  idle: 'idle',
  starting: 'starting',
  error: 'error',
  exited: 'exited',
  stopped: 'not running',
};

export interface PreviewContent {
  view: SessionView;
  /** Mirror of the live agent's screen, when it runs here. */
  term?: Terminal;
  /** Keystrokes are actually going into this session ('i'), as opposed to just being previewed. */
  interacting?: boolean;
  exitCode?: number;
  lastResponse?: string | null;
  /** Lines scrolled back from the live bottom (↑↓/PageUp/PageDown/Home/End while this session is selected), see `renderTerm`. */
  scrollOffset?: number;
  /** Tailed turns for a session open elsewhere (`view.elsewhere`), newest last. `undefined`/empty while the tailer is still backfilling. */
  liveTurns?: TailedTurn[];
}

/** Renders tailed turns for an elsewhere session's preview as a read-only mini-transcript, tail-clipped to `height` lines (latest activity anchored to the bottom, like a live terminal). */
function renderLiveTurns(t: Theme, turns: TailedTurn[], width: number, height: number): string[] {
  const lines: string[] = [];
  for (const turn of turns) {
    const label = turn.role === 'user' ? `${BOLD}${t.fg('cyan')}You${RESET}` : `${BOLD}${t.fg('orange')}Claude${RESET}`;
    lines.push(fitAnsi(`  ${label}`, width));
    for (const line of wrap(turn.text, Math.max(10, width - 4))) {
      lines.push(fitAnsi(`  ${t.fg('text')}${line}${RESET}`, width));
    }
    lines.push(blank(width));
  }
  return lines.length > height ? lines.slice(lines.length - height) : lines;
}

/** Lines for the preview panel: a title + meta header, then the live screen or a summary. */
export function renderPreviewPanel(t: Theme, rect: Rect, content: PreviewContent | undefined): string[] {
  if (!content) {
    return [...panelHeader(t, rect.width, 'PREVIEW', ''), ...Array.from({ length: rect.height - PANEL_HEADER_ROWS }, () => blank(rect.width))];
  }
  const { view: v } = content;
  const base = v.detail ?? STATUS_TEXT[v.status];
  const statusText = v.elsewhere ? `${base} in another terminal` : base;
  const titleLine = `${glyph(t, v.status)} ${BOLD}${t.fg('accent')}${fit(v.title, Math.max(1, rect.width - 2))}${RESET}`;
  const gitText = v.git
    ? ` · ⎇${v.git.branch ?? '(detached)'}${v.git.ahead || v.git.behind ? ` ⇡${v.git.ahead} ⇣${v.git.behind}` : ''}${v.git.dirty ? ` ✱${v.git.dirty}` : ''}`
    : '';
  const scrolled = !!content.scrollOffset;
  const meta = ` ${statusText} · ${v.agent} · ${v.timeLabel} · ${v.cwd}${gitText}${scrolled ? ' · ↑ scrolled · End to jump to latest' : ''} `;
  const metaFit = fit(meta, Math.max(0, rect.width - 2)).trimEnd();
  const metaRole = scrolled ? 'yellow' : 'textDim';
  const metaLine = `${t.fg('border')}─${t.fg(metaRole)}${metaFit}${t.fg('border')}${'─'.repeat(Math.max(0, rect.width - 1 - textWidth(metaFit)))}${RESET}`;
  const bodyHeight = rect.height - PANEL_HEADER_ROWS;

  if (content.term) {
    return [titleLine, metaLine, ...renderTerm(content.term, rect.width, bodyHeight, !!content.interacting, content.scrollOffset)];
  }

  if (v.elsewhere) {
    const header = fitAnsi(
      `  ${t.fg('yellow')}Open in another terminal.${RESET} ${t.fg('textDim')}Live preview, read-only · close it there to open it here.${RESET}`,
      rect.width
    );
    const turnsHeight = bodyHeight - 1;
    const turnLines = content.liveTurns?.length
      ? renderLiveTurns(t, content.liveTurns, rect.width, turnsHeight)
      : [fitAnsi(`  ${t.fg('textDim')}loading…${RESET}`, rect.width)];
    // The header stays pinned right under the meta line; the turns themselves are bottom-anchored
    // (blank padding above, not below) so the latest activity sits at the bottom, like a live terminal.
    while (turnLines.length < turnsHeight) {
      turnLines.unshift(blank(rect.width));
    }
    return [titleLine, metaLine, header, ...turnLines.slice(-turnsHeight)];
  }

  const key = (k: string) => `${BOLD}${t.fg('accent')}${k}${RESET}${t.fg('textDim')}`;
  const action =
    v.status === 'exited'
      ? `${t.fg('red')}Exited (code ${content.exitCode}).${t.fg('textDim')} ${key('Enter')} restart · ${key('x')} clear`
      : `${t.fg('textDim')}${key('Enter')} start + attach · ${key('s')} start in background`;
  const body = [
    '',
    `  ${action}`,
    `  ${t.fg('textDim')}${v.id ?? '(new session)'}`,
    '',
    `  ${BOLD}${t.fg('cyan')}LAST RESPONSE`,
  ].map((l) => `${l}${RESET}`);
  const bodyLines = body.map((l) => fitAnsi(l, rect.width));
  if (content.lastResponse === null) {
    bodyLines.push(fitAnsi(`  ${t.fg('textDim')}loading…${RESET}`, rect.width));
  }
  for (const line of content.lastResponse ? wrap(content.lastResponse, Math.max(10, rect.width - 4)) : []) {
    bodyLines.push(`${t.fg('text')}${fit(`  ${line}`, rect.width)}${RESET}`);
  }
  while (bodyLines.length < bodyHeight) {
    bodyLines.push(blank(rect.width));
  }
  return [titleLine, metaLine, ...bodyLines.slice(0, bodyHeight)];
}


// ---------------------------------------------------------------------------------------------
// Bottom: help bar, and the ? overlay
// ---------------------------------------------------------------------------------------------

type Hint = [key: string, label: string];

/** Widest first; the first variant that fits is shown. */
const HELP_VARIANTS: Hint[][] = [
  [['↑↓', 'select'], ['⏎', 'attach'], ['Space', 'mark'], ['n', 'new'], ['o', 'prompt'], ['e', 'rename'], ['x', 'stop'], ['A', 'archive'], ['d', 'delete'], ['M', 'move'], ['?', 'help'], ['q', 'quit']],
  [['↑↓', 'select'], ['⏎', 'attach'], ['n', 'new'], ['e', 'rename'], ['x', 'stop'], ['?', 'help'], ['q', 'quit']],
  [['⏎', 'attach'], ['?', 'help'], ['q', 'quit']],
  [['?', 'help']],
];

export function renderHelpBar(t: Theme, cols: number): string {
  for (const variant of HELP_VARIANTS) {
    const plain = ' ' + variant.map(([k, l]) => `${k} ${l}`).join(' · ');
    if (textWidth(plain) <= cols) {
      const styled = variant.map(([k, l]) => `${BOLD}${t.fg('accent')}${k}${RESET} ${t.fg('textDim')}${l}`).join(`${t.fg('border')} · `);
      return ` ${styled}${RESET}${blank(cols - textWidth(plain))}`;
    }
  }
  return blank(cols);
}

export function renderMessageBar(t: Theme, cols: number, message: string): string {
  return `${BOLD}${t.fg('yellow')}${fit(` ${message}`, cols)}${RESET}`;
}

export function renderPromptBar(t: Theme, cols: number, label: string, value: string): string {
  const prefix = ` ${label}: `;
  const width = Math.max(1, cols - textWidth(prefix));
  const cursorWidth = width > 0 ? 1 : 0;
  // Keep the tail visible rather than truncating it away when the value overflows.
  const visible = fitTail(value, Math.max(0, width - cursorWidth));
  const visibleWidth = Math.min(textWidth(visible), width - cursorWidth);
  // A reverse-video space instead of a block glyph: relies only on color swap, not on the terminal
  // having (and correctly rendering) a full-block character, which some terminals drop in some window states.
  const cursor = cursorWidth ? `${REVERSE} ${RESET}${t.fg('text')}` : '';
  const pad = ' '.repeat(Math.max(0, width - visibleWidth - cursorWidth));
  return `${BOLD}${t.fg('accent')}${prefix}${RESET}${t.fg('text')}${visible}${cursor}${pad}${RESET}`;
}

const HELP_SECTIONS: { title: string; keys: Hint[] }[] = [
  {
    title: 'QUICK START',
    keys: [
      ['Enter', 'Attach full-screen (starts it if needed)'],
      ['Ctrl+Q', 'Detach back here; the session keeps running'],
      ['Ctrl+K q', 'Detach and stop the session, same as x'],
      ['Ctrl+K n', 'New session in the same project, without detaching first'],
      ['i', 'Type into it right here, list and preview still showing'],
      ['Ctrl+K T', 'Swap attached ⇄ typing here, without detaching to the list first'],
    ],
  },
  {
    title: 'NAVIGATION',
    keys: [
      ['j k', 'Select (preview follows)'],
      ['↑ ↓', 'Select, or scroll the preview if it has a live session in it'],
      ['PgUp/Dn  Home/End', "Scroll the selected session's preview"],
      ['← →  Tab', 'Collapse / expand; ← also goes to the parent'],
      ['1-9', 'Jump to a top-level folder or project'],
      ['`', 'Back to the previously selected session'],
      ['[ ]', 'Previous / next started session (running, waiting or idle), wrapping around'],
      ['/', "Search every session's prompts and replies"],
    ],
  },
  {
    title: 'SESSIONS',
    keys: [
      ['s', 'Start in the background'],
      ['R', 'Restart (a fresh process, same conversation)'],
      ['n', "New session in the project, with n's default agent"],
      ['N', "New session in the project, choosing the agent just this once (doesn't change n's default)"],
      ['F3', "Pick n's default agent"],
      ['p', 'Add a project: new session in any folder'],
      ['o', 'Send a one-line prompt without attaching'],
      ['c', 'Copy the last response'],
      ['v', 'Trajectory: structured trace of messages and tool calls (Claude sessions only)'],
      ['e  F2', "Rename (same as Claude's /rename)"],
      ['Ctrl+L', "Clear context (same as Claude's /clear), without attaching — idle, done, or error only"],
      ['x', 'Stop the selected session'],
      ['u', 'Mark as unread (finished, not seen)'],
      ['U', 'Mark as read'],
      ['A', 'Archive / unarchive (^ shows archived)'],
      ['d', 'Delete: move to the trash'],
      ['Ctrl+Z  Z', 'Undo the delete / open the trash'],
      [',', 'Pin: top · bottom · off'],
      ['r', 'Refresh the list'],
    ],
  },
  {
    title: 'MULTI-SELECT',
    keys: [
      ['Space', 'Check the session for a batch action, then move down'],
      ['Esc', 'Clear the checked sessions'],
      ['A x d M L', 'Archive / stop / delete / move to folder / tag — applied to every checked session'],
    ],
  },
  {
    title: 'FOLDERS & ORDER',
    keys: [
      ['g', 'New folder'],
      ['M', 'Move the project to a folder'],
      ['K J', 'Move the folder / project up or down'],
      ['e  d', 'Rename / delete the selected folder'],
      ['d', 'Remove the selected project from the list (p to add it back)'],
      ['S', 'Sort sessions: recent · actionable'],
      ['t', 'View: normal · active on top'],
    ],
  },
  {
    title: 'TAGS',
    keys: [
      ['L', 'Add / edit tags on the session (checked batch: add to all)'],
      ['Enter', 'On a tag (bottom of the list): filter to it, again to clear'],
      ['d', 'On a tag: remove it from every session'],
    ],
  },
  {
    title: 'FILTER',
    keys: [
      ['!  @  #', 'Running · waiting · idle'],
      ['&  ~', 'Error · stopped'],
      ['*', 'Time: all · today · 3 days · 7 days'],
      ['0', 'Clear filters'],
    ],
  },
  {
    title: 'VIEW',
    keys: [
      ['< >', 'Narrow / widen the sessions panel'],
      ['b  Ctrl+K b', 'Hide / show the sessions panel'],
      ['T', 'Theme: dark · light · system'],
      ['m  Ctrl+K m', 'Mouse scrolling: off by default (so click-drag selects text), on to scroll the preview with the wheel'],
    ],
  },
  {
    title: 'OTHER',
    keys: [
      ['?', 'This help'],
      ['C', 'Show the config file in use'],
      ['w', 'Show local skills / agents (← → switches tabs)'],
      ['a', 'Alert history: every status change sdeck has noticed'],
      ['q  Ctrl+C', 'Quit (stops background sessions)'],
    ],
  },
];

const KEY_COLUMN = 14;

/** The help box's lines (without the frame) and whether it scrolled; drawn centered over the screen. */
export function helpOverlay(t: Theme, cols: number, rows: number, scroll: number, version: string): { x: number; y: number; lines: string[]; maxScroll: number } {
  const width = Math.min(66, cols - 4);
  const inner = width - 6; // borders + two spaces of padding on each side
  const content: string[] = [];
  for (const section of HELP_SECTIONS) {
    content.push(`${BOLD}${t.fg('cyan')}${fit(section.title, inner)}`);
    for (const [k, label] of section.keys) {
      content.push(`${BOLD}${t.fg('purple')}${fit(k, KEY_COLUMN)}${RESET}${t.bg('surface')}${t.fg('text')}${fit(label, inner - KEY_COLUMN)}`);
    }
    content.push(blank(inner));
  }
  content.push(`${t.fg('textDim')}${fit(`Session Deck v${version} · Esc or ? to close`, inner)}`);

  const maxBody = Math.max(3, rows - 6);
  const maxScroll = Math.max(0, content.length - maxBody);
  const offset = Math.min(scroll, maxScroll);
  const visible = content.slice(offset, offset + maxBody);
  const s = t.bg('surface');
  const border = `${s}${t.fg('purple')}`;
  const title = ' KEYBOARD SHORTCUTS ';
  const left = Math.floor((width - 2 - title.length) / 2);
  const lines = [
    `${border}╭${'─'.repeat(left)}${BOLD}${title}${RESET}${border}${'─'.repeat(width - 2 - left - title.length)}╮${RESET}`,
    `${border}│${s}${blank(width - 2)}${border}│${RESET}`,
    ...visible.map((l) => `${border}│${s}  ${l}${RESET}${s}  ${border}│${RESET}`),
    `${border}│${s}${t.fg('yellow')}${fit(offset < maxScroll ? '  ▼ more below' : offset > 0 ? '  ▲ more above' : '', width - 2)}${border}│${RESET}`,
    `${border}╰${'─'.repeat(width - 2)}╯${RESET}`,
  ];
  return { x: Math.max(0, Math.floor((cols - width) / 2)), y: Math.max(0, Math.floor((rows - lines.length) / 2)), lines, maxScroll };
}

// Wide enough for the longest field label (`ui.expandCollapsedOnActiveJump`) plus a gap before the value.
const CONFIG_LABEL_COLUMN = Math.max(...CONFIG_FIELDS.map((f) => f.label.length)) + 2;

/**
 * The settings popup (`C`): every value from `~/.session-deck/config.json` Session Deck is actually
 * using, editable in place. `selected` indexes into `CONFIG_FIELDS` and is drawn like a picker's
 * highlighted row. Scrolls to keep the selected row in view, like `pickerOverlay`.
 */
export function configOverlay(
  t: Theme,
  cols: number,
  rows: number,
  config: DeckConfig,
  configPath: string,
  selected: number
): { x: number; y: number; lines: string[] } {
  const footerText = '↑↓ select · Enter toggle/edit · Esc or C to close';
  // Only the label/value columns and the footer hint drive the width — the config path is left to
  // truncate, since a long filesystem path shouldn't blow the popup out to fill a wide terminal.
  const maxValueWidth = Math.max(...CONFIG_FIELDS.map((f) => textWidth(f.display(config))));
  const neededInner = Math.max(CONFIG_LABEL_COLUMN + maxValueWidth, textWidth(footerText));
  const width = Math.min(cols - 4, neededInner + 6);
  const inner = width - 6; // borders + two spaces of padding on each side
  const s = t.bg('surface');
  const fieldRow = (index: number): string => {
    const field = CONFIG_FIELDS[index];
    const value = field.display(config);
    if (index === selected) {
      return `${t.bg('accent')}${t.fg('bg')}${BOLD}${fit(field.label, CONFIG_LABEL_COLUMN)}${fit(value, inner - CONFIG_LABEL_COLUMN)}`;
    }
    return `${BOLD}${t.fg('purple')}${fit(field.label, CONFIG_LABEL_COLUMN)}${RESET}${s}${t.fg('text')}${fit(value, inner - CONFIG_LABEL_COLUMN)}`;
  };
  // A blank line before each new group of fields (their label shares everything up to the last '.').
  const groupOf = (label: string) => label.slice(0, label.lastIndexOf('.'));
  const content: string[] = [`${t.fg('textDim')}${fit(configPath, inner)}`, blank(inner)];
  let selectedLine = 2;
  CONFIG_FIELDS.forEach((field, index) => {
    if (index > 0 && groupOf(field.label) !== groupOf(CONFIG_FIELDS[index - 1].label)) {
      content.push(blank(inner));
    }
    if (index === selected) {
      selectedLine = content.length;
    }
    content.push(fieldRow(index));
  });
  content.push(blank(inner), `${t.fg('textDim')}${fit(footerText, inner)}`);

  const maxBody = Math.max(3, rows - 6);
  const maxScroll = Math.max(0, content.length - maxBody);
  const offset = Math.max(0, Math.min(maxScroll, selectedLine - Math.floor(maxBody / 2)));
  const visible = content.slice(offset, offset + maxBody);
  const border = `${s}${t.fg('purple')}`;
  const title = ' CONFIG ';
  const left = Math.floor((width - 2 - title.length) / 2);
  const lines = [
    `${border}╭${'─'.repeat(left)}${BOLD}${title}${RESET}${border}${'─'.repeat(width - 2 - left - title.length)}╮${RESET}`,
    `${border}│${s}${blank(width - 2)}${border}│${RESET}`,
    ...visible.map((l) => `${border}│${s}  ${l}${RESET}${s}  ${border}│${RESET}`),
    `${border}│${s}${t.fg('yellow')}${fit(offset < maxScroll ? '  ▼ more below' : offset > 0 ? '  ▲ more above' : '', width - 2)}${border}│${RESET}`,
    `${border}╰${'─'.repeat(width - 2)}╯${RESET}`,
  ];
  return { x: Math.max(0, Math.floor((cols - width) / 2)), y: Math.max(0, Math.floor((rows - lines.length) / 2)), lines };
}

export type SkillsPopupTab = 'skills' | 'agents';

const POPUP_NAME_COLUMN = 32;

const SKILL_STATE_ORDER: readonly SkillState[] = ['on', 'name-only', 'user-invocable-only', 'off'];

const SKILL_STATE_LABELS: Record<SkillState, string> = {
  on: 'ON — visible + auto-triggerable',
  'name-only': 'NAME-ONLY — name visible, no description',
  'user-invocable-only': 'USER-INVOCABLE-ONLY — hidden from context, still in the / menu',
  off: 'OFF — removed entirely, even from /',
};

const POPUP_TAB_LABELS: Record<SkillsPopupTab, string> = { skills: 'Skills', agents: 'Agents' };
const POPUP_TABS: readonly SkillsPopupTab[] = ['skills', 'agents'];

const NOT_CLAUDE_MESSAGE = 'Skills and subagents are a Claude Code concept — press F3 and switch the Session Deck Agent to Claude to see them.';

function skillsTabLines(t: Theme, inner: number, skills: readonly LocalSkill[], agentId: string): string[] {
  const content: string[] = [];
  if (agentId !== 'claude') {
    content.push(`${t.fg('textDim')}${fit(NOT_CLAUDE_MESSAGE, inner)}`);
    return content;
  }
  if (skills.length === 0) {
    content.push(`${t.fg('textDim')}${fit('No local skills found under ~/.claude/skills.', inner)}`);
  }
  for (const state of SKILL_STATE_ORDER) {
    const group = skills.filter((skill) => skill.state === state);
    if (group.length === 0) {
      continue;
    }
    content.push(`${BOLD}${t.fg('cyan')}${fit(`${SKILL_STATE_LABELS[state]} (${group.length})`, inner)}`);
    for (const skill of group) {
      content.push(
        `${BOLD}${t.fg('purple')}${fit(skill.name, POPUP_NAME_COLUMN)}${RESET}${t.bg('surface')}${t.fg('text')}${fit(skill.description, inner - POPUP_NAME_COLUMN)}`
      );
    }
    content.push(blank(inner));
  }
  return content;
}

function agentsTabLines(t: Theme, inner: number, agents: readonly LocalAgent[], agentId: string): string[] {
  const content: string[] = [];
  if (agentId !== 'claude') {
    content.push(`${t.fg('textDim')}${fit(NOT_CLAUDE_MESSAGE, inner)}`);
    return content;
  }
  if (agents.length === 0) {
    content.push(`${t.fg('textDim')}${fit('No local agents found under ~/.claude/agents.', inner)}`);
  }
  for (const agent of agents) {
    content.push(
      `${BOLD}${t.fg('purple')}${fit(agent.name, POPUP_NAME_COLUMN)}${RESET}${t.bg('surface')}${t.fg('text')}${fit(agent.description, inner - POPUP_NAME_COLUMN)}`
    );
  }
  return content;
}

function popupTabsRow(t: Theme, active: SkillsPopupTab, width: number): string {
  const s = t.bg('surface');
  const segments = POPUP_TABS.map((tab) => {
    const label = ` ${POPUP_TAB_LABELS[tab]} `;
    return tab === active ? `${t.bg('accent')}${t.fg('bg')}${BOLD}${label}${RESET}${s}` : `${t.fg('textDim')}${label}${RESET}${s}`;
  });
  return fitAnsi(segments.join('  '), width);
}

/**
 * The skills/agents popup (`w`): local skills from `discoverLocalSkills`, grouped by their effective
 * `skillOverrides` state (a skill with no override shows as `on`), or local subagents from
 * `discoverLocalAgents` sorted alphabetically — subagents have no `skillOverrides`-style visibility
 * state, so there's nothing to group by there. Both are a Claude Code concept: with any other Session
 * Deck Agent selected (see `openAgentPicker`/`F3`), both tabs explain there's nothing to show instead.
 * `← →` switches tabs; scrolls like the help overlay.
 */
export function skillsOverlay(
  t: Theme,
  cols: number,
  rows: number,
  tab: SkillsPopupTab,
  skills: readonly LocalSkill[],
  agents: readonly LocalAgent[],
  scroll: number,
  agentId: string
): { x: number; y: number; lines: string[]; maxScroll: number } {
  const width = Math.min(88, cols - 4);
  const inner = width - 6;
  const content = tab === 'skills' ? skillsTabLines(t, inner, skills, agentId) : agentsTabLines(t, inner, agents, agentId);
  content.push(blank(inner));
  content.push(`${t.fg('textDim')}${fit('← → switch tabs · Esc or w to close', inner)}`);

  const maxBody = Math.max(3, rows - 7);
  const maxScroll = Math.max(0, content.length - maxBody);
  const offset = Math.min(scroll, maxScroll);
  const visible = content.slice(offset, offset + maxBody);
  const s = t.bg('surface');
  const border = `${s}${t.fg('purple')}`;
  const title = ` SKILLS & AGENTS · ${agentDisplayName(agentId).toUpperCase()} `;
  const left = Math.floor((width - 2 - title.length) / 2);
  const lines = [
    `${border}╭${'─'.repeat(left)}${BOLD}${title}${RESET}${border}${'─'.repeat(width - 2 - left - title.length)}╮${RESET}`,
    `${border}│${s}  ${popupTabsRow(t, tab, inner)}${RESET}${s}  ${border}│${RESET}`,
    `${border}│${s}${blank(width - 2)}${border}│${RESET}`,
    ...visible.map((l) => `${border}│${s}  ${l}${RESET}${s}  ${border}│${RESET}`),
    `${border}│${s}${t.fg('yellow')}${fit(offset < maxScroll ? '  ▼ more below' : offset > 0 ? '  ▲ more above' : '', width - 2)}${border}│${RESET}`,
    `${border}╰${'─'.repeat(width - 2)}╯${RESET}`,
  ];
  return { x: Math.max(0, Math.floor((cols - width) / 2)), y: Math.max(0, Math.floor((rows - lines.length) / 2)), lines, maxScroll };
}

const TRACE_KIND_ROLE: Record<TraceStep['kind'], Role> = { user: 'cyan', assistant: 'orange', tool: 'purple' };
const TRACE_KIND_PREFIX: Record<TraceStep['kind'], string> = { user: 'You', assistant: 'Claude', tool: 'Tool' };

/**
 * The trajectory popup (`v`): a two-column trace of one Claude session's prompts, replies and tool
 * calls (see `readSessionTrace`/`buildTraceFromLines`) — left = step list, right = the selected step's
 * full detail, like pitago's `/trajectory`. `selected` picks the step (↑↓); `detailScroll` scrolls the
 * right column independently, since a tool's input/result commonly runs longer than the box is tall.
 */
export function traceOverlay(
  t: Theme,
  cols: number,
  rows: number,
  steps: readonly TraceStep[],
  selected: number,
  detailScroll: number
): { x: number; y: number; lines: string[]; maxDetailScroll: number } {
  const width = Math.min(110, cols - 4);
  const inner = width - 6;
  const leftW = Math.max(16, Math.min(36, Math.floor(inner * 0.34)));
  const rightW = Math.max(10, inner - leftW - 1);
  const bodyHeight = Math.max(3, rows - 7);

  const leftLines: string[] = [];
  if (steps.length === 0) {
    leftLines.push(`${t.fg('textDim')}${fit('No messages or tool calls yet.', leftW)}`);
  }
  const start = Math.max(0, Math.min(selected - Math.floor(bodyHeight / 2), Math.max(0, steps.length - bodyHeight)));
  for (let i = 0; i < bodyHeight; i++) {
    const step = steps[start + i];
    if (!step) {
      leftLines.push(blank(leftW));
      continue;
    }
    const text = `${TRACE_KIND_PREFIX[step.kind]}: ${step.isError ? '✕ ' : ''}${step.label}`;
    leftLines.push(
      start + i === selected
        ? `${t.bg('accent')}${t.fg('bg')}${BOLD}${fit(` ${text}`, leftW)}`
        : `${t.fg(step.isError ? 'red' : TRACE_KIND_ROLE[step.kind])}${fit(` ${text}`, leftW)}`
    );
  }

  const selectedStep = steps[selected];
  const detailWrapped = selectedStep ? wrap(selectedStep.detail, Math.max(10, rightW - 1)) : [];
  const maxDetailScroll = Math.max(0, detailWrapped.length - bodyHeight);
  const detailOffset = Math.min(detailScroll, maxDetailScroll);
  const detailVisible = detailWrapped.slice(detailOffset, detailOffset + bodyHeight);

  const s = t.bg('surface');
  const bodyRows: string[] = [];
  for (let i = 0; i < bodyHeight; i++) {
    const left = leftLines[i] ?? blank(leftW);
    const right = `${t.fg('text')}${fit(detailVisible[i] ?? '', rightW)}`;
    bodyRows.push(`${left}${RESET}${s}${t.fg('border')}│${right}`);
  }
  const scrollNote = maxDetailScroll > 0 ? (detailOffset < maxDetailScroll ? ' ▼ more below' : detailOffset > 0 ? ' ▲ more above' : '') : '';
  bodyRows.push(`${t.fg('textDim')}${fit(`↑↓ select step · PgUp/PgDn scroll detail${scrollNote} · Esc or v to close`, inner)}`);

  const maxBody = Math.max(4, rows - 6);
  const offset = Math.max(0, bodyRows.length - maxBody);
  const visible = bodyRows.slice(offset, offset + maxBody);
  const border = `${s}${t.fg('purple')}`;
  const title = ' TRAJECTORY ';
  const left0 = Math.floor((width - 2 - title.length) / 2);
  const lines = [
    `${border}╭${'─'.repeat(left0)}${BOLD}${title}${RESET}${border}${'─'.repeat(width - 2 - left0 - title.length)}╮${RESET}`,
    `${border}│${s}${blank(width - 2)}${border}│${RESET}`,
    ...visible.map((l) => `${border}│${s}  ${l}${RESET}${s}  ${border}│${RESET}`),
    `${border}╰${'─'.repeat(width - 2)}╯${RESET}`,
  ];
  return { x: Math.max(0, Math.floor((cols - width) / 2)), y: Math.max(0, Math.floor((rows - lines.length) / 2)), lines, maxDetailScroll };
}

export function themeLabel(preference: string, resolved: ThemeName): string {
  return preference === 'system' ? `system (${resolved})` : resolved;
}

export function renderConfirmBar(t: Theme, cols: number, question: string): string {
  return `${BOLD}${t.fg('yellow')}${fit(` ${question} (y/N)`, cols)}${RESET}`;
}

/** A small centered list to choose from (↑↓, Enter, Esc), e.g. the target folder for a move. */
export function pickerOverlay(t: Theme, cols: number, rows: number, title: string, items: string[], index: number): { x: number; y: number; lines: string[] } {
  const width = Math.min(cols - 4, Math.max(textWidth(title) + 8, ...items.map((i) => textWidth(i) + 8), 30));
  const inner = width - 4;
  const maxItems = Math.max(1, rows - 6);
  const first = Math.max(0, Math.min(index - Math.floor(maxItems / 2), items.length - maxItems));
  const s = t.bg('surface');
  const border = `${s}${t.fg('purple')}`;
  const heading = fit(` ${title} `, width - 4).trimEnd();
  const lines = [
    `${border}╭─${BOLD}${heading}${RESET}${border}${'─'.repeat(Math.max(0, width - 3 - textWidth(heading)))}╮${RESET}`,
    ...items.slice(first, first + maxItems).map((item, i) => {
      const selected = first + i === index;
      const style = selected ? `${t.bg('accent')}${t.fg('bg')}${BOLD}` : `${s}${t.fg('text')}`;
      return `${border}│${s} ${style}${fit(` ${item}`, inner)}${RESET}${s} ${border}│${RESET}`;
    }),
    `${border}╰${'─'.repeat(width - 2)}╯${RESET}`,
  ];
  return { x: Math.max(0, Math.floor((cols - width) / 2)), y: Math.max(0, Math.floor((rows - lines.length) / 2)), lines };
}

/** Centered "quit with active sessions" warning (Ctrl+C / q with something running or waiting): Yes/No buttons, default on No. */
export function quitConfirmOverlay(t: Theme, cols: number, rows: number, activeCount: number, selected: 'yes' | 'no'): { x: number; y: number; lines: string[] } {
  const message = `${activeCount} session${activeCount === 1 ? '' : 's'} still running will be stopped.`;
  const width = Math.min(cols - 4, Math.max(textWidth(message) + 8, 40));
  const inner = width - 4;
  const s = t.bg('surface');
  const border = `${s}${t.fg('yellow')}`;
  const heading = ' QUIT SESSION DECK? ';

  const yesText = ' Yes ';
  const noText = ' No ';
  const gap = '  ';
  const buttonsWidth = textWidth(yesText) + textWidth(gap) + textWidth(noText);
  const leftPad = Math.max(0, Math.floor((inner - buttonsWidth) / 2));
  const rightPad = Math.max(0, inner - buttonsWidth - leftPad);
  const button = (text: string, isSelected: boolean): string =>
    isSelected ? `${t.bg('accent')}${t.fg('bg')}${BOLD}${text}${RESET}${s}` : `${t.fg('text')}${text}${RESET}${s}`;
  const buttonsLine = `${' '.repeat(leftPad)}${button(yesText, selected === 'yes')}${gap}${button(noText, selected === 'no')}${' '.repeat(rightPad)}`;

  const blank = ' '.repeat(inner);
  const lines = [
    `${border}╭─${BOLD}${fit(heading, width - 4).trimEnd()}${RESET}${border}${'─'.repeat(Math.max(0, width - 3 - textWidth(heading)))}╮${RESET}`,
    `${border}│${s} ${blank}${s} ${border}│${RESET}`,
    `${border}│${s} ${t.fg('text')}${fit(message, inner)}${RESET}${s} ${border}│${RESET}`,
    `${border}│${s} ${blank}${s} ${border}│${RESET}`,
    `${border}│${s} ${buttonsLine}${s} ${border}│${RESET}`,
    `${border}│${s} ${blank}${s} ${border}│${RESET}`,
    `${border}╰${'─'.repeat(width - 2)}╯${RESET}`,
  ];
  return { x: Math.max(0, Math.floor((cols - width) / 2)), y: Math.max(0, Math.floor((rows - lines.length) / 2)), lines };
}

export interface SearchResultRow {
  view: SessionView;
  projectLabel: string;
}

/** The global search popup (`/`): a live-filtered full-text search across every session's prompts and replies. */
export function searchOverlay(
  t: Theme,
  cols: number,
  rows: number,
  query: string,
  loading: boolean,
  results: readonly SearchResultRow[],
  index: number
): { x: number; y: number; lines: string[] } {
  const width = Math.min(84, cols - 4);
  const inner = width - 4;
  const s = t.bg('surface');
  const border = `${s}${t.fg('purple')}`;
  const title = ' SEARCH ';
  const left = Math.floor((width - 2 - title.length) / 2);

  const resultLine = (row: SearchResultRow, selected: boolean): string => {
    const v = row.view;
    const glyphPart = ` ${glyph(t, v.status)} `;
    const agentPart = ` ${v.agent} `;
    const projectPart = ` ${row.projectLabel} `;
    const titleWidth = Math.max(1, inner - textWidth(glyphPart) - textWidth(agentPart) - textWidth(projectPart));
    const titleText = fit(v.title, titleWidth);
    if (selected) {
      const sel = `${t.bg('accent')}${t.fg('bg')}`;
      return `${sel}${glyphPart}${BOLD}${titleText}${RESET}${sel}${projectPart}${agentPart}${RESET}`;
    }
    return `${s}${glyphPart}${t.fg('text')}${titleText}${RESET}${s}${t.fg('textDim')}${projectPart}${t.fg(AGENT_ROLE[v.agent] ?? 'text')}${agentPart}${RESET}`;
  };

  const maxItems = Math.max(1, rows - 9);
  const first = Math.max(0, Math.min(index - Math.floor(maxItems / 2), results.length - maxItems));
  const body: string[] = loading
    ? [`${s}${t.fg('textDim')}${fit(' Reading session content…', inner)}`]
    : query.trim() === ''
      ? [`${s}${t.fg('textDim')}${fit(" Type to search every session's prompts and replies.", inner)}`]
      : results.length === 0
        ? [`${s}${t.fg('textDim')}${fit(' No matches.', inner)}`]
        : results.slice(first, first + maxItems).map((r, i) => resultLine(r, first + i === index));

  const lines = [
    `${border}╭${'─'.repeat(left)}${BOLD}${title}${RESET}${border}${'─'.repeat(width - 2 - left - title.length)}╮${RESET}`,
    `${border}│${s}${BOLD}${t.fg('accent')}${fit(` /${query}█`, inner)}${RESET}${border}│${RESET}`,
    `${border}│${s}${blank(inner)}${border}│${RESET}`,
    ...body.map((l) => `${border}│${l}${border}│${RESET}`),
    `${border}│${s}${t.fg('textDim')}${fit(' ↑↓ select · Enter jump · !@#&~ status filter · Esc cancel', inner)}${border}│${RESET}`,
    `${border}╰${'─'.repeat(width - 2)}╯${RESET}`,
  ];
  return { x: Math.max(0, Math.floor((cols - width) / 2)), y: Math.max(0, Math.floor((rows - lines.length) / 2)), lines };
}

const ALERT_VERB: Record<AlertEntry['status'], string> = { running: 'Running', waiting: 'Waiting', done: 'Done', error: 'Error' };

function alertLines(t: Theme, inner: number, alerts: readonly AlertEntry[]): string[] {
  if (alerts.length === 0) {
    return [`${t.fg('textDim')}${fit('Nothing yet — status changes show up here as they happen.', inner)}`];
  }
  return alerts.map((a) => {
    const time = humanizeSince(a.at);
    const text = `${ALERT_VERB[a.status]}: ${a.label}${a.project ? ` · ${a.project}` : ''}`;
    const availWidth = Math.max(1, inner - 3 - textWidth(time));
    return `${glyph(t, a.status)} ${t.fg('text')}${fit(text, availWidth)} ${t.fg('textDim')}${time}${RESET}`;
  });
}

/**
 * The alert history popup (`a`): every status change sdeck has noticed (see `appendAlert`/`readAlerts`
 * in core), newest first — logged whether or not it also fired a desktop toast (see `WaitingNotifier`).
 * Scrolls like the skills/help overlays; read-only, no per-row action.
 */
export function alertsOverlay(
  t: Theme,
  cols: number,
  rows: number,
  alerts: readonly AlertEntry[],
  scroll: number
): { x: number; y: number; lines: string[]; maxScroll: number } {
  const width = Math.min(88, cols - 4);
  const inner = width - 6;
  const content = alertLines(t, inner, alerts);
  content.push(blank(inner));
  content.push(`${t.fg('textDim')}${fit('Esc or a to close', inner)}`);

  const maxBody = Math.max(3, rows - 6);
  const maxScroll = Math.max(0, content.length - maxBody);
  const offset = Math.min(scroll, maxScroll);
  const visible = content.slice(offset, offset + maxBody);
  const s = t.bg('surface');
  const border = `${s}${t.fg('purple')}`;
  const title = ' ALERTS ';
  const left = Math.floor((width - 2 - title.length) / 2);
  const lines = [
    `${border}╭${'─'.repeat(left)}${BOLD}${title}${RESET}${border}${'─'.repeat(width - 2 - left - title.length)}╮${RESET}`,
    `${border}│${s}${blank(width - 2)}${border}│${RESET}`,
    ...visible.map((l) => `${border}│${s}  ${l}${RESET}${s}  ${border}│${RESET}`),
    `${border}│${s}${t.fg('yellow')}${fit(offset < maxScroll ? '  ▼ more below' : offset > 0 ? '  ▲ more above' : '', width - 2)}${border}│${RESET}`,
    `${border}╰${'─'.repeat(width - 2)}╯${RESET}`,
  ];
  return { x: Math.max(0, Math.floor((cols - width) / 2)), y: Math.max(0, Math.floor((rows - lines.length) / 2)), lines, maxScroll };
}
