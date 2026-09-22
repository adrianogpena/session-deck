import notifier from 'node-notifier';
import { readEffectiveSessionStatus, SessionStatus } from './sessionStatus';

export interface WatchedSession {
  sessionId: string;
  label: string;
}

const NOTIFIABLE_STATUSES: ReadonlySet<SessionStatus> = new Set(['waiting', 'error']);

/**
 * Fires a real OS desktop notification (`node-notifier` — a plain
 * `vscode.window.showInformationMessage` toast is an in-app affordance only,
 * easy to miss while working in a different application) the moment one of
 * the sessions it's told to watch transitions *into* "waiting" or "error" —
 * never repeatedly while it stays in that state, and never the first time a
 * session is handed to `check()` (so opening the extension onto an
 * already-waiting session doesn't immediately notify; same reasoning as
 * `sessionProvider.ts`'s `enforceArchiveCap` baseline).
 *
 * There's no stable VS Code API to force the OS window to the foreground —
 * clicking the notification balloon itself is what actually brings VS Code
 * forward, an OS behavior this has no control over. `onActivated` is the
 * closest this can do from inside the extension: reveal that session's
 * already-open terminal within the window once it's in front. Whether the
 * underlying platform notifier reports a genuine "clicked" response (as
 * opposed to a dismiss or timeout) via that exact string varies by OS and
 * hasn't been exercised interactively — worst case `onActivated` just never
 * fires, which only loses that one nicety, not the notification itself.
 */
export class WaitingNotifier {
  private readonly lastStatus = new Map<string, SessionStatus | undefined>();

  constructor(private readonly iconPath: string) {}

  check(sessions: readonly WatchedSession[], onActivated?: (sessionId: string) => void): void {
    const stillWatched = new Set(sessions.map((s) => s.sessionId));
    for (const knownId of [...this.lastStatus.keys()]) {
      if (!stillWatched.has(knownId)) {
        this.lastStatus.delete(knownId);
      }
    }

    for (const { sessionId, label } of sessions) {
      const isFirstSight = !this.lastStatus.has(sessionId);
      const previous = this.lastStatus.get(sessionId);
      const current = readEffectiveSessionStatus(sessionId)?.status;
      this.lastStatus.set(sessionId, current);

      if (isFirstSight || current === previous || !current || !NOTIFIABLE_STATUSES.has(current)) {
        continue;
      }

      notifier.notify(
        {
          title: 'Session Deck',
          message: current === 'error' ? `${label} exited with an error` : `${label} is waiting for your input`,
          icon: this.iconPath,
        },
        (error, response) => {
          if (!error && response === 'activate') {
            onActivated?.(sessionId);
          }
        }
      );
    }
  }
}
