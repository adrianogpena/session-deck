/**
 * Ctrl+Q, in every encoding the real terminal may use while attached: plain (0x11), kitty CSI-u, and
 * Windows Terminal's win32-input-mode (`ESC[Vk;Sc;Uc;Kd;Cs;Rc_`, Vk 81 = Q, key-down, a Ctrl bit set
 * in Cs, no Alt). ConPTY itself asks for win32-input-mode (`ESC[?9001h`), and that request reaches
 * the real terminal while attached, so after that plain 0x11 never arrives.
 */
// eslint-disable-next-line no-control-regex -- matching terminal control sequences is the point
const DETACH_PATTERN =/\x11|\x1b\[113;5u|\x1b\[81;\d*;\d*;1;(\d+);\d*_/g;
const CTRL_PRESSED = 0x04 | 0x08;
const ALT_PRESSED = 0x01 | 0x02;

/** Index where the detach key starts in `data`, or -1. */
export function findDetachKey(data: string): number {
  const re = new RegExp(DETACH_PATTERN);
  let m: RegExpExecArray | null;
  while ((m = re.exec(data))) {
    if (m[1] === undefined) {
      return m.index;
    }
    const state = Number(m[1]);
    if (state & CTRL_PRESSED && !(state & ALT_PRESSED)) {
      return m.index;
    }
  }
  return -1;
}

/** Terminal modes an agent (or ConPTY) may switch on in the real terminal while attached. */
export const RESET_AGENT_MODES = '\x1b[?9001l\x1b[?2004l\x1b[?1004l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1l\x1b[<u\x1b[0m';

/**
 * One input chunk as individual keys: fast typing, key repeat and pastes arrive as several keys at
 * once. CSI (`ESC [ … final`) and SS3 (`ESC O x`) sequences stay whole, e.g. arrows and F2.
 */
export function splitKeys(data: string): string[] {
  const keys: string[] = [];
  const chars = Array.from(data);
  for (let i = 0; i < chars.length; i++) {
    if (chars[i] !== '\x1b' || i + 1 >= chars.length) {
      keys.push(chars[i]);
    } else if (chars[i + 1] === 'O' && i + 2 < chars.length) {
      keys.push(chars.slice(i, i + 3).join(''));
      i += 2;
    } else if (chars[i + 1] === '[') {
      let end = i + 2;
      while (end < chars.length && !/[\x40-\x7e]/.test(chars[end])) {
        end++;
      }
      keys.push(chars.slice(i, end + 1).join(''));
      i = end;
    } else {
      keys.push('\x1b'); // lone Esc (or Alt+key, which we don't bind)
    }
  }
  return keys;
}
