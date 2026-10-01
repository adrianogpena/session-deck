import { test } from 'node:test';
import assert from 'node:assert/strict';
import { PALETTE_COMMANDS, paletteMatches } from '../commands';

test('paletteMatches is an order-preserving, case/whitespace-insensitive subsequence filter', () => {
  const labels = ['New session', 'Rename', 'Move project to folder'];
  assert.deepEqual(paletteMatches('ns', labels), [0]);
  assert.deepEqual(paletteMatches('NS', labels), [0]);
  assert.deepEqual(paletteMatches('zz', labels), []); // no label contains two z's
  assert.deepEqual(paletteMatches('', labels), [0, 1, 2]);
  assert.deepEqual(paletteMatches('mptf', labels), [2]);
});

test('PALETTE_COMMANDS entries have a unique id and a non-empty label', () => {
  assert.ok(PALETTE_COMMANDS.length > 0);
  for (const command of PALETTE_COMMANDS) {
    assert.ok(command.label.trim().length > 0, `empty label for id ${command.id}`);
  }
  assert.equal(new Set(PALETTE_COMMANDS.map((c) => c.id)).size, PALETTE_COMMANDS.length);
});
