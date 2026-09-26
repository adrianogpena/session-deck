import * as vscode from 'vscode';
import { DeckStore } from '@session-deck/core';

const LEGACY_KEY_SESSION_NAMES = 'sessionDeck.sessionNameOverrides';
const LEGACY_KEY_ARCHIVED_SESSIONS = 'sessionDeck.archivedSessions';
const KEY_MIGRATED_TO_SHARED_STORE = 'sessionDeck.migratedToSharedStore';

/**
 * Session-level overrides only, keyed by session id (globally unique) — project-level config lives in
 * `workspaceConfig.ts` instead. Backed by the shared `~/.session-deck/state.json` so the terminal UI
 * sees the same names and archive flags. `globalState` only holds the pre-shared-store values, kept
 * as a backup after {@link migrateLegacyState} copies them over.
 */
export class DeckState {
  constructor(
    private readonly memento: vscode.Memento,
    private readonly store = new DeckStore()
  ) {}

  /** One-time copy of names/archive flags from `globalState` into the shared store. Values already in the store win. */
  async migrateLegacyState(): Promise<void> {
    if (this.memento.get<boolean>(KEY_MIGRATED_TO_SHARED_STORE)) {
      return;
    }
    const names = this.memento.get<Record<string, string>>(LEGACY_KEY_SESSION_NAMES, {});
    const archived = this.memento.get<Record<string, true>>(LEGACY_KEY_ARCHIVED_SESSIONS, {});
    const current = this.store.read().sessions;
    const patches: Record<string, { name?: string; archived?: boolean }> = {};
    for (const [id, name] of Object.entries(names)) {
      if (current[id]?.name === undefined) {
        patches[id] = { ...patches[id], name };
      }
    }
    for (const [id, flag] of Object.entries(archived)) {
      if (flag === true && current[id]?.archived === undefined) {
        patches[id] = { ...patches[id], archived: true };
      }
    }
    if (Object.keys(patches).length) {
      await this.store.updateSessions(patches);
    }
    await this.memento.update(KEY_MIGRATED_TO_SHARED_STORE, true);
  }

  /** Fires when the shared store changes, including edits made by the terminal UI. */
  watch(listener: () => void): vscode.Disposable {
    return this.store.watch(listener);
  }

  getSessionName(sessionId: string): string | undefined {
    return this.store.getSession(sessionId)?.name;
  }

  async setSessionName(sessionId: string, name: string): Promise<void> {
    await this.store.updateSession(sessionId, { name });
  }

  isSessionArchived(sessionId: string): boolean {
    return this.store.getSession(sessionId)?.archived === true;
  }

  async setSessionArchived(sessionId: string, archived: boolean): Promise<void> {
    await this.store.updateSession(sessionId, { archived });
  }
}
