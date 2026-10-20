import assert from 'node:assert/strict';
import test from 'node:test';

import { buildSentenceFuriganaFromLookup } from '@/Manatan/utils/japaneseFurigana.ts';

test('sentence furigana accepts the { terms } shape returned by lookupYomitan', async () => {
    const calls: number[] = [];
    const out = await buildSentenceFuriganaFromLookup(
        '日本語です',
        async (_text, index) => {
            calls.push(index);
            if (index === 0) return { terms: [{ headword: '日本語', reading: 'にほんご', matchLen: 3 }] };
            return { terms: [] };
        },
        { language: 'japanese' },
    );

    assert.equal(out, '<ruby>日本語<rt>にほんご</rt></ruby>です');
    // One lookup for the matched word, then one per unmatched trailing character.
    assert.deepEqual(calls.length, 3);
});

test('sentence furigana still accepts a bare term array', async () => {
    const out = await buildSentenceFuriganaFromLookup(
        '日本',
        async (_text, index) => (index === 0 ? [{ headword: '日本', reading: 'にほん', matchLen: 2 }] : []),
        { language: 'japanese' },
    );
    assert.equal(out, '<ruby>日本<rt>にほん</rt></ruby>');
});

test('sentence furigana returns the plain sentence while dictionaries are loading', async () => {
    const out = await buildSentenceFuriganaFromLookup('日本', async () => 'loading', { language: 'japanese' });
    assert.equal(out, '日本');
});
