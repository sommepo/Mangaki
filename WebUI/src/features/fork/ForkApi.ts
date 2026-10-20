export type OcrState = 'not_prepared' | 'queued' | 'processing' | 'partial' | 'ready' | 'failed' | 'out_of_date';
export type DownloadState =
    | 'not_downloaded'
    | 'queued'
    | 'downloading'
    | 'partial'
    | 'downloaded'
    | 'failed'
    | 'out_of_date';

export interface ForkJob {
    id: string;
    kind: 'ocr' | 'download';
    title: string;
    sensitive: boolean;
    state: OcrState | 'paused' | 'cancelled' | 'complete';
    completedPages: number;
    totalPages?: number;
    error?: string;
}

export type OcrJobOptions = {
    ocrProvider?: 'on_device' | 'self_hosted' | 'lens';
    selfHostedUrl?: string;
};

export interface ForkStats {
    activeReadingSeconds: number;
    uniquePages: number;
    pageTurns: number;
    completedVolumes: number;
    dictionaryLookups: number;
    cardsMined: number;
    ocrCharacters?: number;
    ocrCoveredPages: number;
}

export interface ForkStatsDetail {
    history: { localDate: string; pages: number; activeSeconds: number }[];
    perSeries: {
        seriesId: string;
        title: string;
        pages: number;
        activeSeconds: number;
        lookups: number;
        cards: number;
    }[];
}

export interface ForkReadingSession {
    sessionId: string;
    seriesId: string;
    title: string;
    startedAtMs: number;
    endedAtMs: number;
    pages: number;
    activeSeconds: number;
}

export interface ForkSource {
    id: string;
    kind: 'local' | 'smb' | 'webdav' | 'komga';
    name: string;
    status: 'connected' | 'needs_permission' | 'offline' | 'error';
    detail?: string;
    address?: string;
    libraryIds?: string[];
}

export interface ForkVolume {
    id: string;
    title: string;
    pageCount: number;
    fingerprint: string;
    sensitive: boolean;
    progressPage: number;
    ocrState: OcrState;
    downloadState: DownloadState;
    error?: string;
}

export interface ForkSeries {
    id: string;
    sourceId: string;
    title: string;
    sensitive: boolean;
    categoryIds: string[];
    volumes: ForkVolume[];
}

export interface ForkCategory {
    id: string;
    name: string;
    order: number;
}

export interface ForkScanPreview {
    series: { id: string; title: string; volumeCount: number; pageCount: number; warnings: number }[];
}

export interface ForkIncrementalRefresh {
    newSeries: number;
    newVolumes: number;
}

export interface ForkState {
    showSensitive: boolean;
    sources: ForkSource[];
    categories: ForkCategory[];
    jobs: ForkJob[];
    stats: Record<'day' | 'week' | 'month' | 'all', ForkStats>;
}

const request = async <T>(path: string, init?: RequestInit): Promise<T> => {
    const response = await fetch(`/api/fork${path}`, {
        ...init,
        headers: { 'Content-Type': 'application/json', ...(init?.headers ?? {}) },
    });
    if (!response.ok) throw new Error((await response.text()) || `Request failed (${response.status})`);
    if (response.status === 204) return undefined as T;
    return response.json() as Promise<T>;
};

let catalogueCache: ForkSeries[] | undefined;
// Bumped by every mutation that can change catalogue membership or visibility, so a
// response that was already in flight cannot repopulate the cache with stale data.
let catalogueGeneration = 0;
let catalogueRequest: { generation: number; promise: Promise<ForkSeries[]> } | undefined;

const invalidateCatalogue = () => {
    catalogueGeneration += 1;
    catalogueCache = undefined;
    catalogueRequest = undefined;
};

const invalidatingCatalogue = async <T>(work: Promise<T>): Promise<T> => {
    try {
        return await work;
    } finally {
        invalidateCatalogue();
    }
};

export const FORK_JOBS_CHANGED_EVENT = 'mangaki-fork-jobs-changed';
export const ACTIVE_SERIES_KEY = 'manga_reader_active_series_id';
export const ACTIVE_SERIES_SENSITIVE_KEY = 'manga_reader_active_series_sensitive';

const notifyJobsChanged = <T>(work: Promise<T>): Promise<T> =>
    work.finally(() => window.dispatchEvent(new Event(FORK_JOBS_CHANGED_EVENT)));

const patchCachedProgress = (volumeId: string, page: number) => {
    catalogueCache = catalogueCache?.map((series) =>
        series.volumes.some((volume) => volume.id === volumeId)
            ? {
                  ...series,
                  volumes: series.volumes.map((volume) =>
                      volume.id === volumeId ? { ...volume, progressPage: page } : volume,
                  ),
              }
            : series,
    );
};

export const ForkApi = {
    state: () => request<ForkState>('/state'),
    setSensitiveVisibility: (showSensitive: boolean) => {
        invalidateCatalogue();
        return invalidatingCatalogue(
            request<ForkState>('/privacy', { method: 'PUT', body: JSON.stringify({ showSensitive }) }),
        );
    },
    addSource: (source: Record<string, unknown>) =>
        request<ForkSource>('/sources', { method: 'POST', body: JSON.stringify(source) }),
    removeSource: (id: string) =>
        invalidatingCatalogue(request<void>(`/sources/${encodeURIComponent(id)}`, { method: 'DELETE' })),
    scanSource: (id: string) =>
        invalidatingCatalogue(request<ForkSeries[]>(`/sources/${encodeURIComponent(id)}/scan`, { method: 'POST' })),
    refreshSource: (id: string) =>
        invalidatingCatalogue(
            request<ForkIncrementalRefresh>(`/sources/${encodeURIComponent(id)}/refresh`, { method: 'POST' }),
        ),
    previewSource: (id: string) =>
        request<ForkScanPreview>(`/sources/${encodeURIComponent(id)}/preview`, { method: 'POST' }),
    applySourceScan: (id: string, titleOverrides: Record<string, string>) =>
        invalidatingCatalogue(
            request<ForkSeries[]>(`/sources/${encodeURIComponent(id)}/apply-scan`, {
                method: 'POST',
                body: JSON.stringify({ titleOverrides }),
            }),
        ),
    cachedCatalogue: () => catalogueCache,
    /** Concurrent callers share one request; mutations invalidate both the cache and in-flight reads. */
    catalogue: (): Promise<ForkSeries[]> => {
        if (catalogueRequest?.generation === catalogueGeneration) return catalogueRequest.promise;
        const generation = catalogueGeneration;
        const promise = request<ForkSeries[]>('/catalogue')
            .then((catalogue) => {
                if (generation === catalogueGeneration) catalogueCache = catalogue;
                return catalogue;
            })
            .finally(() => {
                if (catalogueRequest?.promise === promise) catalogueRequest = undefined;
            });
        catalogueRequest = { generation, promise };
        return promise;
    },
    patchCachedProgress,
    setProgress: async (volumeId: string, page: number) => {
        await request<void>(`/volumes/${encodeURIComponent(volumeId)}/progress`, {
            method: 'PUT',
            body: JSON.stringify({ page }),
        });
        patchCachedProgress(volumeId, page);
    },
    addCategory: (name: string) =>
        request<ForkCategory>('/categories', { method: 'POST', body: JSON.stringify({ name }) }),
    renameCategory: (id: string, name: string) =>
        request<ForkCategory>(`/categories/${encodeURIComponent(id)}`, {
            method: 'PUT',
            body: JSON.stringify({ name }),
        }),
    removeCategory: (id: string) =>
        invalidatingCatalogue(request<void>(`/categories/${encodeURIComponent(id)}`, { method: 'DELETE' })),
    reorderCategories: (ids: string[]) =>
        request<void>('/categories/order', { method: 'PUT', body: JSON.stringify({ ids }) }),
    setSeriesSensitive: (seriesIds: string[], sensitive: boolean) =>
        invalidatingCatalogue(
            request<void>('/series/sensitive', { method: 'PUT', body: JSON.stringify({ seriesIds, sensitive }) }),
        ),
    setVolumeSensitive: (volumeIds: string[], sensitive: boolean) =>
        invalidatingCatalogue(
            request<void>('/volumes/sensitive', { method: 'PUT', body: JSON.stringify({ volumeIds, sensitive }) }),
        ),
    setSeriesCategory: (seriesIds: string[], categoryId: string, assigned: boolean) =>
        invalidatingCatalogue(
            request<void>('/series/categories', {
                method: 'PUT',
                body: JSON.stringify({ seriesIds, categoryId, assigned }),
            }),
        ),
    testSource: (source: Record<string, unknown>) =>
        request<{ ok: boolean; detail: string; libraries?: { id: string; name: string }[] }>('/sources/test', {
            method: 'POST',
            body: JSON.stringify(source),
        }),
    controlJob: (id: string, action: 'pause' | 'resume' | 'cancel' | 'retry') =>
        notifyJobsChanged(request<ForkJob>(`/jobs/${encodeURIComponent(id)}/${action}`, { method: 'POST' })),
    testSelfHostedOcr: (url: string) =>
        request<{ ok: boolean; engine: string; modelVersion: string }>('/ocr/self-hosted/test', {
            method: 'POST',
            body: JSON.stringify({ url }),
        }),
    prepareOcr: (seriesIds: string[], reprocess = false, options: OcrJobOptions = {}) =>
        notifyJobsChanged(
            request<ForkJob>('/jobs/ocr', {
                method: 'POST',
                body: JSON.stringify({ seriesIds, volumeIds: [], reprocess, ...options }),
            }),
        ),
    prepareVolumeOcr: (volumeIds: string[], reprocess = false, options: OcrJobOptions = {}) =>
        notifyJobsChanged(
            request<ForkJob>('/jobs/ocr', {
                method: 'POST',
                body: JSON.stringify({ seriesIds: [], volumeIds, reprocess, ...options }),
            }),
        ),
    downloadVolumes: (volumeIds: string[], prepareOcrAfter = false, reprocess = false, options: OcrJobOptions = {}) =>
        notifyJobsChanged(
            request<ForkJob>('/jobs/download', {
                method: 'POST',
                body: JSON.stringify({ seriesIds: [], volumeIds, reprocess, prepareOcrAfter, ...options }),
            }),
        ),
    downloadSeries: (seriesIds: string[], prepareOcrAfter = false, reprocess = false, options: OcrJobOptions = {}) =>
        notifyJobsChanged(
            request<ForkJob>('/jobs/download', {
                method: 'POST',
                body: JSON.stringify({ seriesIds, volumeIds: [], reprocess, prepareOcrAfter, ...options }),
            }),
        ),
    exportBackup: () => request<{ schemaVersion: number; state: ForkState }>('/backup'),
    statsDetail: (range: 'day' | 'week' | 'month' | 'all') => request<ForkStatsDetail>(`/stats/detail?range=${range}`),
    readingSessions: (range: 'day' | 'week' | 'month' | 'all') =>
        request<{ sessions: ForkReadingSession[] }>(`/stats/sessions?range=${range}`),
    removeReadingSession: (sessionId: string) =>
        request<void>(`/stats/sessions/${encodeURIComponent(sessionId)}`, { method: 'DELETE' }),
    restoreBackup: (backup: { schemaVersion: number; state: ForkState }) =>
        notifyJobsChanged(
            invalidatingCatalogue(request<ForkState>('/backup', { method: 'PUT', body: JSON.stringify(backup) })),
        ),
    recordEvent: (
        kind:
            | 'active_reading'
            | 'page_encounter'
            | 'volume_complete'
            | 'dictionary_lookup'
            | 'card_mined'
            | 'ocr_characters',
        options: { value?: number; uniqueKey?: string; seriesId?: string; sensitive?: boolean } = {},
    ) => {
        const now = new Date();
        const localDate = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, '0')}-${String(now.getDate()).padStart(2, '0')}`;
        return fetch('/api/fork/events', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                kind,
                localDate,
                occurredAtMs: now.getTime(),
                value: options.value ?? 1,
                uniqueKey: options.uniqueKey,
                seriesId: options.seriesId ?? sessionStorage.getItem(ACTIVE_SERIES_KEY) ?? undefined,
                // Events recorded by shared reader code (lookups, mining, page encounters) do not know
                // the series; inherit the open fork volume's flag so hidden manga stay out of itemised stats.
                sensitive: options.sensitive ?? sessionStorage.getItem(ACTIVE_SERIES_SENSITIVE_KEY) === '1',
            }),
        });
    },
};
