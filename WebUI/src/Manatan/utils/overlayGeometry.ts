/**
 * Geometry helpers for placing OCR overlays over rendered page images.
 *
 * OCR block coordinates are normalised to the decoded image (0..1). When an `<img>` uses
 * `object-fit` the element box and the painted image differ (letterboxing), so the overlay
 * must be sized to the painted content box or every hitbox is stretched/shifted.
 */

export interface ContentBox {
    /** Offset of the painted image from the element box's left edge, in the same units as the box. */
    offsetX: number;
    /** Offset of the painted image from the element box's top edge, in the same units as the box. */
    offsetY: number;
    width: number;
    height: number;
}

const parsePositionComponent = (token: string | undefined, axis: 'x' | 'y'): number => {
    if (!token) return 0.5;
    const value = token.trim().toLowerCase();
    if (value === 'center') return 0.5;
    if (axis === 'x' && value === 'left') return 0;
    if (axis === 'x' && value === 'right') return 1;
    if (axis === 'y' && value === 'top') return 0;
    if (axis === 'y' && value === 'bottom') return 1;
    if (value.endsWith('%')) {
        const parsed = Number.parseFloat(value);
        if (Number.isFinite(parsed)) return parsed / 100;
    }
    return 0.5;
};

/**
 * Parse the common forms of a computed `object-position` value ("50% 50%", "left top", ...).
 * Length offsets are not supported and fall back to centring.
 */
export const parseObjectPosition = (objectPosition: string | undefined): { x: number; y: number } => {
    const tokens = (objectPosition || '').trim().split(/\s+/).filter(Boolean);
    if (tokens.length === 0) return { x: 0.5, y: 0.5 };
    if (tokens.length === 1) {
        const only = tokens[0].toLowerCase();
        if (only === 'top' || only === 'bottom') return { x: 0.5, y: parsePositionComponent(only, 'y') };
        return { x: parsePositionComponent(only, 'x'), y: 0.5 };
    }
    let [first, second] = tokens.map((token) => token.toLowerCase());
    // "top left" is a valid ordering; normalise to x, y.
    if (first === 'top' || first === 'bottom' || second === 'left' || second === 'right') {
        [first, second] = [second, first];
    }
    return { x: parsePositionComponent(first, 'x'), y: parsePositionComponent(second, 'y') };
};

/**
 * Compute where an image with the given natural size is painted inside an element box of
 * `boxWidth` x `boxHeight` for the given CSS `object-fit` / `object-position`.
 */
export const computeObjectFitContentBox = (
    boxWidth: number,
    boxHeight: number,
    naturalWidth: number,
    naturalHeight: number,
    objectFit: string | undefined = 'fill',
    objectPosition?: string,
): ContentBox => {
    const fullBox: ContentBox = { offsetX: 0, offsetY: 0, width: boxWidth, height: boxHeight };
    if (!(naturalWidth > 0) || !(naturalHeight > 0) || !(boxWidth > 0) || !(boxHeight > 0)) {
        return fullBox;
    }

    let scale: number;
    switch ((objectFit || 'fill').trim().toLowerCase()) {
        case 'contain':
            scale = Math.min(boxWidth / naturalWidth, boxHeight / naturalHeight);
            break;
        case 'cover':
            scale = Math.max(boxWidth / naturalWidth, boxHeight / naturalHeight);
            break;
        case 'none':
            scale = 1;
            break;
        case 'scale-down':
            scale = Math.min(1, Math.min(boxWidth / naturalWidth, boxHeight / naturalHeight));
            break;
        default:
            return fullBox;
    }

    const width = naturalWidth * scale;
    const height = naturalHeight * scale;
    const position = parseObjectPosition(objectPosition);
    return {
        offsetX: (boxWidth - width) * position.x,
        offsetY: (boxHeight - height) * position.y,
        width,
        height,
    };
};
