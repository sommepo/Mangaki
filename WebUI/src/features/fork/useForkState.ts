import { useCallback, useEffect, useSyncExternalStore } from 'react';
import { FORK_JOBS_CHANGED_EVENT, ForkApi, ForkJob, ForkState } from '@/features/fork/ForkApi';

// One shared /api/fork/state snapshot for every screen plus the native notification bridge.
// Concurrent refreshes share a request, a single timer serves all pollers, polling pauses while
// the WebView is hidden and backs off while no job is running.
const IDLE_POLL_MS = 10_000;

let snapshot: ForkState | undefined;
let lastError: string | undefined;
let version = 0;
let inflight: Promise<ForkState> | undefined;
const listeners = new Set<() => void>();
const pollers = new Map<symbol, number>();
let pollTimer: number | undefined;

const notify = () => listeners.forEach((listener) => listener());

export const isActiveForkJob = (job: ForkJob) => job.state === 'queued' || job.state === 'processing';

export const getForkStateSnapshot = () => snapshot;
const getForkStateError = () => lastError;

export const setForkStateSnapshot = (next: ForkState) => {
    version += 1;
    snapshot = next;
    lastError = undefined;
    notify();
};

export const subscribeForkState = (listener: () => void) => {
    listeners.add(listener);
    return () => {
        listeners.delete(listener);
    };
};

/**
 * `fresh` callers (e.g. after a mutation) must not reuse a read that started before their change.
 */
export const fetchForkState = ({ fresh = false }: { fresh?: boolean } = {}): Promise<ForkState> => {
    if (inflight) {
        if (!fresh) return inflight;
        const retry = () => fetchForkState();
        return inflight.then(retry, retry);
    }
    const startedAt = version;
    const request = ForkApi.state()
        .then((next) => {
            // A mutation response (e.g. PUT /privacy) that landed while this read was in flight is newer.
            if (startedAt === version) setForkStateSnapshot(next);
            else if (lastError !== undefined) {
                lastError = undefined;
                notify();
            }
            return snapshot ?? next;
        })
        .catch((cause) => {
            lastError = cause instanceof Error ? cause.message : String(cause);
            notify();
            throw cause;
        })
        .finally(() => {
            inflight = undefined;
        });
    inflight = request;
    return request;
};

const schedulePoll = () => {
    if (pollTimer !== undefined) window.clearTimeout(pollTimer);
    pollTimer = undefined;
    if (pollers.size === 0 || document.visibilityState === 'hidden') return;
    const fastest = Math.min(...pollers.values());
    const delay = snapshot?.jobs.some(isActiveForkJob) ? fastest : Math.max(fastest, IDLE_POLL_MS);
    pollTimer = window.setTimeout(() => {
        pollTimer = undefined;
        fetchForkState()
            .catch(() => undefined)
            .finally(schedulePoll);
    }, delay);
};

const refreshAndReschedule = (fresh = false) => {
    fetchForkState({ fresh })
        .catch(() => undefined)
        .finally(schedulePoll);
};

if (typeof window !== 'undefined') {
    document.addEventListener('visibilitychange', () => {
        if (pollers.size === 0) return;
        if (document.visibilityState === 'visible') refreshAndReschedule();
        else schedulePoll();
    });
    window.addEventListener(FORK_JOBS_CHANGED_EVENT, () => {
        if (pollers.size > 0) refreshAndReschedule(true);
    });
}

/** Registers a poller for as long as the calling component is mounted. */
export const useForkStatePolling = (pollMs: number) => {
    useEffect(() => {
        if (!pollMs) return undefined;
        const id = Symbol('fork-state-poller');
        pollers.set(id, pollMs);
        schedulePoll();
        return () => {
            pollers.delete(id);
            schedulePoll();
        };
    }, [pollMs]);
};

export const useForkState = (pollMs = 0) => {
    const state = useSyncExternalStore(subscribeForkState, getForkStateSnapshot);
    const error = useSyncExternalStore(subscribeForkState, getForkStateError);
    const refresh = useCallback(async () => {
        try {
            await fetchForkState({ fresh: true });
        } catch {
            // Surfaced through the shared error snapshot.
        }
    }, []);

    useEffect(() => {
        fetchForkState().catch(() => undefined);
    }, []);
    useForkStatePolling(pollMs);

    return { state, setState: setForkStateSnapshot, error, refresh };
};

/** Navigation badge: reads the shared snapshot only, so it never adds requests of its own. */
export const useActiveForkJobBadge = () => {
    const state = useSyncExternalStore(subscribeForkState, getForkStateSnapshot);
    const count = state?.jobs.filter(isActiveForkJob).length ?? 0;
    return { count, title: count ? `${count} active job${count === 1 ? '' : 's'}` : '' };
};
