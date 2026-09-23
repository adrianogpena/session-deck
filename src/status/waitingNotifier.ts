import notifier from 'node-notifier';
import { readEffectiveSessionStatus, SessionStatus } from './sessionStatus';

export interface WatchedSession {
  sessionId: string;
  label: string;
}

const NOTIFIABLE_STATUSES: ReadonlySet<SessionStatus> = new Set(['waiting', 'error']);

/**
 * Fires an OS desktop notification when a watched session transitions into "waiting"/"error" — not
 * repeatedly, and not on first sight (so opening onto an already-waiting session stays silent).
 * `onActivated` reveals that session's terminal if the OS notifier reports it was clicked.
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
