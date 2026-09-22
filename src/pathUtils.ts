import * as path from 'path';

/**
 * A case-insensitive-safe key for *comparing* filesystem paths on Windows —
 * never for display. Two paths that differ only by drive-letter or directory
 * casing (git's resolved root vs. a hand-typed `.vscode/session-deck.json`
 * entry, for instance) should still be treated as the same project.
 */
export function normalizeFsPath(fsPath: string): string {
  const resolved = path.resolve(fsPath);
  return process.platform === 'win32' ? resolved.toLowerCase() : resolved;
}
