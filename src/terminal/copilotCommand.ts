import { assertSafeSessionId } from './sessionId';

/**
 * `copilot --resume=<id>`, confirmed via `copilot --help`'s own examples. `--allow-all` is the CLI's
 * "skip every permission prompt" flag (equivalent to `--allow-all-tools --allow-all-paths
 * --allow-all-urls`; `--yolo` is a documented alias for the same thing) — the Copilot analogue of
 * Claude's `--dangerously-skip-permissions`, gated behind the same confirmation dialog.
 */
export function buildCopilotResumeCommand(sessionId: string, dangerouslySkipPermissions: boolean): string {
  const args = ['copilot', `--resume=${assertSafeSessionId(sessionId)}`];
  if (dangerouslySkipPermissions) {
    args.push('--allow-all');
  }
  return args.join(' ');
}

/**
 * Unlike Claude Code, Copilot CLI can pre-assign a brand-new session's UUID via `--session-id`
 * (confirmed via `copilot --help`'s own examples: "Start a new session with a specific UUID"). This
 * lets `terminalService.ts` track a new Copilot session under its real session id from the moment its
 * terminal is created — no polling/correlation step needed at all, unlike `correlateNewSession`, which
 * exists only because Claude Code has no equivalent flag.
 */
export function buildCopilotNewSessionCommand(sessionId: string, dangerouslySkipPermissions: boolean): string {
  const args = ['copilot', `--session-id=${assertSafeSessionId(sessionId)}`];
  if (dangerouslySkipPermissions) {
    args.push('--allow-all');
  }
  return args.join(' ');
}
