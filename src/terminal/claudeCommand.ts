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
