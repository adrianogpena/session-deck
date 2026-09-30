import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { parseNameAndDescription } from './frontmatter';

export interface LocalAgent {
  name: string;
  description: string;
}

export function getAgentsDir(): string {
  return path.join(os.homedir(), '.claude', 'agents');
}

/** Pulls `name`/`description` out of a `*.agent.md`'s YAML frontmatter. */
export function parseAgentFrontmatter(text: string): { name?: string; description?: string } {
  return parseNameAndDescription(text);
}

/**
 * Every personal subagent directly under `~/.claude/agents` (each a real directory or a symlink),
 * identified by the one `*.agent.md` file directly inside it — a folder without one (or with more
 * than one) is skipped.
 */
export function discoverLocalAgents(agentsDir: string = getAgentsDir()): LocalAgent[] {
  let entries: string[];
  try {
    entries = fs.readdirSync(agentsDir);
  } catch {
    return [];
  }
  const agents: LocalAgent[] = [];
  for (const entry of entries) {
    const dir = path.join(agentsDir, entry);
    let files: string[];
    try {
      files = fs.readdirSync(dir);
    } catch {
      continue; // not a directory (or unreadable)
    }
    const agentFiles = files.filter((f) => f.endsWith('.agent.md'));
    if (agentFiles.length !== 1) {
      continue;
    }
    const text = fs.readFileSync(path.join(dir, agentFiles[0]), 'utf8');
    const meta = parseAgentFrontmatter(text);
    agents.push({ name: meta.name ?? entry, description: meta.description ?? '' });
  }
  return agents.sort((a, b) => a.name.localeCompare(b.name));
}
