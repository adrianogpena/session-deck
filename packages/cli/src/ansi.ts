import type { IBufferCell, Terminal } from '@xterm/headless';

export const ESC = '\x1b[';

function charWidth(cp: number): number {
  if (cp < 0x20) {
    return 0;
  }
  const wide =
    cp >= 0x1100 &&
    (cp <= 0x115f ||
      (cp >= 0x2e80 && cp <= 0xa4cf) ||
      (cp >= 0xac00 && cp <= 0xd7a3) ||
      (cp >= 0xf900 && cp <= 0xfaff) ||
      (cp >= 0xfe30 && cp <= 0xfe4f) ||
      (cp >= 0xff00 && cp <= 0xff60) ||
      (cp >= 0x1f300 && cp <= 0x1faff));
  return wide ? 2 : 1;
}

/** Truncates (with `…`) or pads plain text to exactly `width` columns. */
export function fit(text: string, width: number): string {
  let result = '';
  let w = 0;
  for (const ch of text) {
    const cw = charWidth(ch.codePointAt(0) ?? 0);
    if (cw === 0) {
      continue;
    }
    if (w + cw > width) {
      // Make room for the ellipsis, measuring in columns (a wide character frees two).
      const kept = Array.from(result);
      while (kept.length && w + 1 > width) {
        w -= charWidth(kept.pop()!.codePointAt(0) ?? 0);
      }
      result = kept.join('');
      if (w + 1 <= width) {
        result += '…';
        w += 1;
      }
      break;
    }
    result += ch;
    w += cw;
  }
  return result + ' '.repeat(Math.max(0, width - w));
}

/** Truncates (with a leading `…`) or leaves plain text as-is, keeping the *tail* within `width` columns. Used where the end of the text (e.g. a trailing cursor) must stay visible. */
export function fitTail(text: string, width: number): string {
  const chars = Array.from(text);
  let w = 0;
  let start = chars.length;
  for (let i = chars.length - 1; i >= 0; i--) {
    const cw = charWidth(chars[i].codePointAt(0) ?? 0);
    if (w + cw > width) {
      break;
    }
    w += cw;
    start = i;
  }
  if (start === 0) {
    return chars.join('');
  }
  const kept = chars.slice(start);
  while (kept.length && w + 1 > width) {
    w -= charWidth(kept.shift()!.codePointAt(0) ?? 0);
  }
  return '…' + kept.join('');
}

export function oneLine(text: string): string {
  return String(text).replace(/\s+/g, ' ').trim();
}

export function wrap(text: string, width: number): string[] {
  const lines: string[] = [];
  for (const para of String(text).split(/\r?\n/)) {
    let line = '';
    for (const word of para.split(' ')) {
      if ((line + ' ' + word).trim().length > width) {
        if (line) {
          lines.push(line);
        }
        line = word.length > width ? word.slice(0, width) : word;
      } else {
        line = line ? `${line} ${word}` : word;
      }
    }
    lines.push(line);
  }
  return lines;
}

function colorCode(isRgb: boolean, isPalette: boolean, color: number, base: number, brightBase: number, ext: number): string {
  if (isRgb) {
    return `${ext};2;${(color >> 16) & 255};${(color >> 8) & 255};${color & 255}`;
  }
  if (isPalette) {
    return color < 8 ? `${base + color}` : color < 16 ? `${brightBase + color - 8}` : `${ext};5;${color}`;
  }
  return '';
}

function cellSgr(c: IBufferCell): string {
  const p = ['0'];
  if (c.isBold()) p.push('1');
  if (c.isDim()) p.push('2');
  if (c.isItalic()) p.push('3');
  if (c.isUnderline()) p.push('4');
  if (c.isInverse()) p.push('7');
  const fg = colorCode(!!c.isFgRGB(), !!c.isFgPalette(), c.getFgColor(), 30, 90, 38);
  const bg = colorCode(!!c.isBgRGB(), !!c.isBgPalette(), c.getBgColor(), 40, 100, 48);
  if (fg) p.push(fg);
  if (bg) p.push(bg);
  return `${ESC}${p.join(';')}m`;
}

type CursorShape = 'block' | 'underline' | 'bar';

/**
 * The child's current cursor shape (DECSCUSR), the same thing an attached terminal honors natively —
 * reading it here, rather than hardcoding a shape, is what keeps the snapshot cursor in sync with the
 * real one whichever one is changed. `decPrivateModes` isn't part of the public API, but it's the only
 * way to see a DECSCUSR override; falling back to the terminal's own (public) default cursorStyle option.
 */
function cursorShape(term: Terminal): CursorShape {
  const override = (term as unknown as { _core?: { coreService?: { decPrivateModes?: { cursorStyle?: CursorShape } } } })._core?.coreService
    ?.decPrivateModes?.cursorStyle;
  return override ?? term.options.cursorStyle ?? 'block';
}

/**
 * The visible screen of a headless terminal, cropped/padded to width x height, as ANSI lines.
 *
 * This is a plain-text snapshot, not a live passthrough, so the child's own blinking terminal
 * cursor (which it positions via escape codes rather than printing a visible glyph) never makes it
 * into the snapshot on its own — we paint it in ourselves, in whatever shape (block/underline/bar)
 * the child last requested, mirroring what an attached terminal would show. `showCursor` is false
 * whenever this session isn't the one actually receiving keystrokes, so a merely-previewed session
 * reads at a glance as not-in-focus.
 *
 * `scrollOffset` reads further back into the terminal's own scrollback instead of the live bottom
 * (0 = live), for the preview's own scroll — independent of the terminal's real viewport, which
 * stays pinned to the bottom throughout (this is a read-only snapshot, not `term.scrollLines`, so
 * an attach mid-scroll still resumes at the live bottom). The cursor is only ever drawn at 0: at
 * any other offset it isn't part of what's on screen.
 */
export function renderTerm(term: Terminal, width: number, height: number, showCursor: boolean, scrollOffset = 0): string[] {
  const buf = term.buffer.active;
  const cell = buf.getNullCell();
  const topY = Math.max(0, buf.baseY - scrollOffset);
  // Not part of the public API, but it's the only way to know the child hid its cursor (e.g. mid-render).
  const cursorHidden = !!(term as unknown as { _core?: { coreService?: { isCursorHidden?: boolean } } })._core?.coreService?.isCursorHidden;
  const shape = showCursor && scrollOffset === 0 && !cursorHidden ? cursorShape(term) : undefined;
  const lines: string[] = [];
  for (let y = 0; y < height; y++) {
    const line = y < term.rows ? buf.getLine(topY + y) : undefined;
    let s = '';
    let lastSgr = '';
    let col = 0;
    if (line) {
      for (let x = 0; x < term.cols && col < width; x++) {
        line.getCell(x, cell);
        const w = cell.getWidth();
        if (w === 0) {
          continue;
        }
        const atCursor = shape !== undefined && y === buf.cursorY && x === buf.cursorX;
        const sgr = atCursor ? `${cellSgr(cell)}${ESC}${shape === 'underline' ? 4 : 7}m` : cellSgr(cell);
        if (sgr !== lastSgr) {
          s += sgr;
          lastSgr = sgr;
        }
        if (col + w > width) {
          s += ' ';
          col += 1;
          break;
        }
        s += atCursor && shape === 'bar' ? '▏' : cell.getChars() || ' ';
        col += w;
      }
    }
    lines.push(`${s}${ESC}0m${' '.repeat(Math.max(0, width - col))}`);
  }
  return lines;
}

/** Display width in terminal columns of plain text (no escape sequences). */
export function textWidth(text: string): number {
  let w = 0;
  for (const ch of text) {
    w += charWidth(ch.codePointAt(0) ?? 0);
  }
  return w;
}

/** Like {@link fit}, for a line that contains SGR color sequences: truncates or pads to `width` visible columns, keeping the sequences. */
export function fitAnsi(line: string, width: number): string {
  let out = '';
  let w = 0;
  // eslint-disable-next-line no-control-regex -- splitting SGR sequences from text is the point
  for (const part of line.split(/(\x1b\[[0-9;]*m)/)) {
    if (part.startsWith('\x1b[')) {
      out += part;
      continue;
    }
    for (const ch of part) {
      const cw = charWidth(ch.codePointAt(0) ?? 0);
      if (w + cw > width) {
        return `${out}${ESC}0m${' '.repeat(Math.max(0, width - w))}`;
      }
      out += ch;
      w += cw;
    }
  }
  return out + ' '.repeat(Math.max(0, width - w));
}
