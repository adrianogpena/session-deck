import { assertSafeSessionId } from './sessionId';

/** `copilot --resume=<id>`. `--allow-all` skips every permission prompt — Copilot's analogue of Claude's `--dangerously-skip-permissions`. */
export function buildCopilotResumeCommand(sessionId: string, dangerouslySkipPermissions: boolean): string {
  const args = ['copilot', `--resume=${assertSafeSessionId(sessionId)}`];
  if (dangerouslySkipPermissions) {
    args.push('--allow-all');
  }
  return args.join(' ');
}

/** Unlike Claude Code, Copilot CLI can pre-assign a new session's UUID via `--session-id` — lets `terminalService.ts` track it under its real id immediately, no polling needed. */
export function buildCopilotNewSessionCommand(sessionId: string, dangerouslySkipPermissions: boolean): string {
  const args = ['copilot', `--session-id=${assertSafeSessionId(sessionId)}`];
  if (dangerouslySkipPermissions) {
    args.push('--allow-all');
  }
  return args.join(' ');
}
