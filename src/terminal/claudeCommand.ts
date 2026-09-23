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

/** `--fork-session` resumes `sessionId`'s history under a brand-new session id, leaving the original untouched. Claude-only — no equivalent in Copilot CLI. */
export function buildClaudeForkCommand(sessionId: string, dangerouslySkipPermissions: boolean): string {
  const args = ['claude'];
  if (dangerouslySkipPermissions) {
    args.push('--dangerously-skip-permissions');
  }
  args.push('--resume', assertSafeSessionId(sessionId), '--fork-session');
  return args.join(' ');
}
