import type { Terminal } from '@xterm/headless';

/**
 * Errors only visible on the agent's screen: Claude's status files say nothing about a failed
 * sign-in, the session just sits there. Checked against the last lines, so a message that scrolled
 * away (e.g. after a successful /login) no longer counts.
 */
const SCREEN_ERRORS: { pattern: RegExp; label: string }[] = [
  { pattern: /API Error: 401|authentication_error|OAuth token (has )?expired|Invalid API key/i, label: 'sign-in failed · run /login' },
  { pattern: /Please run \/login/i, label: 'signed out · run /login' },
  { pattern: /API Error: (429|5\d\d)|overloaded_error|rate_limit_error/i, label: 'API error · retrying or rate-limited' },
];

export const SCREEN_ERROR_LINES = 25;

/** The error the given screen lines show, if any: the first matching pattern wins. */
export function detectScreenError(lines: readonly string[]): string | undefined {
  const text = lines.slice(-SCREEN_ERROR_LINES).join('\n');
  return SCREEN_ERRORS.find((e) => e.pattern.test(text))?.label;
}

/** The visible screen of a headless terminal as plain text lines. */
export function screenLines(term: Terminal): string[] {
  const buf = term.buffer.active;
  const lines: string[] = [];
  for (let y = 0; y < term.rows; y++) {
    lines.push(buf.getLine(buf.baseY + y)?.translateToString(true) ?? '');
  }
  return lines;
}
