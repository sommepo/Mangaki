/**
 * Updated AnkiConnect API call with automatic permission handshake
 */
import { getDownscaledSize } from './image';
import { ForkApi } from '@/features/fork/ForkApi';

const ANKI_LOG_COOLDOWN_MS = 60000;
let lastAnkiLogAt = 0;

const isConnectionError = (error: any): boolean => {
    if (!error) return false;
    const message = String(error?.message ?? error);
    if (error instanceof TypeError && message.includes('Failed to fetch')) {
        return true;
    }
    return (
        message.includes('ERR_CONNECTION_REFUSED') ||
        message.includes('Connection refused') ||
        message.includes('NetworkError')
    );
};

export const logAnkiError = (message: string, error: any) => {
    if (isConnectionError(error)) {
        return;
    }
    const now = Date.now();
    if (now - lastAnkiLogAt < ANKI_LOG_COOLDOWN_MS) {
        return;
    }
    lastAnkiLogAt = now;
    console.warn(message, error);
};

/**
 * `response`: Anki itself answered with an error (the request reached it and was handled).
 * `ambiguous`: the request may have reached Anki but no usable answer came back.
 */
class AnkiRequestError extends Error {
    constructor(
        message: string,
        readonly kind: 'response' | 'ambiguous',
    ) {
        super(message);
    }
}

/** Actions that change the collection; a blind retry could duplicate their effect. */
const MUTATING_ANKI_ACTIONS = new Set([
    'addNote',
    'addNotes',
    'updateNoteFields',
    'updateNote',
    'storeMediaFile',
    'deleteNotes',
]);

/**
 * Escape a value for use inside a double-quoted Anki search term
 * (e.g. `"Expression:${value}"`), so `*`, `_`, `"` and `\` match literally.
 */
export const escapeAnkiSearchText = (value: string): string => value.replace(/([\\"*_])/g, '\\$1');

async function ankiConnect(
    action: string,
    params: Record<string, any>,
    url: string,
) {
    const timeoutMs = 20000;
    const isMutation = MUTATING_ANKI_ACTIONS.has(action);
    const fetchWithTimeout = async (body: Record<string, any>) => {
        const controller = new AbortController();
        const timeout = window.setTimeout(() => controller.abort(), timeoutMs);
        try {
            const isOnDeviceBridge = /^http:\/\/(127\.0\.0\.1|localhost):18765\/?$/i.test(url);
            const token = isOnDeviceBridge ? (window as any).MangaReaderNative?.getAnkiBridgeToken?.() : undefined;
            const res = await fetch(url, {
                method: "POST",
                headers: { 'Content-Type': 'application/json', ...(token ? { 'X-Manga-Reader-Bridge': token } : {}) },
                body: JSON.stringify(body),
                signal: controller.signal,
            });
            let json: any;
            try {
                json = await res.json();
            } catch {
                throw new AnkiRequestError(`Anki returned an unreadable response (HTTP ${res.status}).`, 'ambiguous');
            }
            if (json?.error) throw new AnkiRequestError(String(json.error), 'response');
            return json?.result;
        } catch (error: any) {
            if (error?.name === 'AbortError') {
                throw new AnkiRequestError(
                    'AnkiConnect request timed out. Make sure AnkiDroid is running and permission is granted.',
                    'ambiguous',
                );
            }
            throw error;
        } finally {
            window.clearTimeout(timeout);
        }
    };

    const execute = async () => fetchWithTimeout({ action, params, version: 6 });

    try {
        return await execute();
    } catch (e: any) {
        const errorMessage = e?.message ?? String(e);
        // Anki answered with an error (duplicate, missing deck, ...): a permission handshake
        // cannot change that, and re-sending a mutation must never be done blindly.
        if (e instanceof AnkiRequestError && e.kind === 'response') {
            throw new Error(errorMessage);
        }
        if (e instanceof AnkiRequestError && e.kind === 'ambiguous') {
            if (isMutation) {
                throw new Error(
                    `${errorMessage} The ${action} request may already have been applied; check Anki before retrying to avoid a duplicate card.`,
                );
            }
            throw new Error(errorMessage);
        }

        // Network-level failure (connection refused / CORS preflight blocked): attempt the
        // permission handshake and, if granted, retry once.
        let permissionGranted = false;
        try {
            const permResult = await fetchWithTimeout({ action: "requestPermission", version: 6 });
            permissionGranted = Boolean(permResult && permResult.permission === 'granted');
        } catch (handshakeError) {
            logAnkiError("Anki handshake failed", handshakeError);
        }
        if (permissionGranted) {
            // A dropped connection can also surface as a TypeError after Anki processed the
            // request, so never let the retry of an add knowingly create a duplicate.
            const retryParams =
                action === 'addNote' && params?.note?.options?.allowDuplicate
                    ? {
                          ...params,
                          note: { ...params.note, options: { ...params.note.options, allowDuplicate: false } },
                      }
                    : params;
            try {
                return await fetchWithTimeout({ action, params: retryParams, version: 6 });
            } catch (retryError: any) {
                const retryMessage = retryError?.message ?? String(retryError);
                if (action === 'addNote' && /duplicate/i.test(retryMessage)) {
                    throw new Error(
                        'The note may already have been added by the first attempt (Anki reported a duplicate on retry). Check Anki before adding again.',
                    );
                }
                throw new Error(retryMessage);
            }
        }

        // Standard error handling if handshake doesn't resolve the issue
        if (e instanceof TypeError && errorMessage.includes("Failed to fetch")) {
            throw new Error("Connection blocked. Please click 'Yes' on the Anki permission popup.");
        }
        throw new Error(errorMessage);
    }
}
/**
 * Check connection and get version
 */
export async function getAnkiVersion(url: string) {
    try {
        const ver = await ankiConnect("version", {}, url);
        return { ok: true, version: ver };
    } catch (e) {
        return { ok: false, error: e };
    }
}

/**
 * Get all deck names
 */
export async function getDeckNames(url: string): Promise<string[]> {
    return await ankiConnect("deckNames", {}, url);
}

/**
 * Get all model names (Card Types)
 */
export async function getModelNames(url: string): Promise<string[]> {
    return await ankiConnect("modelNames", {}, url);
}

/**
 * Get fields for a specific model
 */
export async function getModelFields(url: string, modelName: string): Promise<string[]> {
    return await ankiConnect("modelFieldNames", { modelName }, url);
}

/**
 * Find notes based on a query
 */
export async function findNotes(url: string, query: string): Promise<number[]> {
    const res = await ankiConnect("findNotes", { query }, url);
    return res || [];
}

/**
 * Get detailed information about notes
 */
export async function notesInfo(url: string, notes: number[]): Promise<any[]> {
    return await ankiConnect("notesInfo", { notes }, url);
}

/** Ask Anki whether notes can be added without creating duplicates. */
export async function canAddNotes(
    url: string,
    notes: Array<{
        deckName: string;
        modelName: string;
        fields: Record<string, string>;
    }>,
): Promise<boolean[]> {
    return await ankiConnect("canAddNotes", { notes }, url);
}

/**
 * Update a note, including fields and optional media
 */
export async function updateNote(
    url: string,
    note: {
        id: number;
        fields: Record<string, string>;
        picture?: { url?: string; data?: string; filename: string; fields: string[] } | Array<{ url?: string; data?: string; filename: string; fields: string[] }>;
        audio?: { url?: string; data?: string; filename: string; fields: string[] } | Array<{ url?: string; data?: string; filename: string; fields: string[] }>;
    }
) {
    // Both AnkiConnect (via updateNoteFields) and our Android bridge handle field updates.
    // Our Android bridge specifically handles "picture" and "audio" in the note object for updates.
    // For Desktop AnkiConnect, we should ideally use storeMediaFile then update fields,
    // but the most compatible way for now is to just call updateNoteFields.
    return await ankiConnect("updateNoteFields", { note }, url);
}

/**
 * Deprecated: Use updateNote instead
 */
export async function updateNoteFields(url: string, id: number, fields: Record<string, string>) {
    return await updateNote(url, { id, fields });
}

/**
 * Calculate updated fields based on modes (Append, Prepend, etc.)
 */
export function calculateUpdatedFields(
    currentFields: Record<string, { value: string }>,
    newFields: Record<string, string>,
    modes: Record<string, string>
): Record<string, string> {
    const updated: Record<string, string> = {};

    // We iterate over newFields because those are the ones we might want to change.
    // We should also consider fields that are in modes but NOT in newFields if we want to support 'skip' explicitly,
    // but usually newFields contains all mapped fields.
    for (const [field, newValue] of Object.entries(newFields)) {
        const currentVal = currentFields[field]?.value || '';
        const mode = modes[field] || 'overwrite';

        switch (mode) {
            case 'overwrite':
                updated[field] = newValue;
                break;
            case 'overwrite-if-available':
                if (newValue && newValue.trim()) {
                    updated[field] = newValue;
                } else {
                    updated[field] = currentVal;
                }
                break;
            case 'append':
                if (!currentVal) {
                    updated[field] = newValue;
                } else if (newValue) {
                    const separator = (currentVal.includes('<') || newValue.includes('<')) ? '<br>' : ' ';
                    updated[field] = currentVal + separator + newValue;
                } else {
                    updated[field] = currentVal;
                }
                break;
            case 'prepend':
                if (!currentVal) {
                    updated[field] = newValue;
                } else if (newValue) {
                    const separator = (currentVal.includes('<') || newValue.includes('<')) ? '<br>' : ' ';
                    updated[field] = newValue + separator + currentVal;
                } else {
                    updated[field] = currentVal;
                }
                break;
            case 'fill-if-empty':
                if (!currentVal || !currentVal.trim()) {
                    updated[field] = newValue;
                } else {
                    updated[field] = currentVal;
                }
                break;
            case 'skip':
                updated[field] = currentVal;
                break;
            default:
                updated[field] = newValue;
        }
    }
    return updated;
}

/**
 * Open the Anki Browser to a specific query
 */
export async function guiBrowse(url: string, query: string) {
    return await ankiConnect("guiBrowse", { query }, url);
}

/**
 * Add a new note
 */
export async function addNote(
    url: string, 
    deckName: string, 
    modelName: string, 
    fields: Record<string, string>, 
    tags: string[] = [],
    picture?: { url?: string; data?: string; filename: string; fields: string[] },
    audio?: { url?: string; data?: string; filename: string; fields: string[] } | Array<{ url?: string; data?: string; filename: string; fields: string[] }>,
    options?: {
        allowDuplicate?: boolean;
        duplicateScope?: string;
        recordMiningEvent?: boolean;
    }
) {
    const params: any = {
        note: {
            deckName,
            modelName,
            fields,
            tags,
            options: {
                allowDuplicate: options?.allowDuplicate ?? false,
                duplicateScope: options?.duplicateScope ?? 'deck'
            }
        }
    };

    if (picture) {
        params.note.picture = [picture];
    }

    if (audio) {
        params.note.audio = Array.isArray(audio) ? audio : [audio];
    }

    const result = await ankiConnect("addNote", params, url);
    if (result !== null && result !== undefined && result !== false && options?.recordMiningEvent !== false) {
        ForkApi.recordEvent('card_mined', { uniqueKey: String(result) }).catch(() => undefined);
    }
    return result;
}

/**
 * Get the most recent card ID created today
 */
async function getLastCardId(url: string) {
    const notesToday = await ankiConnect(
        "findNotes",
        { query: "added:1" },
        url,
    );
    if (!notesToday || !Array.isArray(notesToday)) {
        return undefined;
    }
    const id = notesToday.reduce<number | undefined>(
        (latest, current) =>
            typeof current === 'number' && (latest === undefined || current > latest) ? current : latest,
        undefined,
    );
    return id;
}

/**
 * Calculate card age in minutes
 */
function getCardAgeInMin(id: number) {
    return Math.floor((Date.now() - id) / 60000);
}

/**
 * Fetch image, downscale it if it exceeds max dimensions (keeping aspect ratio), 
 * and convert to base64 webp using Image element (better CORS handling)
 */
export async function imageUrlToBase64Webp(
    imageUrl: string,
    quality: number = 0.92,
    maxWidth?: number,
    maxHeight?: number
): Promise<string | null> {
    return new Promise((resolve) => {
        const img = new Image();
        // Use anonymous to attempt to get CORS headers, but handle failure
        img.crossOrigin = "anonymous";
        
        img.onload = () => {
            try {
                const { width, height } = getDownscaledSize(img.width, img.height, maxWidth, maxHeight);

                const canvas = new OffscreenCanvas(width, height);
                const ctx = canvas.getContext('2d');
                if (!ctx) throw new Error('No context');

                ctx.drawImage(img, 0, 0, width, height);
                
                canvas.convertToBlob({ type: 'image/webp', quality })
                    .then(blob => {
                        const reader = new FileReader();
                        reader.onloadend = () => resolve(reader.result as string);
                        reader.onerror = () => resolve(null);
                        reader.readAsDataURL(blob);
                    })
                    .catch(err => {
                        console.error("Blob conversion failed", err);
                        resolve(null);
                    });
            } catch (e) {
                console.error("Canvas operation failed (likely tainted)", e);
                resolve(null);
            }
        };

        img.onerror = () => {
            console.error("Failed to load image for Anki conversion");
            resolve(null);
        };

        img.src = imageUrl;
    });
}

/**
 * HTML inheritance logic adapted from asbplayer
 * Original Author: killergerbah
 * License: MIT
 * Source: https://github.com/killergerbah/asbplayer
 */


const htmlTagRegexString = '<([^/ >])*[^>]*>(.*?)</\\1>';

// Given <a><b>content</b></a> return ['<a><b>content</b></a>', '<b>content</b>', 'content']
const tagContent = (html: string) => {
    const htmlTagRegex = new RegExp(htmlTagRegexString);
    let content = html;
    let contents = [html];

    while (true) {
        const match = htmlTagRegex.exec(content);

        if (match === null || match.length < 3) {
            break;
        }

        content = match[2];
        contents.push(content);
    }

    return contents;
};

export const inheritHtmlMarkup = (original: string, markedUp: string) => {
    // If there is no markup to inherit, just return the original plain text
    if (!markedUp) return original;
    
    const htmlTagRegex = new RegExp(htmlTagRegexString, 'ig');
    const markedUpWithoutBreaklines = markedUp.replaceAll('<br>', '');
    let inherited = original;

    // Safety brake to prevent infinite loops if regex fails to advance
    let safetyCounter = 0; 

    while (safetyCounter++ < 100) {
        const match = htmlTagRegex.exec(markedUpWithoutBreaklines);

        if (match === null || match.length < 3) {
            break;
        }

        let newInherited = inherited;

        // Only try to apply the tag if the new string doesn't already have this exact tag+content
        if (!inherited.includes(match[0])) {
            const candidateTargets = tagContent(match[2]);

            // Try to find the inner text in the new string and wrap it
            for (const target of candidateTargets) {
                // Skip very short targets to avoid accidentally bolding single letters like "a" or "I"
                if (target.trim().length < 2 && !target.match(/[\u3000-\u303f\u3040-\u309f\u30a0-\u30ff\uff00-\uff9f\u4e00-\u9faf\u3400-\u4dbf]/)) {
                     continue; 
                }
                
                // If we find the text content in our new string, apply the tag from the match
                if (inherited.includes(target)) {
                    newInherited = inherited.replace(target, match[0]); 
                }

                if (newInherited !== inherited) {
                    break;
                }
            }
        }

        inherited = newInherited;
    }

    return inherited;
};


/**
 * Update the last created Anki card with image and/or sentence
 */
export async function updateLastCard(
    ankiConnectUrl: string,
    imageUrl: string | undefined, 
    sentence: string,
    pictureField: string,
    sentenceField: string,
    quality: number,
    preEncodedBase64?: string,
    audioField?: string,
    audioBase64?: string,
    downscaleMaxWidth?: number,
    downscaleMaxHeight?: number
) {
    // Find the last card
    const id = await getLastCardId(ankiConnectUrl);

    if (!id) {
        throw new Error("Could not find recent card (no cards created today)");
    }

    if (getCardAgeInMin(id) >= 5) {
        throw new Error("Card created over 5 minutes ago");
    }

    const fields: Record<string, any> = {};
    
    // Handle sentence field (if specified)
    if (sentenceField && sentenceField.trim() && sentence) {
        try {
            const noteInfo = await ankiConnect('notesInfo', { notes: [id] }, ankiConnectUrl);
            
            let finalSentence = sentence;

            // If the field exists in Anki, try to merge its HTML tags
            if (noteInfo && noteInfo[0] && noteInfo[0].fields && noteInfo[0].fields[sentenceField]) {
                const currentAnkiText = noteInfo[0].fields[sentenceField].value;
                finalSentence = inheritHtmlMarkup(sentence, currentAnkiText);
            }

            fields[sentenceField] = finalSentence;

        } catch (e) {
            console.warn("Failed to fetch existing note info, overwriting sentence without preserving HTML.", e);
            fields[sentenceField] = sentence;
        }
    }
    
    const updatePayload: any = {
        note: {
            id,
            fields,
        },
    };

    // Handle picture field (if specified)
    if (pictureField && pictureField.trim()) {
        let rawData: string | null = null;

        if (preEncodedBase64) {
            rawData = preEncodedBase64.includes('base64,') 
                ? preEncodedBase64.split(';base64,')[1] 
                : preEncodedBase64;
        } else if (imageUrl) {
            const fullBase64 = await imageUrlToBase64Webp(imageUrl, quality, downscaleMaxWidth, downscaleMaxHeight);
            if (!fullBase64) throw new Error("Failed to process image (CORS or Load Error)");
            rawData = fullBase64.split(';base64,')[1];
        }

        if (rawData) {
            // Clear existing image first
            fields[pictureField] = ""; 
            updatePayload.note.picture = {
                filename: `manatan_${id}.webp`,
                data: rawData,
                fields: [pictureField],
            };
        }
    }

    // Handle audio field (if specified)
    if (audioField && audioField.trim() && audioBase64) {
        const rawAudio = audioBase64.includes('base64,')
            ? audioBase64.split(';base64,')[1]
            : audioBase64;

        if (rawAudio) {
            fields[audioField] = '';
            const extension = audioBase64.startsWith('data:audio/mp4')
                ? 'm4a'
                : audioBase64.startsWith('data:audio/ogg')
                    ? 'ogg'
                    : audioBase64.startsWith('data:audio/wav')
                        ? 'wav'
                        : 'webm';
            updatePayload.note.audio = {
                filename: `manatan_audio_${id}.${extension}`,
                data: rawAudio,
                fields: [audioField],
            };
        }
    }

    await ankiConnect("updateNoteFields", updatePayload, ankiConnectUrl);
}
