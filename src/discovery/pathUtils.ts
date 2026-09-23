import * as os from 'os';
import * as path from 'path';

/** A case-insensitive-safe key for *comparing* filesystem paths on Windows — never for display. */
export function normalizeFsPath(fsPath: string): string {
  const resolved = path.resolve(fsPath);
  return process.platform === 'win32' ? resolved.toLowerCase() : resolved;
}

/** True when `candidate` resolves to `root` itself or somewhere inside it. */
export function isInside(root: string, candidate: string): boolean {
  const normalizedRoot = normalizeFsPath(root);
  const normalizedCandidate = normalizeFsPath(candidate);
  return normalizedCandidate === normalizedRoot || normalizedCandidate.startsWith(normalizedRoot + path.sep);
}

/** `~` is a shell convention, never expanded by Node/VS Code. Only a leading `~` is handled — no `~user` support. */
export function expandHome(root: string): string {
  if (root === '~') {
    return os.homedir();
  }
  if (root.startsWith('~/') || root.startsWith('~\\')) {
    return path.join(os.homedir(), root.slice(2));
  }
  return root;
}
