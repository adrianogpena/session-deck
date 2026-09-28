import notifier from 'node-notifier';
import { claimNotification, readEffectiveSessionStatus, SessionStatus } from './sessionStatus';

export interface WatchedSession {
  sessionId: string;
  label: string;
  /** Shown as the toast's title. Falls back to "Session Deck" when omitted. */
  project?: string;
}

export interface WaitingNotifierOptions {
  /** Raster icon for the notification (most OS notifiers ignore SVG). */
  iconPath?: string;
  /** Statuses worth a notification. Defaults to "waiting" and "error". */
  statuses?: readonly SessionStatus[];
  /** Skip a session the user is already looking at. */
  skip?(sessionId: string): boolean;
}

const DEFAULT_STATUSES: readonly SessionStatus[] = ['waiting', 'error'];

const MESSAGES: Record<SessionStatus, (label: string) => string> = {
  waiting: (label) => `${label} is waiting for your input`,
  done: (label) => `${label} finished`,
  error: (label) => `${label} exited with an error`,
  running: (label) => `${label} is running`,
};

/**
 * Fires an OS desktop notification when a watched session transitions into one of `statuses` — not
 * repeatedly, and not on first sight (so opening onto an already-waiting session stays silent).
 * `claimNotification` makes sure the extension and the terminal UI don't both notify the same event.
 * `onActivated` is called with the session id if the OS notifier reports it was clicked.
 */
export class WaitingNotifier {
  private readonly lastStatus = new Map<string, SessionStatus | undefined>();
  private readonly options: WaitingNotifierOptions;

  constructor(options: WaitingNotifierOptions | string = {}) {
    this.options = typeof options === 'string' ? { iconPath: options } : options;
  }

  /** `statusesOverride` wins over the constructor's `statuses` option, for a caller whose notified statuses can change at runtime (e.g. a config setting). */
  check(sessions: readonly WatchedSession[], onActivated?: (sessionId: string) => void, statusesOverride?: readonly SessionStatus[]): void {
    const statuses = statusesOverride ?? this.options.statuses ?? DEFAULT_STATUSES;
    const stillWatched = new Set(sessions.map((s) => s.sessionId));
    for (const knownId of [...this.lastStatus.keys()]) {
      if (!stillWatched.has(knownId)) {
        this.lastStatus.delete(knownId);
      }
    }

    for (const { sessionId, label, project } of sessions) {
      const isFirstSight = !this.lastStatus.has(sessionId);
      const previous = this.lastStatus.get(sessionId);
      const record = readEffectiveSessionStatus(sessionId);
      const current = record?.status;
      this.lastStatus.set(sessionId, current);

      if (isFirstSight || current === previous || !record || !current || !statuses.includes(current)) {
        continue;
      }
      if (this.options.skip?.(sessionId) || !claimNotification(sessionId, record.updatedAt)) {
        continue;
      }

      notifier.notify(
        { title: project ?? 'Session Deck', message: MESSAGES[current](label), icon: this.options.iconPath, appID: 'Session Deck' },
        (error, response) => {
          if (!error && response === 'activate') {
            onActivated?.(sessionId);
          }
        }
      );
    }
  }
}
