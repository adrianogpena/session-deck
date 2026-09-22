import * as vscode from 'vscode';

const KEY_SESSION_NAMES = 'sessionDeck.sessionNameOverrides';

/**
 * Session name overrides live in `globalState` rather than `workspaceState` —
 * Session Deck spans every project on the machine, not just the current
 * workspace. Sessions are keyed by their session id alone: it's a UUID, so it's
 * already globally unique.
 *
 * Project-level display names/membership are *not* here — they live entirely in
 * `.vscode/session-deck.json` (`workspaceConfig.ts`), which is the sole source of
 * truth for which projects show and what they're called.
 */
export class DeckState {
  constructor(private readonly memento: vscode.Memento) {}

  getSessionName(sessionId: string): string | undefined {
    return this.memento.get<Record<string, string>>(KEY_SESSION_NAMES, {})[sessionId];
  }

  async setSessionName(sessionId: string, name: string): Promise<void> {
    const map = { ...this.memento.get<Record<string, string>>(KEY_SESSION_NAMES, {}) };
    map[sessionId] = name;
    await this.memento.update(KEY_SESSION_NAMES, map);
  }
}
