import { allAgentIds, DeckConfig, SessionStatus } from '@session-deck/core';

/** `toggle` applies immediately on Enter; the others open a text prompt first, pre-filled with `editValue`. */
export type ConfigFieldKind = 'toggle' | 'number' | 'text' | 'args' | 'statusList';

export interface ConfigField {
  label: string;
  kind: ConfigFieldKind;
  /** Friendly rendering for the popup row, e.g. `(default: claude)` when unset. */
  display(config: DeckConfig): string;
  /** The prompt's starting value when editing (ignored for `toggle`) — the raw stored value, empty when unset. */
  editValue(config: DeckConfig): string;
  /**
   * `toggle` fields ignore `input` and flip themselves. The others parse the prompt's submitted text;
   * `undefined` means invalid input, so the caller keeps the config unchanged.
   */
  apply(config: DeckConfig, input: string): DeckConfig | undefined;
  /** One-line plain-English explanation shown under the popup when this row is selected, for a field whose label/value alone wouldn't tell a first-time user what it does. */
  hint?: string;
}

const splitArgs = (input: string): string[] => input.trim().split(/\s+/).filter(Boolean);

const VALID_STATUSES: readonly SessionStatus[] = ['running', 'waiting', 'done', 'error'];

/** One agent's `enabled`/`command`/`args` fields — `tools.<agent>.*` for every agent in `allAgentIds()` is identical apart from which key it touches. */
function toolFields(agent: string): ConfigField[] {
  return [
    {
      label: `tools.${agent}.enabled`,
      kind: 'toggle',
      display: (c) => (c.tools[agent].enabled === false ? 'off' : 'on'),
      editValue: (c) => (c.tools[agent].enabled === false ? 'off' : 'on'),
      apply: (c) => ({ ...c, tools: { ...c.tools, [agent]: { ...c.tools[agent], enabled: c.tools[agent].enabled === false } } }),
    },
    {
      label: `tools.${agent}.command`,
      kind: 'text',
      display: (c) => c.tools[agent].command ?? `(default: ${agent})`,
      editValue: (c) => c.tools[agent].command ?? '',
      apply: (c, input) => ({ ...c, tools: { ...c.tools, [agent]: { ...c.tools[agent], command: input.trim() || undefined } } }),
    },
    {
      label: `tools.${agent}.args`,
      kind: 'args',
      display: (c) => c.tools[agent].args?.join(' ') || '(none)',
      editValue: (c) => c.tools[agent].args?.join(' ') ?? '',
      apply: (c, input) => {
        const args = splitArgs(input);
        return { ...c, tools: { ...c.tools, [agent]: { ...c.tools[agent], args: args.length ? args : undefined } } };
      },
    },
  ];
}

export const CONFIG_FIELDS: readonly ConfigField[] = [
  {
    label: 'ui.maxSessionsListed',
    kind: 'number',
    display: (c) => String(c.ui.maxSessionsListed),
    editValue: (c) => String(c.ui.maxSessionsListed),
    apply: (c, input) => {
      const n = Number(input);
      return Number.isInteger(n) && n > 0 ? { ...c, ui: { ...c.ui, maxSessionsListed: n } } : undefined;
    },
    hint: 'How many of the most recent sessions load into the list from disk.',
  },
  {
    label: 'ui.notifications',
    kind: 'toggle',
    display: (c) => (c.ui.notifications ? 'on' : 'off'),
    editValue: (c) => (c.ui.notifications ? 'on' : 'off'),
    apply: (c) => ({ ...c, ui: { ...c.ui, notifications: !c.ui.notifications } }),
    hint: 'Desktop notifications for sessions that need you — see notifyStatuses below for which ones.',
  },
  {
    label: 'ui.notifyStatuses',
    kind: 'statusList',
    display: (c) => c.ui.notifyStatuses.join(' ') || '(none)',
    editValue: (c) => c.ui.notifyStatuses.join(' '),
    apply: (c, input) => {
      const values = splitArgs(input);
      if (!values.every((v): v is SessionStatus => (VALID_STATUSES as readonly string[]).includes(v))) {
        return undefined;
      }
      return { ...c, ui: { ...c.ui, notifyStatuses: values as SessionStatus[] } };
    },
    hint: 'Which session statuses trigger a desktop notification (space-separated: running waiting done error).',
  },
  {
    label: 'ui.recentProjectsFirst',
    kind: 'toggle',
    display: (c) => (c.ui.recentProjectsFirst ? 'on' : 'off'),
    editValue: (c) => (c.ui.recentProjectsFirst ? 'on' : 'off'),
    apply: (c) => ({ ...c, ui: { ...c.ui, recentProjectsFirst: !c.ui.recentProjectsFirst } }),
    hint: 'On: top-level projects reorder by most-recent activity. Off: a fixed, alphabetical order until you move one with K/J.',
  },
  {
    label: 'ui.recentSessionsFirst',
    kind: 'toggle',
    display: (c) => (c.ui.recentSessionsFirst ? 'on' : 'off'),
    editValue: (c) => (c.ui.recentSessionsFirst ? 'on' : 'off'),
    apply: (c) => ({ ...c, ui: { ...c.ui, recentSessionsFirst: !c.ui.recentSessionsFirst } }),
    hint: 'On: sessions in a project reorder by most-recent activity. Off: a fixed order until you move one with K/J.',
  },
  {
    label: 'ui.newSessionFullScreen',
    kind: 'toggle',
    display: (c) => (c.ui.newSessionFullScreen ? 'Attached' : 'Interacting'),
    editValue: (c) => (c.ui.newSessionFullScreen ? 'on' : 'off'),
    apply: (c) => ({ ...c, ui: { ...c.ui, newSessionFullScreen: !c.ui.newSessionFullScreen } }),
    hint: 'Which mode n/N opens a new session in: Attached — full-screen, same as pressing Enter — or Interacting — typed into in place, list and preview still showing, same as pressing i.',
  },
  {
    label: 'ui.gitStatus',
    kind: 'toggle',
    display: (c) => (c.ui.gitStatus ? 'on' : 'off'),
    editValue: (c) => (c.ui.gitStatus ? 'on' : 'off'),
    apply: (c) => ({ ...c, ui: { ...c.ui, gitStatus: !c.ui.gitStatus } }),
    hint: 'Shows ⇡/⇣/✱ git badges on rows and the branch name in the preview panel.',
  },
  {
    label: 'ui.expandCollapsedOnActiveJump',
    kind: 'toggle',
    display: (c) => (c.ui.expandCollapsedOnActiveJump ? 'on' : 'off'),
    editValue: (c) => (c.ui.expandCollapsedOnActiveJump ? 'on' : 'off'),
    apply: (c) => ({ ...c, ui: { ...c.ui, expandCollapsedOnActiveJump: !c.ui.expandCollapsedOnActiveJump } }),
    hint: 'On: [ and ] can expand a collapsed folder/project to reach a session inside. Off: they only jump between sessions already shown.',
  },
  {
    label: 'ui.showUsage',
    kind: 'toggle',
    display: (c) => (c.ui.showUsage ? 'on' : 'off'),
    editValue: (c) => (c.ui.showUsage ? 'on' : 'off'),
    apply: (c) => ({ ...c, ui: { ...c.ui, showUsage: !c.ui.showUsage } }),
    hint: 'Shows a Context/5h/7d usage section at the bottom of the list for the selected Claude session.',
  },
  {
    label: 'ui.use24HourClock',
    kind: 'toggle',
    display: (c) => (c.ui.use24HourClock ? '24-hour' : '12-hour'),
    editValue: (c) => (c.ui.use24HourClock ? 'on' : 'off'),
    apply: (c) => ({ ...c, ui: { ...c.ui, use24HourClock: !c.ui.use24HourClock } }),
    hint: "The 5h usage row's reset time shows as 20:30 instead of 8:30 PM.",
  },
  ...allAgentIds().flatMap(toolFields),
  {
    label: 'trash.retentionDays',
    kind: 'number',
    display: (c) => String(c.trash.retentionDays),
    editValue: (c) => String(c.trash.retentionDays),
    apply: (c, input) => {
      const n = Number(input);
      return Number.isInteger(n) && n > 0 ? { ...c, trash: { ...c.trash, retentionDays: n } } : undefined;
    },
    hint: "Deleted sessions older than this are purged from ~/.session-deck/trash/ at startup.",
  },
];
