import type { Terminal } from '@xterm/headless';
import { fit, fitAnsi, renderTerm, textWidth, wrap } from './ansi';
import { STATUS_CATEGORIES, StatusCategory, TimeFilter } from './filters';
import { PANEL_HEADER_ROWS, Rect } from './layout';
import type { SessionStatus } from './sessions';
import { Role, Theme, ThemeName } from './theme';

const RESET = '\x1b[0m';
const BOLD = '\x1b[1m';
const UNDERLINE = '\x1b[4m';

const GLYPHS: Record<SessionStatus, { char: string; role: Role; bold: boolean }> = {
  running: { char: '●', role: 'green', bold: true },
  waiting: { char: '◐', role: 'yellow', bold: true },
  // Finished, not seen yet: waiting for a look, so it reads like "waiting".
  done: { char: '◐', role: 'yellow', bold: true },
  idle: { char: '○', role: 'textDim', bold: false },
  starting: { char: '⟳', role: 'yellow', bold: false },
  error: { char: '✕', role: 'red', bold: true },
  exited: { char: '✕', role: 'red', bold: true },
  stopped: { char: '■', role: 'textDim', bold: false },
};

const CATEGORY_GLYPHS: Record<StatusCategory, { char: string; role: Role }> = {
  running: { char: '●', role: 'green' },
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
}

interface GroupCounts {
  count: number;
  running: number;
  waiting: number;
}

export type ListRow =
  | ({ kind: 'folder'; name: string; collapsed: boolean; hotkey?: number } & GroupCounts)
  | ({ kind: 'project'; label: string; collapsed: boolean; depth: number; hotkey?: number } & GroupCounts)
  | { kind: 'session'; view: SessionView; isLast: boolean; depth: number; pin?: 'top' | 'bottom' }
  | { kind: 'divider'; label: string };

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

export function renderHeader(t: Theme, cols: number, counts: Record<StatusCategory, number>, liveCount: number, themeLabel: string, version: string): string {
  const dot = (char: string, active: boolean, role: Role) => `${t.fg(active ? role : 'textDim')}${char}`;
  const logo = `${t.fg('border')}⟨${dot('●', counts.running > 0, 'green')}${t.fg('border')}│${dot('◐', counts.waiting > 0, 'yellow')}${t.fg('border')}│${dot('○', counts.idle > 0, 'text')}${t.fg('border')}⟩`;
  const leftPlain = ' ⟨●│◐│○⟩ Session Deck';
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
  statusFilter: ReadonlySet<StatusCategory>,
  timeFilter: TimeFilter
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
  }
  const time = TIME_LABEL[timeFilter];
  out += `${t.fg('border')}│${RESET}` + pill(time, timeFilter !== 'all', t.fg('purple'));
  plainWidth += 1 + textWidth(time) + 2;
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

/** Folder and project rows: `1▾ name (n) ●r ◐w`. The leading column shows the 1–9 jump key on top-level rows. */
function renderGroupRow(t: Theme, width: number, row: Extract<ListRow, { kind: 'folder' | 'project' }>, selected: boolean): string {
  const depth = row.kind === 'project' ? row.depth : 0;
  const hotkey = row.hotkey === undefined ? ' ' : String(row.hotkey);
  const arrow = row.collapsed ? '▸' : '▾';
  const lead = `${hotkey}${'  '.repeat(depth)}${arrow} `;
  const name = row.kind === 'folder' ? row.name : row.label;
  const counts = `${row.running ? ` ●${row.running}` : ''}${row.waiting ? ` ◐${row.waiting}` : ''}`;
  const suffix = ` (${row.count})${counts}`;
  const label = fit(name, Math.max(1, width - textWidth(lead) - textWidth(suffix))).trimEnd();
  const pad = blank(width - textWidth(lead) - textWidth(label) - textWidth(suffix));
  if (selected) {
    const sel = `${t.bg('accent')}${t.fg('bg')}`;
    return `${sel}${lead}${BOLD}${label}${RESET}${sel}${suffix}${pad}${RESET}`;
  }
  const nameRole: Role = row.kind === 'folder' ? 'purple' : 'cyan';
  const running = row.running ? ` ${t.fg('green')}●${row.running}` : '';
  const waiting = row.waiting ? ` ${t.fg('yellow')}◐${row.waiting}` : '';
  return (
    `${t.fg('textDim')}${hotkey}${t.fg('text')}${'  '.repeat(depth)}${arrow} ` +
    `${BOLD}${t.fg(nameRole)}${label}${RESET}${t.fg('text')} (${row.count})${running}${waiting}${RESET}${pad}`
  );
}

function renderSessionRow(t: Theme, width: number, row: Extract<ListRow, { kind: 'session' }>, selected: boolean): string {
  const v = row.view;
  const indent = '  '.repeat(row.depth);
  const connector = row.isLast ? '└─' : '├─';
  // Narrow glyphs only: emoji are one column wide in some terminals and two in others.
  const pinMark = row.pin === 'top' ? '↑ ' : row.pin === 'bottom' ? '↓ ' : '';
  const markers = `${pinMark}${v.elsewhere ? '↗ ' : ''}`;
  const leftPlain = `${indent}${connector} ${GLYPHS[v.status].char} ${markers}`;
  // Right-hand columns shrink away on narrow lists: first the agent, then the time.
  const right = width >= 44 ? ` ${v.agent} ${v.timeLabel} ` : width >= 34 ? ` ${v.timeLabel} ` : ' ';
  const titleWidth = Math.max(1, width - textWidth(leftPlain) - textWidth(right));
  const title = fit(v.title, titleWidth);
  const active = v.status === 'running' || v.status === 'waiting';

  if (selected) {
    const sel = `${t.bg('accent')}${t.fg('bg')}`;
    return `${sel}${leftPlain}${BOLD}${title}${RESET}${sel}${right}${RESET}`;
  }
  const titleStyle = `${active ? BOLD : ''}${v.status === 'exited' ? UNDERLINE : ''}${t.fg('text')}`;
  const agentPart = width >= 44 ? ` ${t.fg(AGENT_ROLE[v.agent] ?? 'text')}${v.agent} ${t.fg('textDim')}${v.timeLabel} ` : `${t.fg('textDim')}${right}`;
  return (
    `${indent}${t.fg('border')}${connector}${RESET} ${glyph(t, v.status)} ${pinMark ? `${t.fg('accent')}${pinMark}` : ''}${v.elsewhere ? `${t.fg('purple')}↗ ` : ''}${RESET}` +
    `${titleStyle}${title}${RESET}${agentPart}${RESET}`
  );
}

function renderDividerRow(t: Theme, width: number, label: string): string {
  const text = fit(` ${label} `, Math.max(0, width - 2));
  return `${t.fg('border')}──${t.fg('textDim')}${text}${t.fg('border')}${'─'.repeat(Math.max(0, width - 2 - textWidth(text)))}${RESET}`;
}

/** Lines for the sessions panel, exactly `rect.width` columns each, `rect.height` lines. */
export function renderListPanel(t: Theme, rect: Rect, rows: ListRow[], selected: number, note: string, emptyMessage: string): string[] {
  const lines = panelHeader(t, rect.width, 'SESSIONS', note);
  const height = rect.height - PANEL_HEADER_ROWS;
  if (rows.length === 0) {
    lines.push(blank(rect.width), `${t.fg('textDim')}${fit(`  ${emptyMessage}`, rect.width)}${RESET}`);
  }
  // Keep the selection in view, roughly centered.
  const start = Math.max(0, Math.min(selected - Math.floor(height / 2), rows.length - height));
  for (let i = 0; i < height && lines.length < rect.height; i++) {
    const row = rows[start + i];
    const isSelected = start + i === selected;
    if (!row) {
      lines.push(blank(rect.width));
    } else if (row.kind === 'session') {
      lines.push(renderSessionRow(t, rect.width, row, isSelected));
    } else if (row.kind === 'divider') {
      lines.push(renderDividerRow(t, rect.width, row.label));
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
      : `${key('Enter')} collapse/expand · ${key('M')} move to folder · ${key('K J')} reorder · ${key('n')} new session`;
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
  exitCode?: number;
  lastResponse?: string | null;
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
  const meta = ` ${statusText} · ${v.agent} · ${v.timeLabel} · ${v.cwd} `;
  const metaFit = fit(meta, Math.max(0, rect.width - 2)).trimEnd();
  const metaLine = `${t.fg('border')}─${t.fg('textDim')}${metaFit}${t.fg('border')}${'─'.repeat(Math.max(0, rect.width - 1 - textWidth(metaFit)))}${RESET}`;
  const bodyHeight = rect.height - PANEL_HEADER_ROWS;

  if (content.term) {
    return [titleLine, metaLine, ...renderTerm(content.term, rect.width, bodyHeight)];
  }

  const key = (k: string) => `${BOLD}${t.fg('accent')}${k}${RESET}${t.fg('textDim')}`;
  const action = v.elsewhere
    ? `${t.fg('yellow')}Open in another terminal. Close it there to open it here.`
    : v.status === 'exited'
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
  [['↑↓', 'select'], ['⏎', 'attach'], ['n', 'new'], ['e', 'rename'], ['x', 'stop'], ['g', 'folder'], ['M', 'move'], ['K J', 'reorder'], [',', 'pin'], ['?', 'help'], ['q', 'quit']],
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
  return `${BOLD}${t.fg('accent')}${prefix}${RESET}${t.fg('text')}${fit(`${value}█`, Math.max(1, cols - textWidth(prefix)))}${RESET}`;
}

const HELP_SECTIONS: { title: string; keys: Hint[] }[] = [
  {
    title: 'QUICK START',
    keys: [
      ['Enter', 'Attach full-screen (starts it if needed)'],
      ['Ctrl+Q', 'Detach back here; the session keeps running'],
    ],
  },
  {
    title: 'NAVIGATION',
    keys: [
      ['↑ ↓  j k', 'Select (preview follows)'],
      ['← →  Tab', 'Collapse / expand; ← also goes to the parent'],
      ['1-9', 'Jump to a top-level folder or project'],
      ['`', 'Back to the previously selected session'],
    ],
  },
  {
    title: 'SESSIONS',
    keys: [
      ['s', 'Start in the background'],
      ['n', 'New session in the selected project'],
      ['e  F2', "Rename (same as Claude's /rename)"],
      ['x', 'Stop the selected session'],
      ['u', 'Mark as unread (finished, not seen)'],
      [',', 'Pin: top · bottom · off'],
      ['r', 'Refresh the list'],
    ],
  },
  {
    title: 'FOLDERS & ORDER',
    keys: [
      ['g', 'New folder'],
      ['M', 'Move the project to a folder'],
      ['K J', 'Move the folder / project up or down'],
      ['e  d', 'Rename / delete the selected folder'],
      ['S', 'Sort sessions: recent · actionable'],
      ['t', 'View: normal · active on top'],
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
      ['b  Ctrl+B', 'Hide / show the sessions panel'],
      ['T', 'Theme: dark · light · system'],
    ],
  },
  { title: 'OTHER', keys: [['?', 'This help'], ['q  Ctrl+C', 'Quit (stops background sessions)']] },
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
