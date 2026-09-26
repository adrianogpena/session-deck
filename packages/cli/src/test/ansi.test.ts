import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fit, fitAnsi, textWidth } from '../ansi';

// eslint-disable-next-line no-control-regex -- stripping SGR sequences to compare visible text
const strip = (s: string) => s.replace(/\x1b\[[0-9;]*m/g, '');

test('fit truncates with an ellipsis and pads to the exact width', () => {
  assert.equal(fit('hello world', 8), 'hello w…');
  assert.equal(fit('hi', 5), 'hi   ');
  assert.equal(textWidth(fit('日本語テキスト', 7)), 7);
});

test('fitAnsi keeps color sequences while truncating or padding to visible width', () => {
  const line = '\x1b[31mred\x1b[0m and \x1b[32mgreen text\x1b[0m';
  const cut = fitAnsi(line, 10);
  assert.equal(strip(cut), 'red and gr');
  assert.ok(cut.startsWith('\x1b[31mred'));
  assert.equal(strip(fitAnsi('\x1b[1mab\x1b[0m', 5)), 'ab   ');
});
