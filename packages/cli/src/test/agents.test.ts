import { test } from 'node:test';
import assert from 'node:assert/strict';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { discoverLocalAgents, parseAgentFrontmatter } from '../agents';

test('parseAgentFrontmatter reads a plain single-line description', () => {
  const text = '---\nname: mmp-pr-preflight\ndescription: Runs the CI checks locally.\ntools: Bash\n---\n\nBody.';
  assert.deepEqual(parseAgentFrontmatter(text), { name: 'mmp-pr-preflight', description: 'Runs the CI checks locally.' });
});

test('parseAgentFrontmatter folds a ">" block scalar description onto one line', () => {
  const text = ['---', 'name: mmp-pr-preflight', 'description: >', '  Runs the CI checks locally.', '  Second line.', 'tools: Bash', '---', '', 'Body.'].join('\n');
  assert.deepEqual(parseAgentFrontmatter(text), { name: 'mmp-pr-preflight', description: 'Runs the CI checks locally. Second line.' });
});

test('parseAgentFrontmatter returns {} without a frontmatter block', () => {
  assert.deepEqual(parseAgentFrontmatter('# Just a heading\n\nNo frontmatter here.'), {});
});

test('discoverLocalAgents returns [] when the agents directory does not exist', () => {
  assert.deepEqual(discoverLocalAgents(path.join(os.tmpdir(), `session-deck-agents-test-missing-dir-${Date.now()}`)), []);
});

test('discoverLocalAgents reads every folder with exactly one *.agent.md, skips the rest', () => {
  const dir = path.join(os.tmpdir(), `session-deck-agents-test-dir-${Date.now()}`);
  fs.mkdirSync(path.join(dir, 'mmp-pr-preflight'), { recursive: true });
  fs.writeFileSync(path.join(dir, 'mmp-pr-preflight', 'mmp-pr-preflight.agent.md'), '---\nname: mmp-pr-preflight\ndescription: Runs CI checks locally.\n---\n');
  fs.mkdirSync(path.join(dir, 'code-reviewer'), { recursive: true });
  fs.writeFileSync(path.join(dir, 'code-reviewer', 'code-reviewer.agent.md'), '---\nname: code-reviewer\ndescription: Reviews a diff.\n---\n');
  fs.mkdirSync(path.join(dir, 'no-agent-file'), { recursive: true }); // no *.agent.md — not an agent itself
  fs.writeFileSync(path.join(dir, 'not-a-dir.agent.md'), '---\nname: stray\ndescription: Not inside its own folder.\n---\n'); // not a directory — skipped

  try {
    const agents = discoverLocalAgents(dir);
    assert.deepEqual(agents, [
      { name: 'code-reviewer', description: 'Reviews a diff.' },
      { name: 'mmp-pr-preflight', description: 'Runs CI checks locally.' },
    ]);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});
