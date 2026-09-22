import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';

/**
 * Standalone script invoked as a Claude Code hook (never imported by the
 * extension itself — it runs as a separate `node` process spawned by the
 * `claude` CLI, outside the extension host, receiving the hook's JSON payload
 * on stdin per Claude Code's hook protocol). Deliberately has no dependency on
 * `vscode` or anything else in this project so it keeps working exactly as
 * configured even if the rest of the extension changes shape.
 *
 * Maps a subset of hook events to a coarse session status, written to
 * `~/.claude/session-deck-status/<sessionId>.json` for the extension to read
 * and turn into a colored dot in the tree (`sessionStatus.ts`). Never exits
 * non-zero and never prints anything that could be mistaken for hook output
 * that blocks the tool/turn — this is a passive observer only.
 */

type SessionStatus = 'running' | 'waiting' | 'done';

type Action = { kind: 'set'; status: SessionStatus } | { kind: 'clear' };

interface HookPayload {
  hook_event_name?: string;
  session_id?: string;
  notification_type?: string;
}

function statusDir(): string {
  return path.join(os.homedir(), '.claude', 'session-deck-status');
}

/**
 * `session_id` is external input to this process (a hook payload from the `claude` CLI on stdin), used
 * below to build the status file's path — Claude Code itself always sends a plain UUID, but this is the
 * boundary that actually matters, so anything else is rejected outright rather than trusted to be safe.
 */
const SAFE_SESSION_ID = /^[A-Za-z0-9-]+$/;

/**
 * Only these `notification_type`s represent something genuinely needing
 * *your* action. `claudeSettings.ts` already configures the `Notification`
 * hook's `matcher` to this same set, so Claude Code shouldn't invoke this
 * script for anything else (notably `idle_prompt`, the automatic "you've been
 * idle a while" nudge) — this is a defensive second check in case that
 * matcher isn't honored by some Claude Code version, since an unfiltered
 * `idle_prompt` firing asynchronously on its own timer could otherwise land
 * *after* a `UserPromptSubmit` write and incorrectly flip "running" back to
 * "waiting".
 */
const ACTIONABLE_NOTIFICATION_TYPES = new Set([
  'permission_prompt',
  'agent_needs_input',
  'elicitation_dialog',
  'elicitation_url_dialog',
]);

/**
 * `Stop` fires on every ordinary turn completion — that's the everyday "the
 * task finished" moment, so it's "done" (green), not "waiting". `Notification`
 * (filtered to the actionable types above) is the real "waiting for input"
 * (yellow) — Claude blocked on something only you can resolve. `SessionEnd`
 * (the `claude` process actually exiting) clears the status file entirely
 * rather than setting a state: once the session is gone there's nothing left
 * to reflect a color for, so it reverts to the plain default icon ("no
 * status") instead of staying green forever.
 */
function actionForPayload(payload: HookPayload): Action | undefined {
  switch (payload.hook_event_name) {
    case 'UserPromptSubmit':
      return { kind: 'set', status: 'running' };
    case 'Stop':
      return { kind: 'set', status: 'done' };
    case 'Notification':
      return payload.notification_type && ACTIONABLE_NOTIFICATION_TYPES.has(payload.notification_type)
        ? { kind: 'set', status: 'waiting' }
        : undefined;
    case 'SessionEnd':
      return { kind: 'clear' };
    default:
      return undefined;
  }
}

function main(): void {
  let raw = '';
  process.stdin.setEncoding('utf8');
  process.stdin.on('data', (chunk) => {
    raw += chunk;
  });
  process.stdin.on('end', () => {
    try {
      const payload = JSON.parse(raw) as HookPayload;
      const action = actionForPayload(payload);
      const sessionId = payload.session_id;
      if (!action || !sessionId || !SAFE_SESSION_ID.test(sessionId)) {
        return;
      }

      const filePath = path.join(statusDir(), `${sessionId}.json`);
      if (action.kind === 'clear') {
        fs.rmSync(filePath, { force: true });
        return;
      }

      fs.mkdirSync(statusDir(), { recursive: true });
      fs.writeFileSync(filePath, JSON.stringify({ status: action.status, updatedAt: Date.now() }), 'utf8');
    } catch {
      // A malformed/partial payload should never surface as a hook failure to the user.
    }
  });
}

main();
