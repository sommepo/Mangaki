import { useEffect, useRef } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

const MARKER = 'mangakiDismissible';

/**
 * Makes Android/browser Back dismiss a transient mode (e.g. multi-selection) before leaving the
 * screen. While `active`, a same-URL history entry carrying a marker is pushed; Back pops it and
 * `dismiss` runs. Dismissing through the UI instead removes that entry again.
 */
export const useBackDismiss = (active: boolean, dismiss: () => void) => {
    const location = useLocation<Record<string, unknown> | null>();
    const navigate = useNavigate();
    const token = useRef<string | undefined>(undefined);
    const marked = token.current !== undefined && location.state?.[MARKER] === token.current;
    const pushed = useRef(false);
    const dismissRef = useRef(dismiss);
    dismissRef.current = dismiss;

    useEffect(() => {
        if (active && !pushed.current) {
            pushed.current = true;
            token.current = `${Date.now()}:${Math.random()}`;
            navigate(`${location.pathname}${location.search}`, {
                state: { ...(location.state ?? {}), [MARKER]: token.current },
            });
            return;
        }
        if (!pushed.current || marked === active) return;
        pushed.current = false;
        if (active) {
            // Back removed our entry.
            dismissRef.current();
        } else {
            // Closed through the UI: drop the entry so Back is not swallowed later.
            navigate(-1);
        }
    }, [active, marked]);
};
