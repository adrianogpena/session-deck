import { test } from 'node:test';
import assert from 'node:assert/strict';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { resolveNpmGlobalExecutable } from '../liveSession';

function makeShimDir(pkgName: string, bin: Record<string, string> | string, binFiles: string[]): string {
  const shimDir = fs.mkdtempSync(path.join(os.tmpdir(), 'sdeck-npm-'));
  const pkgDir = path.join(shimDir, 'node_modules', pkgName);
  fs.mkdirSync(pkgDir, { recursive: true });
  fs.writeFileSync(path.join(pkgDir, 'package.json'), JSON.stringify({ bin }));
  for (const file of binFiles) {
    fs.mkdirSync(path.dirname(path.join(pkgDir, file)), { recursive: true });
    fs.writeFileSync(path.join(pkgDir, file), '');
  }
  return shimDir;
}

test('resolveNpmGlobalExecutable resolves a native .exe the package actually ships', () => {
  const shimDir = makeShimDir('@anthropic-ai/claude-code', { claude: 'bin/claude.exe' }, ['bin/claude.exe']);
  assert.equal(resolveNpmGlobalExecutable('claude', shimDir), path.join(shimDir, 'node_modules', '@anthropic-ai/claude-code', 'bin', 'claude.exe'));
});

test('resolveNpmGlobalExecutable returns undefined when the bin entry is a JS loader, not an .exe', () => {
  const shimDir = makeShimDir('@github/copilot', { copilot: 'npm-loader.js' }, ['npm-loader.js']);
  assert.equal(resolveNpmGlobalExecutable('copilot', shimDir), undefined);
});

test('resolveNpmGlobalExecutable returns undefined when the .exe the package.json points to is missing', () => {
  const shimDir = makeShimDir('@anthropic-ai/claude-code', { claude: 'bin/claude.exe' }, []);
  assert.equal(resolveNpmGlobalExecutable('claude', shimDir), undefined);
});

test('resolveNpmGlobalExecutable returns undefined when there is no package.json at all', () => {
  const shimDir = fs.mkdtempSync(path.join(os.tmpdir(), 'sdeck-npm-'));
  assert.equal(resolveNpmGlobalExecutable('claude', shimDir), undefined);
});
