import { useEffect, useRef } from 'react';
import { ForkApi } from '@/features/fork/ForkApi';

export const BackgroundJobBridge = () => {
    const lastPayload = useRef('');

    useEffect(() => {
        let stopped = false;
        const sync = async () => {
            try {
                const state = await ForkApi.state();
                if (stopped) return;
                const activeJobs = state.jobs.filter((job) => ['queued', 'processing'].includes(job.state));
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
                if (payload !== lastPayload.current) {
                    lastPayload.current = payload;
                    (window as any).MangaReaderNative?.updateBackgroundJobs?.(
                        activeJobs.length > 0,
                        title,
                        completed,
                        total,
                    );
                }
            } catch {
                // The loopback service can be briefly unavailable while Android restores the process.
            }
        };
        sync();
        const timer = window.setInterval(sync, 1500);
        return () => {
            stopped = true;
            window.clearInterval(timer);
        };
    }, []);

    return null;
};
