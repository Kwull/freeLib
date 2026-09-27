// `node --test`: the letter strip reads in the order of the name list (the server's collation:
// lower-cased sort keys in code point order).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { stripLetters, listOrder } from '../src/lib/utils/scripts.ts';

test('Cyrillic strip: Ы between Щ and Э, Ukrainian letters after Я, as in the list', () => {
  const firsts = ['а', 'щ', 'ы', 'ь', 'э', 'я', 'є', 'і', 'ї', 'ґ'];
  // the list order the server produces
  assert.deepEqual([...firsts].sort(), firsts);
  const letters: [string, number, number][] = firsts.map((l, i) => [l.toUpperCase(), 1, i]);
  const strip = stripLetters('cyr', letters).map((l) => l.letter).join('');
  assert.equal(strip, 'АБВГДЕЖЗИЙКЛМНОПРСТУФХЦЧШЩЫЬЭЮЯЄІЇҐ');
  // absent letters of the basic alphabet are there, disabled
  assert.equal(stripLetters('cyr', letters).find((l) => l.letter === 'Б')?.index, null);
});

test('strip letters are sorted like the rows they jump to', () => {
  const letters: [string, number, number][] = [['Я', 1, 0], ['Ы', 1, 1], ['Ґ', 1, 2], ['І', 1, 3]];
  const present = stripLetters('cyr', letters).filter((l) => l.index !== null).map((l) => l.letter);
  assert.deepEqual(present, ['Ы', 'Я', 'І', 'Ґ']);
  assert.ok(listOrder('Ы', 'Э') < 0 && listOrder('Я', 'Є') < 0 && listOrder('Ї', 'Ґ') < 0);
  // Latin and "#" are unchanged
  assert.equal(stripLetters('lat', [['B', 1, 0]]).map((l) => l.letter).join(''), 'ABCDEFGHIJKLMNOPQRSTUVWXYZ');
  assert.deepEqual(stripLetters('other', [['#', 1, 0], ['1', 1, 1]]).map((l) => l.letter), ['1', '#']);
});
