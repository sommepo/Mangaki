import { GLOBAL_READER_SETTINGS_MANGA } from '@/features/manga/Manga.constants';
import { doesMetadataKeyExistIn } from '@/features/metadata/Metadata.utils';
import { convertFromGqlMeta } from '@/features/metadata/services/MetadataConverter';
import { IReaderSettings, ReaderPageScaleMode, ReadingDirection, ReadingMode } from '@/features/reader/Reader.types';
import { updateReaderSettings } from '@/features/reader/settings/ReaderSettingsMetadata';
import { ReaderPreferences } from '@/features/fork/ReaderPreferences';
import { requestManager } from '@/lib/requests/RequestManager';

/**
 * The manga reader is the inherited Manatan reader, whose defaults are stored as global reader
 * metadata. These are the settings the Mangaki Settings page edits, so Settings and the reader's
 * quick controls share one source of truth.
 */
export type MangakiReaderSettings = Pick<IReaderSettings, 'readingMode' | 'readingDirection' | 'pageScaleMode'>;

const MIGRATION_KEY = 'mangaki_reader_defaults_v1';
const LEGACY_PREFERENCES_KEY = 'manga_reader_reader_preferences_v1';

/** Japanese manga default (brief §6): paged, right-to-left, whole page visible. */
const MANGA_DEFAULTS: MangakiReaderSettings = {
    readingMode: ReadingMode.SINGLE_PAGE,
    readingDirection: ReadingDirection.RTL,
    pageScaleMode: ReaderPageScaleMode.SCREEN,
};

/** Maps the earlier fork-only preference object (never read by the real reader) onto reader settings. */
export const legacyPreferencesToReaderSettings = (
    preferences: Partial<ReaderPreferences>,
): Partial<MangakiReaderSettings> => {
    const result: Partial<MangakiReaderSettings> = {};
    if (preferences.direction === 'vertical') result.readingMode = ReadingMode.CONTINUOUS_VERTICAL;
    else if (preferences.direction === 'rtl' || preferences.direction === 'ltr') {
        result.readingDirection = preferences.direction === 'rtl' ? ReadingDirection.RTL : ReadingDirection.LTR;
        result.readingMode = preferences.twoPage ? ReadingMode.DOUBLE_PAGE : ReadingMode.SINGLE_PAGE;
    }
    if (preferences.fit === 'width') result.pageScaleMode = ReaderPageScaleMode.WIDTH;
    if (preferences.fit === 'contain') result.pageScaleMode = ReaderPageScaleMode.SCREEN;
    return result;
};

export const writeReaderDefaults = async (settings: Partial<MangakiReaderSettings>) => {
    const entries = Object.entries(settings) as [keyof MangakiReaderSettings, number][];
    for (const [key, value] of entries) {
        // Sequential: each write is a separate global-meta request on the loopback server.
        // eslint-disable-next-line no-await-in-loop
        await updateReaderSettings(GLOBAL_READER_SETTINGS_MANGA, key, value, true);
    }
};

const readLegacyPreferences = (): Partial<ReaderPreferences> | undefined => {
    try {
        const raw = localStorage.getItem(LEGACY_PREFERENCES_KEY);
        return raw ? JSON.parse(raw) : undefined;
    } catch {
        return undefined;
    }
};

/**
 * One-time: seed manga defaults for any reader setting the user has never chosen, carrying over
 * choices made in the earlier fork-only Settings controls. Existing global choices are kept.
 */
export const applyMangakiReaderDefaults = async () => {
    try {
        if (localStorage.getItem(MIGRATION_KEY) === 'done') return;
    } catch {
        return;
    }
    const { data } = await requestManager.getGlobalMeta().response;
    const meta = convertFromGqlMeta(data?.metas.nodes);
    const legacy = readLegacyPreferences();
    const wanted: Partial<MangakiReaderSettings> = {
        ...MANGA_DEFAULTS,
        ...(legacy ? legacyPreferencesToReaderSettings(legacy) : {}),
    };
    const missing = Object.fromEntries(
        Object.entries(wanted).filter(([key]) => !doesMetadataKeyExistIn(meta, key)),
    ) as Partial<MangakiReaderSettings>;
    await writeReaderDefaults(missing);
    if (Object.keys(missing).length > 0) await requestManager.getGlobalMeta().response;
    localStorage.setItem(MIGRATION_KEY, 'done');
};
