import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseDeckConfig } from '@session-deck/core';
import { CONFIG_FIELDS } from '../configFields';

const field = (label: string) => CONFIG_FIELDS.find((f) => f.label === label)!;
const base = () => parseDeckConfig('{}');

test('ui.maxSessionsListed accepts a positive integer and rejects everything else', () => {
  const f = field('ui.maxSessionsListed');
  assert.equal(f.apply(base(), '10')?.ui.maxSessionsListed, 10);
  assert.equal(f.apply(base(), '0'), undefined);
  assert.equal(f.apply(base(), '-5'), undefined);
  assert.equal(f.apply(base(), '3.5'), undefined);
  assert.equal(f.apply(base(), 'abc'), undefined);
});

test('ui.notifications toggles regardless of input', () => {
  const f = field('ui.notifications');
  assert.equal(f.apply(base(), '')?.ui.notifications, false);
  assert.equal(f.apply(f.apply(base(), '')!, '')?.ui.notifications, true);
});

test('tools.claude.command trims input and clears back to the default on blank input', () => {
  const f = field('tools.claude.command');
  assert.equal(f.apply(base(), '  claude-nightly  ')?.tools.claude.command, 'claude-nightly');
  assert.equal(f.apply(base(), '   ')?.tools.claude.command, undefined);
  assert.equal(f.display(base()), '(default: claude)');
  assert.equal(f.display(f.apply(base(), 'claude-nightly')!), 'claude-nightly');
});

test('tools.claude.args splits on whitespace and clears back to none on blank input', () => {
  const f = field('tools.claude.args');
  assert.deepEqual(f.apply(base(), '--model  opus')?.tools.claude.args, ['--model', 'opus']);
  assert.equal(f.apply(base(), '   ')?.tools.claude.args, undefined);
  assert.equal(f.display(base()), '(none)');
  assert.equal(f.display(f.apply(base(), '--allow-all')!), '--allow-all');
});

test('editValue returns the raw stored value, not the friendly default text', () => {
  const f = field('tools.copilot.command');
  assert.equal(f.editValue(base()), '');
  assert.equal(f.display(base()), '(default: copilot)');
});

test('ui.notifyStatuses accepts a space-separated list of known statuses and rejects an unknown one', () => {
  const f = field('ui.notifyStatuses');
  assert.deepEqual(f.apply(base(), 'waiting error')?.ui.notifyStatuses, ['waiting', 'error']);
  assert.equal(f.apply(base(), 'waiting bogus'), undefined);
  assert.equal(f.display(base()), 'waiting done error');
  assert.equal(f.display(f.apply(base(), '')!), '(none)');
});

test('ui.recentProjectsFirst toggles regardless of input, off by default', () => {
  const f = field('ui.recentProjectsFirst');
  assert.equal(f.display(base()), 'off');
  assert.equal(f.apply(base(), '')?.ui.recentProjectsFirst, true);
  assert.equal(f.apply(f.apply(base(), '')!, '')?.ui.recentProjectsFirst, false);
});

test('ui.recentSessionsFirst toggles regardless of input, on by default', () => {
  const f = field('ui.recentSessionsFirst');
  assert.equal(f.display(base()), 'on');
  assert.equal(f.apply(base(), '')?.ui.recentSessionsFirst, false);
  assert.equal(f.apply(f.apply(base(), '')!, '')?.ui.recentSessionsFirst, true);
});

test('ui.newSessionFullScreen toggles regardless of input, on by default', () => {
  const f = field('ui.newSessionFullScreen');
  assert.equal(f.display(base()), 'full screen');
  assert.equal(f.apply(base(), '')?.ui.newSessionFullScreen, false);
  assert.equal(f.display(f.apply(base(), '')!), 'preview pane');
  assert.equal(f.apply(f.apply(base(), '')!, '')?.ui.newSessionFullScreen, true);
});

test('ui.gitStatus toggles regardless of input, on by default', () => {
  const f = field('ui.gitStatus');
  assert.equal(f.display(base()), 'on');
  assert.equal(f.apply(base(), '')?.ui.gitStatus, false);
  assert.equal(f.apply(f.apply(base(), '')!, '')?.ui.gitStatus, true);
});

test('ui.expandCollapsedOnActiveJump toggles regardless of input, on by default', () => {
  const f = field('ui.expandCollapsedOnActiveJump');
  assert.equal(f.display(base()), 'on');
  assert.equal(f.apply(base(), '')?.ui.expandCollapsedOnActiveJump, false);
  assert.equal(f.apply(f.apply(base(), '')!, '')?.ui.expandCollapsedOnActiveJump, true);
});

test('tools.claude.enabled and tools.copilot.enabled toggle independently', () => {
  const claudeEnabled = field('tools.claude.enabled');
  const copilotEnabled = field('tools.copilot.enabled');
  assert.equal(claudeEnabled.display(base()), 'on');
  const off = claudeEnabled.apply(base(), '')!;
  assert.equal(off.tools.claude.enabled, false);
  assert.equal(claudeEnabled.display(off), 'off');
  // Toggling Claude off leaves Copilot untouched.
  assert.equal(copilotEnabled.display(off), 'on');
  assert.equal(claudeEnabled.display(claudeEnabled.apply(off, '')!), 'on');
});

test('CONFIG_FIELDS builds tools.*.* fields for every catalog agent, not just claude/copilot', () => {
  const f = field('tools.codex.enabled');
  assert.equal(f.display(base()), 'on');
  assert.equal(f.apply(base(), '')!.tools.codex.enabled, false);
});

test('trash.retentionDays accepts a positive integer and rejects everything else', () => {
  const f = field('trash.retentionDays');
  assert.equal(f.apply(base(), '7')?.trash.retentionDays, 7);
  assert.equal(f.apply(base(), '0'), undefined);
  assert.equal(f.display(base()), '30');
});
