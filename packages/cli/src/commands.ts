import type { App } from './app';

/** One entry in the `:` command palette — wraps an existing key's action under a typed name. */
export interface PaletteCommand {
  id: string;
  label: string;
  run(app: App): void;
}

/**
 * Seeds the palette with 7 existing single-key actions, unchanged — no new behavior, just a second,
 * typed-name way to reach each one. `App` exposes the handful of methods below (and `activeAgentId`)
 * specifically so this module can call them without duplicating their logic.
 */
export const PALETTE_COMMANDS: readonly PaletteCommand[] = [
  { id: 'new-session', label: 'New session', run: (app) => app.newSession(app.activeAgentId) },
  { id: 'new-session-choose-agent', label: 'New session, choose agent', run: (app) => app.openAgentPicker({ setActive: false }) },
  { id: 'rename', label: 'Rename', run: (app) => app.rename() },
  { id: 'move-to-folder', label: 'Move project to folder', run: (app) => app.openMovePicker() },
  { id: 'open-config', label: 'Open config', run: (app) => app.openConfig() },
  { id: 'toggle-sidebar', label: 'Toggle sidebar', run: (app) => app.toggleSidebar() },
  { id: 'open-trash', label: 'Open trash', run: (app) => app.openTrashPicker() },
];

/**
 * Ported as-is from z4-oriel's `palette_matches`: every typed character of `query` must appear, in
 * order, somewhere in a label (case- and whitespace-insensitive) — no scoring, no ranking, just a
 * forgiving subsequence filter. An empty query matches every label, in list order.
 */
export function paletteMatches(query: string, labels: readonly string[]): number[] {
  const q = Array.from(query.toLowerCase().replace(/\s+/g, ''));
  const matches: number[] = [];
  labels.forEach((label, i) => {
    const chars = label.toLowerCase().replace(/\s+/g, '');
    let pos = 0;
    const matched = q.every((c) => {
      const found = chars.indexOf(c, pos);
      if (found === -1) {
        return false;
      }
      pos = found + 1;
      return true;
    });
    if (matched) {
      matches.push(i);
    }
  });
  return matches;
}
