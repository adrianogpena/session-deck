import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { parseNameAndDescription } from './frontmatter';

/**
 * The four states Claude Code's `skillOverrides` setting can put a skill in:
 * - `on`: visible to the model and auto-triggerable (the default with no override).
 * - `name-only`: name visible, no description — so it doesn't add context cost but can't fire on its own.
 * - `user-invocable-only`: fully hidden from the model's context, still reachable by typing its name in `/`.
 * - `off`: removed entirely, even from `/`.
 */
export type SkillState = 'on' | 'name-only' | 'user-invocable-only' | 'off';

export const SKILL_STATES: readonly SkillState[] = ['on', 'name-only', 'user-invocable-only', 'off'];

export interface LocalSkill {
  name: string;
  description: string;
  state: SkillState;
}

export function getSkillsDir(): string {
  return path.join(os.homedir(), '.claude', 'skills');
}

export function getClaudeSettingsPath(): string {
  return path.join(os.homedir(), '.claude', 'settings.json');
}

/**
 * Pulls `name`/`description` out of a SKILL.md's YAML frontmatter. Not a real YAML parser — every
 * skill in practice uses either a plain scalar or a folded (`>`) / literal (`|`) block for these two
 * keys, and that's all this handles.
 */
export function parseSkillFrontmatter(text: string): { name?: string; description?: string } {
  return parseNameAndDescription(text);
}

/** `skillOverrides` from `~/.claude/settings.json` — `{}` if the file is missing, unreadable, or has none. */
export function readSkillOverrides(settingsPath: string = getClaudeSettingsPath()): Partial<Record<string, SkillState>> {
  let raw: unknown;
  try {
    raw = JSON.parse(fs.readFileSync(settingsPath, 'utf8'));
  } catch {
    return {};
  }
  const overrides = (raw as { skillOverrides?: unknown })?.skillOverrides;
  if (typeof overrides !== 'object' || overrides === null) {
    return {};
  }
  const result: Partial<Record<string, SkillState>> = {};
  for (const [name, value] of Object.entries(overrides as Record<string, unknown>)) {
    if (SKILL_STATES.includes(value as SkillState)) {
      result[name] = value as SkillState;
    }
  }
  return result;
}

/**
 * Every personal skill directly under `~/.claude/skills` (each a real directory or a symlink, e.g.
 * into `~/.agents/skills` for one managed by the skills CLI) — not the nested `synced/` bundle, which
 * has no `SKILL.md` of its own at that level and so is naturally skipped. Each skill's state is
 * `skillOverrides[name]` from `~/.claude/settings.json`, or `'on'` when there's no override.
 */
export function discoverLocalSkills(skillsDir: string = getSkillsDir(), settingsPath: string = getClaudeSettingsPath()): LocalSkill[] {
  let entries: string[];
  try {
    entries = fs.readdirSync(skillsDir);
  } catch {
    return [];
  }
  const overrides = readSkillOverrides(settingsPath);
  const skills: LocalSkill[] = [];
  for (const entry of entries) {
    let text: string;
    try {
      text = fs.readFileSync(path.join(skillsDir, entry, 'SKILL.md'), 'utf8');
    } catch {
      continue; // no SKILL.md directly under this entry (e.g. the synced/ bundle folder)
    }
    const meta = parseSkillFrontmatter(text);
    const name = meta.name ?? entry;
    skills.push({ name, description: meta.description ?? '', state: overrides[name] ?? 'on' });
  }
  return skills.sort((a, b) => a.name.localeCompare(b.name));
}
