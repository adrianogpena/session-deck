import { test } from 'node:test';
import assert from 'node:assert/strict';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { discoverLocalSkills, parseSkillFrontmatter, readSkillOverrides } from '../skills';

test('parseSkillFrontmatter reads a plain single-line description', () => {
  const text = '---\nname: mermaid-diagrams\ndescription: Comprehensive guide for creating software diagrams.\n---\n\nBody.';
  assert.deepEqual(parseSkillFrontmatter(text), { name: 'mermaid-diagrams', description: 'Comprehensive guide for creating software diagrams.' });
});

test('parseSkillFrontmatter folds a ">" block scalar description onto one line', () => {
  const text = [
    '---',
    'name: coding-conventions',
    'version: 1.2.0',
    'description: >',
    '  Checks Sofico code, tests, and Flyway migration scripts against the coding conventions of the',
    '  miles-core/miles-ria monolith.',
    '---',
    '',
    'Body.',
  ].join('\n');
  assert.deepEqual(parseSkillFrontmatter(text), {
    name: 'coding-conventions',
    description: 'Checks Sofico code, tests, and Flyway migration scripts against the coding conventions of the miles-core/miles-ria monolith.',
  });
});

test('parseSkillFrontmatter folds a "|" literal block scalar the same way', () => {
  const text = ['---', 'name: humanizer', 'description: |', '  Remove signs of AI-generated writing from text.', '  Second line.', '---'].join('\n');
  assert.deepEqual(parseSkillFrontmatter(text), { name: 'humanizer', description: 'Remove signs of AI-generated writing from text. Second line.' });
});

test('parseSkillFrontmatter returns {} without a frontmatter block', () => {
  assert.deepEqual(parseSkillFrontmatter('# Just a heading\n\nNo frontmatter here.'), {});
});

test('readSkillOverrides returns {} when the settings file is missing, malformed, or has no skillOverrides', () => {
  assert.deepEqual(readSkillOverrides(path.join(os.tmpdir(), `session-deck-skills-test-missing-${Date.now()}.json`)), {});
  const malformed = path.join(os.tmpdir(), `session-deck-skills-test-malformed-${Date.now()}.json`);
  fs.writeFileSync(malformed, '{ not json');
  try {
    assert.deepEqual(readSkillOverrides(malformed), {});
  } finally {
    fs.unlinkSync(malformed);
  }
  const noOverrides = path.join(os.tmpdir(), `session-deck-skills-test-none-${Date.now()}.json`);
  fs.writeFileSync(noOverrides, JSON.stringify({ theme: 'dark' }));
  try {
    assert.deepEqual(readSkillOverrides(noOverrides), {});
  } finally {
    fs.unlinkSync(noOverrides);
  }
});

test('readSkillOverrides keeps only recognized state values', () => {
  const file = path.join(os.tmpdir(), `session-deck-skills-test-overrides-${Date.now()}.json`);
  fs.writeFileSync(file, JSON.stringify({ skillOverrides: { 'mermaid-diagrams': 'user-invocable-only', bogus: 'not-a-real-state' } }));
  try {
    assert.deepEqual(readSkillOverrides(file), { 'mermaid-diagrams': 'user-invocable-only' });
  } finally {
    fs.unlinkSync(file);
  }
});

test('discoverLocalSkills returns [] when the skills directory does not exist', () => {
  assert.deepEqual(discoverLocalSkills(path.join(os.tmpdir(), `session-deck-skills-test-missing-dir-${Date.now()}`)), []);
});

test('discoverLocalSkills reads every entry with a SKILL.md, skips ones without, defaults to "on", and applies overrides', () => {
  const dir = path.join(os.tmpdir(), `session-deck-skills-test-dir-${Date.now()}`);
  fs.mkdirSync(path.join(dir, 'mermaid-diagrams'), { recursive: true });
  fs.writeFileSync(path.join(dir, 'mermaid-diagrams', 'SKILL.md'), '---\nname: mermaid-diagrams\ndescription: Draw diagrams.\n---\n');
  fs.mkdirSync(path.join(dir, 'plan-review'), { recursive: true });
  fs.writeFileSync(path.join(dir, 'plan-review', 'SKILL.md'), '---\nname: plan-review\ndescription: Review a plan.\n---\n');
  fs.mkdirSync(path.join(dir, 'synced'), { recursive: true }); // no SKILL.md directly inside — not a skill itself

  const settingsPath = path.join(dir, 'settings.json');
  fs.writeFileSync(settingsPath, JSON.stringify({ skillOverrides: { 'mermaid-diagrams': 'user-invocable-only' } }));

  try {
    const skills = discoverLocalSkills(dir, settingsPath);
    assert.deepEqual(skills, [
      { name: 'mermaid-diagrams', description: 'Draw diagrams.', state: 'user-invocable-only' },
      { name: 'plan-review', description: 'Review a plan.', state: 'on' },
    ]);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});
