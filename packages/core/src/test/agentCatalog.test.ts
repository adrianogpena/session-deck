import { test } from 'node:test';
import assert from 'node:assert/strict';
import { agentDisplayName, allAgentIds, CATALOG_AGENTS, findCatalogAgent, isBuiltinAgent } from '../agentCatalog';

test('isBuiltinAgent is true only for claude/copilot', () => {
  assert.equal(isBuiltinAgent('claude'), true);
  assert.equal(isBuiltinAgent('copilot'), true);
  assert.equal(isBuiltinAgent('codex'), false);
  assert.equal(isBuiltinAgent('bogus'), false);
});

test('allAgentIds lists the two built-ins followed by every catalog entry', () => {
  assert.deepEqual(allAgentIds(), ['claude', 'copilot', ...CATALOG_AGENTS.map((a) => a.id)]);
});

test('findCatalogAgent finds a seeded entry and misses an unknown id', () => {
  assert.equal(findCatalogAgent('codex')?.name, 'Codex');
  assert.equal(findCatalogAgent('bogus'), undefined);
});

test('agentDisplayName: built-ins get their fixed name, a catalog agent its own, anything else falls back to the bare id', () => {
  assert.equal(agentDisplayName('claude'), 'Claude');
  assert.equal(agentDisplayName('copilot'), 'Copilot');
  assert.equal(agentDisplayName('codex'), 'Codex');
  assert.equal(agentDisplayName('some-future-agent'), 'some-future-agent');
});
