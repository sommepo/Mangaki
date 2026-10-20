import assert from 'node:assert/strict';
import test from 'node:test';

import { calculateUpdatedFields, escapeAnkiSearchText } from '@/Manatan/utils/anki.ts';

test('escapeAnkiSearchText makes wildcard and quote characters literal', () => {
    assert.equal(escapeAnkiSearchText('食べる'), '食べる');
    assert.equal(escapeAnkiSearchText('a_b*c"d\\e'), 'a\\_b\\*c\\"d\\\\e');
    assert.equal(`"Expression:${escapeAnkiSearchText('*')}"`, '"Expression:\\*"');
});

test('calculateUpdatedFields honours per-field update modes', () => {
    const updated = calculateUpdatedFields(
        { Sentence: { value: 'old' }, Notes: { value: 'kept' }, Empty: { value: '' } },
        { Sentence: 'new', Notes: 'ignored', Empty: 'filled' },
        { Sentence: 'append', Notes: 'skip', Empty: 'fill-if-empty' },
    );
    assert.deepEqual(updated, { Sentence: 'old new', Notes: 'kept', Empty: 'filled' });
});
