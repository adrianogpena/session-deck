/**
 * The agent registry behind `tools.<agent>.*` (see `deckConfig.ts`): Claude and Copilot each have a
 * rich adapter (discovery, structured status, resume) in their own `claude*.ts`/`copilot*.ts` modules.
 * Everything else here is "basic" tier — spawned in a PTY on PATH, liveness-only status (running vs
 * exited, no "waiting for you" distinction), no persisted session history — same split z4-oriel makes
 * between its Claude/Codex integrations and its 10+ other catalog entries (see the plan review's Phase
 * 3, step 1). Seeded with a couple of entries for now; the full z4-oriel-derived list is a separate follow-on.
 */
export interface CatalogAgent {
  id: string;
  /** Shown in pickers and flash messages. */
  name: string;
}

export const BUILTIN_AGENTS = ['claude', 'copilot'] as const;
export type BuiltinAgentId = (typeof BUILTIN_AGENTS)[number];

const BUILTIN_AGENT_NAMES: Record<BuiltinAgentId, string> = {
  claude: 'Claude',
  copilot: 'Copilot',
};

export const CATALOG_AGENTS: readonly CatalogAgent[] = [
  { id: 'codex', name: 'Codex' },
  { id: 'gemini', name: 'Gemini' },
];

export function isBuiltinAgent(id: string): id is BuiltinAgentId {
  return (BUILTIN_AGENTS as readonly string[]).includes(id);
}

export function findCatalogAgent(id: string): CatalogAgent | undefined {
  return CATALOG_AGENTS.find((a) => a.id === id);
}

/** The two rich-adapter ids plus every basic-tier catalog id — every agent `tools.*`/the config popup knows about. */
export function allAgentIds(): string[] {
  return [...BUILTIN_AGENTS, ...CATALOG_AGENTS.map((a) => a.id)];
}

/** `'Claude'`/`'Copilot'` for the two built-ins, a catalog entry's own name, or the bare id as a last resort. */
export function agentDisplayName(id: string): string {
  if (isBuiltinAgent(id)) {
    return BUILTIN_AGENT_NAMES[id];
  }
  return findCatalogAgent(id)?.name ?? id;
}
