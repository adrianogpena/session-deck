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
      if (w < width) {
        result = result.slice(0, -1) + '…';
      }
      break;
    }
    result += ch;
    w += cw;
  }
  return result + ' '.repeat(Math.max(0, width - w));
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

/** The visible screen of a headless terminal, cropped/padded to width x height, as ANSI lines. */
export function renderTerm(term: Terminal, width: number, height: number): string[] {
  const buf = term.buffer.active;
  const cell = buf.getNullCell();
  const lines: string[] = [];
  for (let y = 0; y < height; y++) {
    const line = y < term.rows ? buf.getLine(buf.baseY + y) : undefined;
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
        const sgr = cellSgr(cell);
        if (sgr !== lastSgr) {
          s += sgr;
          lastSgr = sgr;
        }
        if (col + w > width) {
          s += ' ';
          col += 1;
          break;
        }
        s += cell.getChars() || ' ';
        col += w;
      }
    }
    lines.push(`${s}${ESC}0m${' '.repeat(Math.max(0, width - col))}`);
  }
  return lines;
}
