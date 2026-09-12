export type ReaderDirection = 'rtl' | 'ltr' | 'vertical';
export type ReaderFit = 'contain' | 'width';

export interface ReaderPreferences {
    direction: ReaderDirection;
    fit: ReaderFit;
    twoPage: boolean;
}

const KEY = 'manga_reader_reader_preferences_v1';
const EVENT = 'manga-reader-preferences';
const defaults: ReaderPreferences = { direction: 'rtl', fit: 'contain', twoPage: false };
let cachedRaw: string | null | undefined;
let cachedValue: ReaderPreferences = defaults;

export const readReaderPreferences = (): ReaderPreferences => {
    const raw = localStorage.getItem(KEY);
    if (raw === cachedRaw) return cachedValue;
    try {
        cachedValue = { ...defaults, ...JSON.parse(raw ?? '{}') };
    } catch {
        cachedValue = defaults;
    }
    cachedRaw = raw;
    return cachedValue;
};

export const writeReaderPreferences = (value: ReaderPreferences) => {
    const raw = JSON.stringify(value);
    localStorage.setItem(KEY, raw);
    cachedRaw = raw;
    cachedValue = value;
    window.dispatchEvent(new Event(EVENT));
};

export const subscribeReaderPreferences = (listener: () => void) => {
    window.addEventListener(EVENT, listener);
    window.addEventListener('storage', listener);
    return () => {
        window.removeEventListener(EVENT, listener);
        window.removeEventListener('storage', listener);
    };
};
