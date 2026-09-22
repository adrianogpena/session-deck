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
