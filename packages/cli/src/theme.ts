import { execFile } from 'child_process';

export type ThemeName = 'dark' | 'light';

/** Tokyo Night (dark) and Tokyo Night Light, the palette agent-deck uses. */
const PALETTES = {
  dark: {
    bg: '#1a1b26',
    surface: '#24283b',
    border: '#414868',
    text: '#c0caf5',
    textDim: '#787fa0',
    accent: '#7aa2f7',
    purple: '#bb9af7',
    cyan: '#7dcfff',
    green: '#9ece6a',
    yellow: '#e0af68',
    orange: '#ff9e64',
    red: '#f7768e',
  },
  light: {
    bg: '#d5d6db',
    surface: '#e9e9ec',
    border: '#9699a3',
    text: '#343b58',
    textDim: '#6a6d7c',
    accent: '#34548a',
    purple: '#7847bd',
    cyan: '#166775',
    green: '#485e30',
    yellow: '#8f5e15',
    orange: '#965027',
    red: '#8c4351',
  },
} as const;

export type Role = keyof (typeof PALETTES)['dark'];

function rgb(hex: string): string {
  const n = parseInt(hex.slice(1), 16);
  return `${(n >> 16) & 255};${(n >> 8) & 255};${n & 255}`;
}

/** Truecolor SGR sequences for one palette. Styles are open-ended: callers reset with `\x1b[0m`. */
export class Theme {
  constructor(readonly name: ThemeName) {}

  fg(role: Role): string {
    return `\x1b[38;2;${rgb(PALETTES[this.name][role])}m`;
  }

  bg(role: Role): string {
    return `\x1b[48;2;${rgb(PALETTES[this.name][role])}m`;
  }
}

/** Reply to an OSC 11 ("what's your background color?") query: `ESC]11;rgb:RRRR/GGGG/BBBB` ended by BEL or ST. */
// eslint-disable-next-line no-control-regex -- matching a terminal reply is the point
const OSC11_REPLY = /\x1b\]11;rgb:([0-9a-f]+)\/([0-9a-f]+)\/([0-9a-f]+)(?:\x07|\x1b\\)/gi;
export const OSC11_QUERY = '\x1b]11;?\x07';

/** Pulls OSC 11 replies out of an input chunk: the theme the last one implies, and the input without them. */
export function extractBackgroundReply(data: string): { theme?: ThemeName; rest: string } {
  let theme: ThemeName | undefined;
  const rest = data.replace(OSC11_REPLY, (_match, r: string, g: string, b: string) => {
    const channel = (hex: string) => parseInt(hex, 16) / (16 ** hex.length - 1);
    const luminance = 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
    theme = luminance < 0.5 ? 'dark' : 'light';
    return '';
  });
  return { theme, rest };
}

/** The OS app theme (Windows registry, macOS defaults), or `undefined` where there's no such setting. */
export function readOsTheme(): Promise<ThemeName | undefined> {
  return new Promise((resolve) => {
    if (process.platform === 'win32') {
      execFile('reg', ['query', 'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize', '/v', 'AppsUseLightTheme'], (err, stdout) => {
        const m = /AppsUseLightTheme\s+REG_DWORD\s+0x([0-9a-f]+)/i.exec(stdout ?? '');
        resolve(err || !m ? undefined : parseInt(m[1], 16) === 0 ? 'dark' : 'light');
      });
    } else if (process.platform === 'darwin') {
      // Prints "Dark" in dark mode; fails (no such key) in light mode.
      execFile('defaults', ['read', '-g', 'AppleInterfaceStyle'], (err, stdout) => resolve(!err && /dark/i.test(stdout) ? 'dark' : 'light'));
    } else {
      resolve(undefined);
    }
  });
}
