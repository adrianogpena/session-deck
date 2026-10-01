import { test } from 'node:test';
import assert from 'node:assert/strict';
import { extractFocusEvents, findChordKey, findDetachKey, findPlainKey, parseMouseSequence, splitKeys } from '../keys';

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

test('findChordKey recognizes Ctrl+K in every encoding, and only Ctrl+K', () => {
  assert.deepEqual(findChordKey('\x0b'), { index: 0, end: 1 });
  assert.deepEqual(findChordKey('\x1b[107;5u'), { index: 0, end: 8 });
  assert.deepEqual(findChordKey('ab\x1b[75;16;17;1;8;1_'), { index: 2, end: 19 });
  assert.equal(findChordKey('\x1b[75;16;107;1;0;1_'), null); // plain k
  assert.equal(findChordKey('\x1b[75;16;17;0;8;1_'), null); // key-up
  assert.equal(findChordKey('\x1b[75;16;0;1;10;1_'), null); // Ctrl+Alt+K (AltGr)
});

test("findChordKey exposes where its match ends, so a chord's resolving key already in the same chunk is not lost", () => {
  const m = findChordKey('\x0bN');
  assert.deepEqual(m, { index: 0, end: 1 });
  assert.equal(findPlainKey('\x0bN'.slice(m!.end), 'N'), 0);
});

test('findPlainKey recognizes an unmodified key press in every encoding', () => {
  assert.equal(findPlainKey('n', 'n'), 0);
  assert.equal(findPlainKey('ab n', 'n'), 3);
  assert.equal(findPlainKey('\x1b[78;49;110;1;0;1_', 'n'), 0);
  assert.equal(findPlainKey('\x1b[78;49;78;1;8;1_', 'N'), 0);
  assert.equal(findPlainKey('\x1b[78;49;110;0;0;1_', 'n'), -1); // key-up
  assert.equal(findPlainKey('x', 'n'), -1);
});

test('splitKeys keeps an SGR mouse report whole', () => {
  assert.deepEqual(splitKeys('\x1b[<64;10;5M\x1b[<65;10;5M'), ['\x1b[<64;10;5M', '\x1b[<65;10;5M']);
});

test('parseMouseSequence recognizes a wheel notch, direction, and a plain click, and rejects non-mouse input', () => {
  assert.deepEqual(parseMouseSequence('\x1b[<64;10;5M'), { wheel: 'up' });
  assert.deepEqual(parseMouseSequence('\x1b[<65;10;5M'), { wheel: 'down' });
  assert.deepEqual(parseMouseSequence('\x1b[<68;10;5M'), { wheel: 'up' }); // Shift+wheel-up
  assert.deepEqual(parseMouseSequence('\x1b[<69;10;5M'), { wheel: 'down' }); // Shift+wheel-down
  assert.deepEqual(parseMouseSequence('\x1b[<0;10;5M'), {}); // left-click press
  assert.deepEqual(parseMouseSequence('\x1b[<0;10;5m'), {}); // left-click release
  assert.equal(parseMouseSequence('\x1b[A'), undefined);
  assert.equal(parseMouseSequence('n'), undefined);
});

test('extractFocusEvents strips focus-in/out and reports the last one in the chunk, leaving unrelated input untouched', () => {
  assert.deepEqual(extractFocusEvents('\x1b[I'), { focused: true, rest: '' });
  assert.deepEqual(extractFocusEvents('\x1b[O'), { focused: false, rest: '' });
  assert.deepEqual(extractFocusEvents('n'), { focused: undefined, rest: 'n' });
  assert.deepEqual(extractFocusEvents('a\x1b[Ob\x1b[Ic'), { focused: true, rest: 'abc' });
});
