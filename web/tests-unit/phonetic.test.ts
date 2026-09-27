// `node --test`: the SPA's phonetic key and name ranking (the name filter of the authors /
// series lists) must match the server's, using the vectors shared with
// server/crates/catalog/tests/phonetic_vectors.rs.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { normalize } from '../src/lib/utils/normalize.ts';
import { phoneticKey, nameRank } from '../src/lib/utils/phonetic.ts';

type Doc = {
  keys: { input: string; key: string }[];
  ranks: { name: string; query: string; rank: number }[];
};
const doc = JSON.parse(
  readFileSync(new URL('../../docs/web/phonetic-vectors.json', import.meta.url), 'utf8'),
) as Doc;

test('phoneticKey matches the server vectors', () => {
  assert.ok(doc.keys.length > 50);
  for (const v of doc.keys) assert.equal(phoneticKey(normalize(v.input)), v.key, `phoneticKey(${JSON.stringify(v.input)})`);
});

test('nameRank matches the server vectors', () => {
  for (const v of doc.ranks) {
    const tokens = normalize(v.query).split(' ').filter(Boolean);
    assert.equal(nameRank(normalize(v.name), tokens), v.rank, `nameRank(${v.name}, ${v.query})`);
  }
});

test('asimov spellings share a key', () => {
  for (const w of ['asimov', 'azimov', 'asimow', 'azimoff', 'Азимов']) assert.equal(phoneticKey(normalize(w)), 'asimov', w);
});
