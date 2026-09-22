import { assertSafeSessionId } from './sessionId';

export function buildClaudeResumeCommand(sessionId: string, dangerouslySkipPermissions: boolean): string {
  const args = ['claude'];
  if (dangerouslySkipPermissions) {
    args.push('--dangerously-skip-permissions');
  }
  args.push('--resume', assertSafeSessionId(sessionId));
  return args.join(' ');
}

export function buildClaudeNewSessionCommand(dangerouslySkipPermissions: boolean): string {
  return dangerouslySkipPermissions ? 'claude --dangerously-skip-permissions' : 'claude';
}

/**
 * `--fork-session` (confirmed via `claude --help`: "When resuming, create a new session ID instead
 * of reusing the original") resumes `sessionId`'s full history but writes every new turn under a
 * brand-new session id, leaving the original session completely untouched — Claude Code's own
 * native fork. No equivalent flag exists in `copilot --help` (`copilot sessions --help` only has
 * `import`), so this is Claude-only. Like a plain new session, there's no flag to pre-assign the
 * forked session's new id (`--session-id` isn't documented as composable with `--fork-session`, and
 * this wasn't verified live to avoid spending real API/credit usage just to test it), so its real id
 * is learned the same way `startNewClaudeSession` learns a new session's id: `correlateNewSession`
 * polling `~/.claude/projects` for the next new `.jsonl` file to appear.
 */
export function buildClaudeForkCommand(sessionId: string, dangerouslySkipPermissions: boolean): string {
  const args = ['claude'];
  if (dangerouslySkipPermissions) {
    args.push('--dangerously-skip-permissions');
  }
  args.push('--resume', assertSafeSessionId(sessionId), '--fork-session');
  return args.join(' ');
}
