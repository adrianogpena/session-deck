import { DeckConfig, SessionStatus } from '@session-deck/core';

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
}

const splitArgs = (input: string): string[] => input.trim().split(/\s+/).filter(Boolean);

const VALID_STATUSES: readonly SessionStatus[] = ['running', 'waiting', 'done', 'error'];

/** One agent's `enabled`/`command`/`args` fields — `tools.claude.*` and `tools.copilot.*` are identical apart from which key they touch. */
function toolFields(agent: 'claude' | 'copilot'): ConfigField[] {
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
  },
  {
    label: 'ui.notifications',
    kind: 'toggle',
    display: (c) => (c.ui.notifications ? 'on' : 'off'),
    editValue: (c) => (c.ui.notifications ? 'on' : 'off'),
    apply: (c) => ({ ...c, ui: { ...c.ui, notifications: !c.ui.notifications } }),
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
  },
  {
    label: 'ui.recentProjectsFirst',
    kind: 'toggle',
    display: (c) => (c.ui.recentProjectsFirst ? 'on' : 'off'),
    editValue: (c) => (c.ui.recentProjectsFirst ? 'on' : 'off'),
    apply: (c) => ({ ...c, ui: { ...c.ui, recentProjectsFirst: !c.ui.recentProjectsFirst } }),
  },
  ...toolFields('claude'),
  ...toolFields('copilot'),
  {
    label: 'trash.retentionDays',
    kind: 'number',
    display: (c) => String(c.trash.retentionDays),
    editValue: (c) => String(c.trash.retentionDays),
    apply: (c, input) => {
      const n = Number(input);
      return Number.isInteger(n) && n > 0 ? { ...c, trash: { ...c.trash, retentionDays: n } } : undefined;
    },
  },
];
