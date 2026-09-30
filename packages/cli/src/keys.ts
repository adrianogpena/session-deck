/**
 * Ctrl+Q, in every encoding the real terminal may use while attached: plain (0x11), kitty CSI-u, and
 * Windows Terminal's win32-input-mode (`ESC[Vk;Sc;Uc;Kd;Cs;Rc_`, Vk 81 = Q, key-down, a Ctrl bit set
 * in Cs, no Alt). ConPTY itself asks for win32-input-mode (`ESC[?9001h`), and that request reaches
 * the real terminal while attached, so after that plain 0x11 never arrives.
 */
// eslint-disable-next-line no-control-regex -- matching terminal control sequences is the point
const DETACH_PATTERN =/\x11|\x1b\[113;5u|\x1b\[81;\d*;\d*;1;(\d+);\d*_/g;
/** Same encodings as {@link DETACH_PATTERN}, for Ctrl+K (0x0b, Vk 75 = K, kitty code point 107 = k). */
// eslint-disable-next-line no-control-regex -- matching terminal control sequences is the point
const CHORD_PATTERN = /\x0b|\x1b\[107;5u|\x1b\[75;\d*;\d*;1;(\d+);\d*_/g;
const CTRL_PRESSED = 0x04 | 0x08;
const ALT_PRESSED = 0x01 | 0x02;

function matchCtrlKey(data: string, pattern: RegExp): RegExpExecArray | null {
  const re = new RegExp(pattern);
  let m: RegExpExecArray | null;
  while ((m = re.exec(data))) {
    if (m[1] === undefined) {
      return m;
    }
    const state = Number(m[1]);
    if (state & CTRL_PRESSED && !(state & ALT_PRESSED)) {
      return m;
    }
  }
  return null;
}

/** Index where the detach key starts in `data`, or -1. */
export function findDetachKey(data: string): number {
  return matchCtrlKey(data, DETACH_PATTERN)?.index ?? -1;
}

/**
 * Where the Ctrl+K chord's prefix starts and ends in `data`, or null (resolved by a following `n`/`N`
 * for a new session, `t`/`T` to swap attach/interacting, or `q`/`Q` to stop the session — see
 * `onLiveInput`). `end` matters because a chord typed quickly can arrive with its resolving key
 * already in the same chunk, right after the match — unlike Ctrl+Q, this key needs to look past its
 * own match to find that out.
 */
export function findChordKey(data: string): { index: number; end: number } | null {
  const m = matchCtrlKey(data, CHORD_PATTERN);
  return m ? { index: m.index, end: m.index + m[0].length } : null;
}

/**
 * Index where an unmodified, key-down press of `ch` (a single ASCII letter) starts in `data`: the
 * plain character itself, or Windows Terminal's win32-input-mode encoding of it (`Uc` = its code
 * point, `Kd` = 1) — the same reason `findDetachKey` needs that encoding. -1 if not found.
 */
export function findPlainKey(data: string, ch: string): number {
  const code = ch.charCodeAt(0);
  const re = new RegExp(`${ch}|\\x1b\\[\\d+;\\d*;${code};1;\\d*;\\d*_`, 'g');
  const m = re.exec(data);
  return m ? m.index : -1;
}

/** Terminal modes an agent (or ConPTY) may switch on in the real terminal while attached. */
export const RESET_AGENT_MODES = '\x1b[?9001l\x1b[?2004l\x1b[?1004l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1l\x1b[<u\x1b[0m';

/**
 * Basic click/wheel reporting (mode 1000), SGR-encoded (mode 1006) so coordinates never collide with
 * printable bytes. A wheel notch then arrives as its own unambiguous sequence (see `onMouseSequence`)
 * instead of the arrow keys a terminal falls back to translating it into with no mouse mode on — which
 * a real arrow-key press also sends, and can't be told apart from. Off by default (`App.mouseTracking`)
 * so click-drag still does the terminal's own text selection; `m`/`Ctrl+K M` turns it on to scroll the
 * preview with the wheel instead. Always off while attached, or whenever input is forwarded to a live
 * agent (`RESET_AGENT_MODES` already turns it back off too).
 */
export const ENABLE_MOUSE = '\x1b[?1000h\x1b[?1006h';
export const DISABLE_MOUSE = '\x1b[?1000l\x1b[?1006l';

/** A wheel notch or click, reported because {@link ENABLE_MOUSE} is on: `ESC [ < Cb ; Cx ; Cy (M|m)`. */
// eslint-disable-next-line no-control-regex -- matching terminal control sequences is the point
const MOUSE_SEQUENCE = /^\x1b\[<(\d+);(\d+);(\d+)[Mm]$/;

/** Whether `data` is one SGR mouse report, and if it's a wheel notch, which way. `wheel` is undefined for a plain click/release (reported the same as a wheel notch would be, just consumed silently — sdeck has no use for clicks yet). */
export function parseMouseSequence(data: string): { wheel?: 'up' | 'down' } | undefined {
  const m = MOUSE_SEQUENCE.exec(data);
  if (!m) {
    return undefined;
  }
  const cb = Number(m[1]);
  if ((cb & 0x40) === 0) {
    return {}; // an ordinary button press/release, not the wheel
  }
  return { wheel: (cb & 1) === 0 ? 'up' : 'down' };
}

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
