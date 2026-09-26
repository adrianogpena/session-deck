import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fuzzyMatch } from '../fuzzyMatch';

test('fuzzyMatch matches a contiguous substring', () => {
  assert.equal(fuzzyMatch('fix', 'fix the login bug').matched, true);
});

test('fuzzyMatch matches non-contiguous characters in order', () => {
  assert.equal(fuzzyMatch('fnc', 'function').matched, true);
});

test('fuzzyMatch rejects characters out of order', () => {
  assert.equal(fuzzyMatch('cnf', 'function').matched, false);
});

test('fuzzyMatch rejects a query with a character missing from the text', () => {
  assert.equal(fuzzyMatch('fnz', 'function').matched, false);
});

test('fuzzyMatch is case-insensitive', () => {
  assert.equal(fuzzyMatch('FIX', 'fix the login bug').matched, true);
  assert.equal(fuzzyMatch('fix', 'FIX THE LOGIN BUG').matched, true);
});

test('fuzzyMatch treats an empty query as matching everything with the best score', () => {
  const result = fuzzyMatch('', 'anything at all');
  assert.equal(result.matched, true);
  assert.equal(result.score, 0);
});

test('fuzzyMatch scores a tighter, earlier match better than a loose, later one', () => {
  const tight = fuzzyMatch('fn', 'function');
  const loose = fuzzyMatch('fn', 'far from concrete');
  assert.ok(tight.matched && loose.matched);
  assert.ok(tight.score < loose.score, `expected tight match to score better: tight=${tight.score} loose=${loose.score}`);
});

test('fuzzyMatch prefers an earlier match when spans are equal length', () => {
  const early = fuzzyMatch('ab', 'ab----------');
  const late = fuzzyMatch('ab', '----------ab');
  assert.ok(early.matched && late.matched);
  assert.ok(early.score < late.score);
});
