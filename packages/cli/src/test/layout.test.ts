import { test } from 'node:test';
import assert from 'node:assert/strict';
import { computeLayout, ptySizeFor } from '../layout';

test('computeLayout splits side by side at 80+ columns using the sidebar percentage', () => {
  const layout = computeLayout(120, 40, 35, true);
  assert.equal(layout.mode, 'split');
  assert.deepEqual(layout.list, { x: 0, y: 2, width: 42, height: 37 });
  assert.equal(layout.dividerX, 42);
  assert.deepEqual(layout.preview, { x: 43, y: 2, width: 77, height: 37 });
  assert.deepEqual(ptySizeFor(layout), { cols: 77, rows: 35 });
});

test('computeLayout stacks the list above the preview between 50 and 79 columns', () => {
  const layout = computeLayout(70, 40, 35, true);
  assert.equal(layout.mode, 'stacked');
  assert.deepEqual(layout.list, { x: 0, y: 2, width: 70, height: 14 });
  assert.deepEqual(layout.preview, { x: 0, y: 16, width: 70, height: 23 });
});

test('computeLayout shows only the list below 50 columns, even with the sidebar hidden', () => {
  const layout = computeLayout(45, 30, 35, false);
  assert.equal(layout.mode, 'list');
  assert.equal(layout.preview, undefined);
  assert.equal(ptySizeFor(layout), undefined);
});

test('computeLayout gives the preview the whole area when the sidebar is hidden', () => {
  const layout = computeLayout(120, 40, 35, false);
  assert.equal(layout.list, undefined);
  assert.deepEqual(layout.preview, { x: 0, y: 2, width: 120, height: 37 });
});
