// Bundles the extension into a single out/extension.js. vsce can't collect dependencies from npm
// workspaces (they're hoisted to the repo root), so the .vsix ships the bundle with --no-dependencies.
import * as esbuild from 'esbuild';
import * as fs from 'fs';
import * as path from 'path';
import { createRequire } from 'module';

const watch = process.argv.includes('--watch');
const production = process.argv.includes('--production');

// node-notifier finds its helper binaries (SnoreToast etc.) at path.resolve(__dirname, '../vendor/...').
// Bundled, __dirname is out/, so they must sit in vendor/ next to it.
const notifierDir = path.dirname(createRequire(import.meta.url).resolve('node-notifier/package.json'));
fs.rmSync('vendor', { recursive: true, force: true });
fs.cpSync(path.join(notifierDir, 'vendor'), 'vendor', { recursive: true });

const ctx = await esbuild.context({
  entryPoints: ['src/extension.ts'],
  bundle: true,
  platform: 'node',
  target: 'node18',
  format: 'cjs',
  outfile: 'out/extension.js',
  external: ['vscode'],
  sourcemap: !production,
  minify: production,
  logLevel: 'info',
});

if (watch) {
  await ctx.watch();
} else {
  await ctx.rebuild();
  await ctx.dispose();
}
