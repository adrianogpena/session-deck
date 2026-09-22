import * as os from 'os';
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

/**
 * True when `candidate` resolves to `root` itself or somewhere inside it — the standard guard against a
 * path built from external/decoded input (a URI segment, a hook payload) escaping outside `root` via a
 * "../" or similar segment.
 */
export function isInside(root: string, candidate: string): boolean {
  const normalizedRoot = normalizeFsPath(root);
  const normalizedCandidate = normalizeFsPath(candidate);
  return normalizedCandidate === normalizedRoot || normalizedCandidate.startsWith(normalizedRoot + path.sep);
}

/**
 * `~` is a shell convention, never expanded by Node/VS Code — a `root` typed by hand as `~/foo`
 * in `.vscode/session-deck.json` would otherwise be treated as a literal, nonexistent path
 * (reported live: broke both "+ New Session" and matching real sessions back to their project).
 * Only a *leading* `~` is special (`~user` isn't supported — no cross-platform way to resolve
 * another user's home directory).
 */
export function expandHome(root: string): string {
  if (root === '~') {
    return os.homedir();
  }
  if (root.startsWith('~/') || root.startsWith('~\\')) {
    return path.join(os.homedir(), root.slice(2));
  }
  return root;
}
