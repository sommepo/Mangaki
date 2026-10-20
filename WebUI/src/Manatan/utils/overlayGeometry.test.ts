import assert from 'node:assert/strict';
import test from 'node:test';

import { computeObjectFitContentBox, parseObjectPosition } from '@/Manatan/utils/overlayGeometry.ts';

test('fill (default) maps OCR coordinates to the whole element box', () => {
    assert.deepEqual(computeObjectFitContentBox(400, 600, 800, 1000), {
        offsetX: 0,
        offsetY: 0,
        width: 400,
        height: 600,
    });
});

test('contain letterboxes a portrait page inside a wider box (fit-height on a wide viewport)', () => {
    // 700x1000 page painted in a 1000x500 box -> scale 0.5 -> 350x500, centred horizontally.
    const box = computeObjectFitContentBox(1000, 500, 700, 1000, 'contain', '50% 50%');
    assert.deepEqual(box, { offsetX: 325, offsetY: 0, width: 350, height: 500 });
});

test('contain letterboxes vertically when the box is taller than the page (rotation to portrait)', () => {
    // 1400x1000 spread painted in a 400x900 box -> scale 400/1400 -> 400x285.7, centred vertically.
    const box = computeObjectFitContentBox(400, 900, 1400, 1000, 'contain');
    assert.equal(box.offsetX, 0);
    assert.equal(box.width, 400);
    assert.ok(Math.abs(box.height - 285.714) < 0.01);
    assert.ok(Math.abs(box.offsetY - (900 - box.height) / 2) < 1e-9);
});

test('a hitbox normalised to the image lands on the same image pixel after letterboxing', () => {
    const naturalWidth = 700;
    const naturalHeight = 1000;
    const box = computeObjectFitContentBox(1000, 500, naturalWidth, naturalHeight, 'contain');
    // A bubble at image pixel (350, 500) (the centre) is normalised to (0.5, 0.5).
    const screenX = box.offsetX + 0.5 * box.width;
    const screenY = box.offsetY + 0.5 * box.height;
    assert.equal(screenX, 500);
    assert.equal(screenY, 250);
});

test('contain is scale invariant, so zoomed (transformed) boxes keep proportional hitboxes', () => {
    const base = computeObjectFitContentBox(1000, 500, 700, 1000, 'contain');
    const zoomed = computeObjectFitContentBox(2500, 1250, 700, 1000, 'contain');
    assert.ok(Math.abs(zoomed.offsetX - base.offsetX * 2.5) < 1e-9);
    assert.ok(Math.abs(zoomed.width - base.width * 2.5) < 1e-9);
    assert.ok(Math.abs(zoomed.height - base.height * 2.5) < 1e-9);
});

test('scale-down never upscales and cover overflows the box', () => {
    assert.deepEqual(computeObjectFitContentBox(1000, 1000, 200, 100, 'scale-down'), {
        offsetX: 400,
        offsetY: 450,
        width: 200,
        height: 100,
    });
    const cover = computeObjectFitContentBox(100, 100, 200, 100, 'cover');
    assert.deepEqual(cover, { offsetX: -50, offsetY: 0, width: 200, height: 100 });
});

test('unknown natural size falls back to the element box', () => {
    assert.deepEqual(computeObjectFitContentBox(300, 400, 0, 0, 'contain'), {
        offsetX: 0,
        offsetY: 0,
        width: 300,
        height: 400,
    });
});

test('object-position keywords and percentages are honoured', () => {
    assert.deepEqual(parseObjectPosition('50% 50%'), { x: 0.5, y: 0.5 });
    assert.deepEqual(parseObjectPosition('left top'), { x: 0, y: 0 });
    assert.deepEqual(parseObjectPosition('top right'), { x: 1, y: 0 });
    assert.deepEqual(parseObjectPosition('25% 100%'), { x: 0.25, y: 1 });
    const box = computeObjectFitContentBox(1000, 500, 700, 1000, 'contain', 'right center');
    assert.equal(box.offsetX, 650);
});
