import { useCallback, useEffect, useState } from 'react';
import { ForkApi, ForkState } from '@/features/fork/ForkApi';

export const useForkState = (pollMs = 0) => {
    const [state, setState] = useState<ForkState>();
    const [error, setError] = useState<string>();
    const refresh = useCallback(async () => {
        try {
            setState(await ForkApi.state());
            setError(undefined);
        } catch (cause) {
            setError(cause instanceof Error ? cause.message : String(cause));
        }
    }, []);

    useEffect(() => {
        refresh();
        if (!pollMs) return undefined;
        const timer = window.setInterval(refresh, pollMs);
        return () => window.clearInterval(timer);
    }, [pollMs, refresh]);

    return { state, setState, error, refresh };
};
