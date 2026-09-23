import { execFile } from 'child_process';
import { promisify } from 'util';
import * as path from 'path';
import { normalizeFsPath } from './pathUtils';

const execFileAsync = promisify(execFile);

export interface ProjectRoot {
  /** Absolute path git (or the raw cwd, if it's not a git repo) considers this project's root. */
  root: string;
  isGitRepo: boolean;
}

const cache = new Map<string, Promise<ProjectRoot>>();

/**
 * Resolves a working directory to its project root, git-aware — `git rev-parse --git-common-dir`
 * consolidates worktrees and subfolders under one root. Falls back to the raw cwd when git is
 * missing or it's not a repo. Requires git >= 2.31 for `--path-format=absolute`.
 */
export function resolveProjectRoot(cwd: string): Promise<ProjectRoot> {
  // Case-insensitive on Windows: cwds differing only by casing are still the same directory.
  const key = normalizeFsPath(cwd);
  const cached = cache.get(key);
  if (cached) {
    return cached;
  }

  const promise = (async (): Promise<ProjectRoot> => {
    try {
      const { stdout } = await execFileAsync(
        'git',
        ['-C', cwd, 'rev-parse', '--path-format=absolute', '--git-common-dir'],
        { timeout: 5000 }
      );
      const gitDir = stdout.trim();
      return { root: path.dirname(gitDir), isGitRepo: true };
    } catch {
      return { root: cwd, isGitRepo: false };
    }
  })();

  cache.set(key, promise);
  return promise;
}

/** Call before a manual refresh so a folder that became/stopped being a git repo is picked up. */
export function clearProjectRootCache(): void {
  cache.clear();
}
