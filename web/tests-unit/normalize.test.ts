// `node --test` (Node ≥ 22.18 strips the types): the SPA's normalize must match the server's,
// using the vectors shared with server/crates/catalog/tests/normalize_vectors.rs.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { normalize, letterOf } from '../src/lib/utils/normalize.ts';

type Vector = { input: string; normalized: string; letter: string };
const doc = JSON.parse(
  readFileSync(new URL('../../docs/web/normalize-vectors.json', import.meta.url), 'utf8'),
) as { vectors: Vector[] };

test('normalize and letterOf match the server vectors', () => {
  assert.ok(doc.vectors.length > 20);
  for (const v of doc.vectors) {
    const n = normalize(v.input);
    assert.equal(n, v.normalized, `normalize(${JSON.stringify(v.input)})`);
    assert.equal(letterOf(n), v.letter, `letterOf(${JSON.stringify(n)})`);
  }
});
