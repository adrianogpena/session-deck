import { execFile } from 'child_process';
import { promisify } from 'util';

const execFileAsync = promisify(execFile);

export interface GitStatus {
  /** Undefined when detached HEAD (or the branch name couldn't be read). */
  branch?: string;
  /** Commits ahead of the upstream branch. 0 when there's no upstream. */
  ahead: number;
  /** Commits behind the upstream branch. 0 when there's no upstream. */
  behind: number;
  /** Changed + untracked entries. */
  dirty: number;
}

/**
 * Parses `git status --porcelain=v2 --branch --ahead-behind` output. A stable, documented format
 * (unlike Codex/Copilot's own session files), so this is a real contract, not best-effort.
 */
export function parseGitStatusPorcelain(output: string): GitStatus {
  let branch: string | undefined;
  let ahead = 0;
  let behind = 0;
  let dirty = 0;
  for (const line of output.split('\n')) {
    if (!line) {
      continue;
    }
    if (line.startsWith('# branch.head ')) {
      const name = line.slice('# branch.head '.length).trim();
      branch = name === '(detached)' ? undefined : name;
    } else if (line.startsWith('# branch.ab ')) {
      const m = /^# branch\.ab \+(\d+) -(\d+)$/.exec(line);
      if (m) {
        ahead = Number(m[1]);
        behind = Number(m[2]);
      }
    } else if (!line.startsWith('#')) {
      dirty++;
    }
  }
  return { branch, ahead, behind, dirty };
}

/** `undefined` for anything that isn't a clean read: not a repo, git missing, or a transient error — callers just show nothing. */
export async function readGitStatus(cwd: string): Promise<GitStatus | undefined> {
  try {
    // --no-optional-locks: never blocks on (or trips) a lock the agent's own git commands are holding.
    const { stdout } = await execFileAsync('git', ['-C', cwd, '--no-optional-locks', 'status', '--porcelain=v2', '--branch', '--ahead-behind'], {
      timeout: 5000,
    });
    return parseGitStatusPorcelain(stdout);
  } catch {
    return undefined;
  }
}
