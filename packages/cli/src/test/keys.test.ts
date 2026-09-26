import { test } from 'node:test';
import assert from 'node:assert/strict';
import { findDetachKey, splitKeys } from '../keys';

test('splitKeys separates typed characters and keeps escape sequences whole', () => {
  assert.deepEqual(splitKeys('>>>'), ['>', '>', '>']);
  assert.deepEqual(splitKeys('0?'), ['0', '?']);
  assert.deepEqual(splitKeys('\x1b[Aj\x1b[B'), ['\x1b[A', 'j', '\x1b[B']);
  assert.deepEqual(splitKeys('\x1bOQ\x1b[12~'), ['\x1bOQ', '\x1b[12~']);
  assert.deepEqual(splitKeys('\x1b'), ['\x1b']);
  assert.deepEqual(splitKeys('é🙂'), ['é', '🙂']);
});

test('findDetachKey recognizes Ctrl+Q in every encoding, and only Ctrl+Q', () => {
  assert.equal(findDetachKey('\x11'), 0);
  assert.equal(findDetachKey('\x1b[113;5u'), 0);
  assert.equal(findDetachKey('ab\x1b[81;16;17;1;8;1_'), 2);
  assert.equal(findDetachKey('\x1b[81;16;113;1;0;1_'), -1); // plain q
  assert.equal(findDetachKey('\x1b[81;16;17;0;8;1_'), -1); // key-up
  assert.equal(findDetachKey('\x1b[81;16;0;1;10;1_'), -1); // Ctrl+Alt+Q (AltGr)
});
