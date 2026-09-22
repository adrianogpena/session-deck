import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';

/**
 * The Claude Code hook events (and, where it matters, the `notification_type`
 * matcher) Session Deck listens on to derive a session's coarse status.
 *
 * `Notification` is filtered to only the sub-types that represent something
 * genuinely needing *your* action (`permission_prompt`, `agent_needs_input`,
 * an MCP elicitation dialog) — deliberately excluding `idle_prompt`, Claude
 * Code's automatic "you've been idle a while" nudge. `Notification` hooks run
 * asynchronously (Claude Code doesn't wait for them), so an unfiltered
 * `idle_prompt` firing on its own timer could have its (slow) write land
 * *after* a fast `UserPromptSubmit` write and incorrectly flip a freshly
 * "running" session back to "waiting". Filtering by matcher means Claude Code
 * never even invokes the hook for `idle_prompt` — no process, no race.
 */
interface StatusHookSpec {
  event: string;
  matcher?: string;
}

const STATUS_HOOK_SPECS: StatusHookSpec[] = [
  { event: 'UserPromptSubmit' },
  { event: 'Stop' },
  { event: 'Notification', matcher: 'permission_prompt|agent_needs_input|elicitation_dialog|elicitation_url_dialog' },
  { event: 'SessionEnd' },
];

interface HookCommandEntry {
  type: 'command';
  command: string;
}

interface HookGroup {
  matcher?: string;
  hooks: HookCommandEntry[];
}

interface ClaudeSettings {
  hooks?: Record<string, HookGroup[]>;
  [key: string]: unknown;
}

export function getClaudeSettingsPath(): string {
  return path.join(os.homedir(), '.claude', 'settings.json');
}

/** Throws if the file exists but isn't valid JSON — callers should surface that rather than silently overwrite it. */
function readClaudeSettings(): ClaudeSettings {
  const settingsPath = getClaudeSettingsPath();
  if (!fs.existsSync(settingsPath)) {
    return {};
  }
  const raw = fs.readFileSync(settingsPath, 'utf8');
  try {
    return raw.trim() ? (JSON.parse(raw) as ClaudeSettings) : {};
  } catch (error) {
    throw new Error(
      `${settingsPath} is not valid JSON (${String(
        error instanceof Error ? error.message : error
      )}) — fix or back it up before enabling status tracking.`
    );
  }
}

function writeClaudeSettings(settings: ClaudeSettings): void {
  const settingsPath = getClaudeSettingsPath();
  fs.mkdirSync(path.dirname(settingsPath), { recursive: true });
  fs.writeFileSync(settingsPath, `${JSON.stringify(settings, null, 2)}\n`, 'utf8');
}

function groupContainsCommand(group: HookGroup, command: string): boolean {
  return group.hooks.some((h) => h.command === command);
}

export function isStatusTrackingEnabled(command: string): boolean {
  const settings = readClaudeSettings();
  return STATUS_HOOK_SPECS.every((spec) =>
    (settings.hooks?.[spec.event] ?? []).some(
      (group) => groupContainsCommand(group, command) && group.matcher === spec.matcher
    )
  );
}

/**
 * Adds one hook group per entry in {@link STATUS_HOOK_SPECS}, each running
 * `command` — idempotent, and leaves every other hook/setting untouched.
 * Also self-heals: any *existing* occurrence of our own command for that
 * event (from an older version of this extension, e.g. an unfiltered
 * `Notification` matcher before that was tightened) is replaced rather than
 * left running alongside the new one — otherwise both would fire.
 */
export function enableStatusTracking(command: string): void {
  const settings = readClaudeSettings();
  settings.hooks = settings.hooks ?? {};

  for (const spec of STATUS_HOOK_SPECS) {
    const groups = (settings.hooks[spec.event] ?? []).filter((group) => !groupContainsCommand(group, command));
    const newGroup: HookGroup = spec.matcher
      ? { matcher: spec.matcher, hooks: [{ type: 'command', command }] }
      : { hooks: [{ type: 'command', command }] };
    settings.hooks[spec.event] = [...groups, newGroup];
  }

  writeClaudeSettings(settings);
}

/** Removes every hook entry running `command` from {@link STATUS_HOOK_SPECS}'s events, regardless of matcher shape — leaves every other hook/setting untouched. */
export function disableStatusTracking(command: string): void {
  const settings = readClaudeSettings();
  if (!settings.hooks) {
    return;
  }

  for (const spec of STATUS_HOOK_SPECS) {
    const groups = settings.hooks[spec.event];
    if (!groups) {
      continue;
    }
    const filtered = groups.filter((group) => !groupContainsCommand(group, command));

    if (filtered.length > 0) {
      settings.hooks[spec.event] = filtered;
    } else {
      delete settings.hooks[spec.event];
    }
  }

  if (Object.keys(settings.hooks).length === 0) {
    delete settings.hooks;
  }

  writeClaudeSettings(settings);
}
