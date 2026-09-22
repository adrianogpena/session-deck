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
 * Resolves a working directory to its project root, git-aware.
 *
 * `git rev-parse --git-common-dir` returns the *same* .git directory for a
 * repo's main worktree and every linked worktree (e.g. Claude Code's
 * `.claude/worktrees/<branch>` checkouts created by the EnterWorktree tool), and
 * for a plain subfolder it walks up to the containing repo on its own — so this
 * naturally consolidates worktrees and subfolders (like `.idea/localDebug`)
 * under one project root with no manual directory-walking needed on our side.
 * Falls back to the raw cwd (each becomes its own project) when git is missing
 * or the folder isn't a repo.
 *
 * Requires git >= 2.31 (2021) for `--path-format=absolute`.
 */
export function resolveProjectRoot(cwd: string): Promise<ProjectRoot> {
  // Keyed case-insensitively on Windows: two dirName folders whose recorded cwd
  // differs only by drive-letter/segment casing are still the same real directory.
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
