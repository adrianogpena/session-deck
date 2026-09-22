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
      )}) — fix or back it up before enabling status tracking.`,
      { cause: error }
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

/**
 * `command` embeds the absolute path to `reportStatus.js` inside wherever this extension happens to be
 * installed and which `src/` subfolder it compiles under — both of which can change (a reinstall, or a
 * source reorganization, as already happened once) without the hook actually being a different thing.
 * Recognizing "one of ours" by the script's filename, not the exact current path, is what lets
 * enable/disable self-heal an *older-shaped* entry instead of leaving it running (broken) alongside a
 * freshly-added correct one, or refusing to remove it at all because it no longer matches exactly.
 */
const OUR_HOOK_COMMAND = /^node ".*[\\/]reportStatus\.js"$/;

function groupContainsOurCommand(group: HookGroup): boolean {
  return group.hooks.some((h) => OUR_HOOK_COMMAND.test(h.command));
}

/** True with the *current, exact* `command` installed for every event — i.e. tracking is enabled and up to date, not just present in some older shape. */
export function isStatusTrackingEnabled(command: string): boolean {
  const settings = readClaudeSettings();
  return STATUS_HOOK_SPECS.every((spec) =>
    (settings.hooks?.[spec.event] ?? []).some(
      (group) => groupContainsCommand(group, command) && group.matcher === spec.matcher
    )
  );
}

/** True if any of our hook entries are present at all, in any shape/path — whether or not it's the current exact command. What decides if "Disable" has anything to clean up. */
export function isStatusTrackingPresent(): boolean {
  const settings = readClaudeSettings();
  return STATUS_HOOK_SPECS.some((spec) => (settings.hooks?.[spec.event] ?? []).some(groupContainsOurCommand));
}

/**
 * Adds one hook group per entry in {@link STATUS_HOOK_SPECS}, each running
 * `command` — idempotent, and leaves every other hook/setting untouched.
 * Also self-heals: any *existing* occurrence of one of our hooks for that
 * event, in any older shape (a stale path, an unfiltered `Notification`
 * matcher before that was tightened, ...), is replaced rather than left
 * running (and possibly broken) alongside the new one.
 */
export function enableStatusTracking(command: string): void {
  const settings = readClaudeSettings();
  settings.hooks = settings.hooks ?? {};

  for (const spec of STATUS_HOOK_SPECS) {
    const groups = (settings.hooks[spec.event] ?? []).filter((group) => !groupContainsOurCommand(group));
    const newGroup: HookGroup = spec.matcher
      ? { matcher: spec.matcher, hooks: [{ type: 'command', command }] }
      : { hooks: [{ type: 'command', command }] };
    settings.hooks[spec.event] = [...groups, newGroup];
  }

  writeClaudeSettings(settings);
}

/** Removes every one of our hook entries from {@link STATUS_HOOK_SPECS}'s events, in any shape/path — leaves every other hook/setting untouched. */
export function disableStatusTracking(): void {
  const settings = readClaudeSettings();
  if (!settings.hooks) {
    return;
  }

  for (const spec of STATUS_HOOK_SPECS) {
    const groups = settings.hooks[spec.event];
    if (!groups) {
      continue;
    }
    const filtered = groups.filter((group) => !groupContainsOurCommand(group));

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
