import { useEffect, useRef } from 'react';
import { isActiveForkJob, useForkState } from '@/features/fork/useForkState';

const ACTIVE_POLL_MS = 1500;

export const BackgroundJobBridge = () => {
    // Shares the app-wide /state snapshot and timer: 1.5 s while a job runs, slower while idle,
    // paused while the WebView is hidden. The loopback service can be briefly unavailable while
    // Android restores the process; the shared store simply retries on the next tick.
    const { state } = useForkState(ACTIVE_POLL_MS);
    const lastPayload = useRef('');

    useEffect(() => {
        if (!state) return;
        const activeJobs = state.jobs.filter(isActiveForkJob);
        const completed = activeJobs.reduce((sum, job) => sum + job.completedPages, 0);
        const total = activeJobs.reduce((sum, job) => sum + (job.totalPages ?? 0), 0);
        const hasSensitive = activeJobs.some((job) => job.sensitive);
        let title = '';
        if (activeJobs.length > 0) title = `Processing ${activeJobs.length} manga jobs`;
        if (activeJobs.length === 1) title = activeJobs[0].title;
        if (activeJobs.length > 0 && hasSensitive && !state.showSensitive) {
            title = 'Processing hidden manga';
        }
        const payload = JSON.stringify([activeJobs.length > 0, title, completed, total]);
        if (payload === lastPayload.current) return;
        lastPayload.current = payload;
        (window as any).MangaReaderNative?.updateBackgroundJobs?.(activeJobs.length > 0, title, completed, total);
    }, [state]);

    return null;
};
