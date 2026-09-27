// Bundles the CLI into a single out/main.js so it's installable standalone via `npm install -g sdeck`
// — otherwise its @session-deck/core dependency ("*") only resolves inside this npm workspace.
// node-pty has a native binding and can't be bundled, so it stays a real, external dependency.
import * as esbuild from 'esbuild';
import * as fs from 'fs';
import * as path from 'path';
import { createRequire } from 'module';

const watch = process.argv.includes('--watch');
const production = process.argv.includes('--production');

// node-notifier (pulled in via @session-deck/core) finds its helper binaries (SnoreToast etc.) at
// path.resolve(__dirname, '../vendor/...'). Bundled, __dirname is out/, so they must sit in vendor/ next to it.
const notifierDir = path.dirname(createRequire(import.meta.url).resolve('node-notifier/package.json'));
fs.rmSync('vendor', { recursive: true, force: true });
fs.cpSync(path.join(notifierDir, 'vendor'), 'vendor', { recursive: true });

const ctx = await esbuild.context({
  entryPoints: ['src/main.ts'],
  bundle: true,
  platform: 'node',
  target: 'node22',
  format: 'cjs',
  outfile: 'out/main.js',
  external: ['node-pty'],
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
