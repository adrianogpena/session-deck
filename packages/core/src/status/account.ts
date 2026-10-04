import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

/**
 * Claude Code's own global config file (not part of this repo) — read-only here, just to tell which
 * Claude.ai account is currently logged in, so usage data recorded under different accounts (e.g.
 * switching between a personal and a work login) isn't blended together.
 */
function claudeConfigPath(): string {
  // SESSION_DECK_CLAUDE_CONFIG_PATH: tests keep off the real ~/.claude.json.
  return process.env.SESSION_DECK_CLAUDE_CONFIG_PATH || path.join(os.homedir(), '.claude.json');
}

/**
 * The email of whichever Claude.ai account is currently logged in to Claude Code, or `undefined` if that
 * can't be determined (config missing/unparseable, or no account logged in yet). Keyed by email rather
 * than Claude Code's internal account uuid — email is the identifier a person actually thinks in terms of
 * ("my personal account" vs "my work account"), and it's the one field both sides of an account switch
 * reliably carry.
 */
export function currentAccountEmail(): string | undefined {
  try {
    const parsed: unknown = JSON.parse(fs.readFileSync(claudeConfigPath(), 'utf8'));
    if (!parsed || typeof parsed !== 'object') {
      return undefined;
    }
    const oauthAccount = (parsed as Record<string, unknown>).oauthAccount;
    if (!oauthAccount || typeof oauthAccount !== 'object') {
      return undefined;
    }
    const email = (oauthAccount as Record<string, unknown>).emailAddress;
    return typeof email === 'string' && email.length > 0 ? email : undefined;
  } catch {
    return undefined; // missing file, not logged in yet, or a transient partial write racing a read
  }
}
