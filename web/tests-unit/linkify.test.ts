// `node --test`: plain-text URLs in annotations (utils/linkify.ts) — what becomes a link and
// what stays text.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { cleanUrl, splitUrls } from '../src/lib/utils/linkify.ts';

test('trailing punctuation is not part of a URL; balanced parentheses are', () => {
  assert.equal(cleanUrl('https://example.org/a.'), 'https://example.org/a');
  assert.equal(cleanUrl('https://example.org/a),'), 'https://example.org/a');
  assert.equal(cleanUrl('https://en.wikipedia.org/wiki/Foo_(bar)'), 'https://en.wikipedia.org/wiki/Foo_(bar)');
  assert.equal(cleanUrl('https://en.wikipedia.org/wiki/Foo_(bar)).'), 'https://en.wikipedia.org/wiki/Foo_(bar)');
  assert.equal(cleanUrl('http://'), null);
});

test('only http(s) URLs are split out', () => {
  const parts = splitUrls('See https://example.org/x, or javascript:alert(1) and ftp://h/f.');
  assert.deepEqual(parts, [
    { text: 'See ' },
    { text: 'https://example.org/x', url: 'https://example.org/x' },
    { text: ', or javascript:alert(1) and ftp://h/f.' },
  ]);
  assert.deepEqual(splitUrls('no links here'), [{ text: 'no links here' }]);
  // markup-looking text is just text: the caller builds anchors with DOM calls
  const tricky = splitUrls('https://e.org/"><img src=x onerror=alert(1)>');
  assert.equal(tricky[0].url, 'https://e.org/');
});
