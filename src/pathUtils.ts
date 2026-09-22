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
