import { useEffect, useRef } from 'react';
import { useReaderSettingsStore, useReaderPagesStore } from '@/features/reader/stores/ReaderStore.ts';
import { useMangaObserver } from '@/Manatan/hooks/useMangaObserver';
import { ImageOverlay } from '@/Manatan/components/ImageOverlay';
import { YomitanPopup } from '@/Manatan/components/YomitanPopup';
import { useOCR } from '@/Manatan/context/OCRContext';
import { GlobalDialog } from '@/Manatan/components/GlobalDialog';
import { ReadingDirection, ReadingMode } from '@/features/reader/Reader.types.ts';
import { ForkApi } from '@/features/fork/ForkApi';

export const OCRManager = () => {
    const images = useMangaObserver();
    const { openSettings } = useOCR();

    // FIX: Removed 'refreshKey' state and resize listener.
    // This stops the app from destroying/recreating overlays every time you scroll (URL bar resize).
    // ImageOverlay now handles resizing internally via ResizeObserver.

    const { pages, currentPageIndex } = useReaderPagesStore((state) => ({
        pages: state.pages.pages,
        currentPageIndex: state.pages.currentPageIndex,
    }));
    const { readingMode, readingDirection } = useReaderSettingsStore((state) => ({
        readingMode: state.settings.readingMode.value,
        readingDirection: state.settings.readingDirection.value,
    }));
    const lastEncounterRef = useRef<string>();

    const getSpreadData = (imgSrc: string) => {
        if (!pages || pages.length === 0) return undefined;
        if (readingMode !== ReadingMode.DOUBLE_PAGE) return undefined;

        const page = pages.find(
            (p) => imgSrc.includes(p.primary.url) || (p.secondary && imgSrc.includes(p.secondary.url)),
        );

        if (!page || !page.secondary) return undefined;

        const p1 = page.primary;
        const p2 = page.secondary;

        const isRTL = readingDirection === ReadingDirection.RTL;

        if (isRTL) return { leftSrc: p2.url, rightSrc: p1.url };
        return { leftSrc: p1.url, rightSrc: p2.url };
    };

    useEffect(() => {
        const checkUrl = () => {
            const isReader =
                window.location.href.includes('/chapter/') || window.location.pathname.startsWith('/fork/reader/');
            const native = (window as any).MangaReaderNative;
            native?.setImmersive?.(isReader);
            if (isReader) {
                document.documentElement.classList.add('ocr-reader-mode');
            } else {
                document.documentElement.classList.remove('ocr-reader-mode');
            }
        };

        checkUrl();
        const interval = setInterval(checkUrl, 500);
        return () => {
            clearInterval(interval);
            (window as any).MangaReaderNative?.setImmersive?.(false);
            document.documentElement.classList.remove('ocr-reader-mode');
        };
    }, []);

    useEffect(() => {
        if (
            !window.location.href.includes('/chapter/') &&
            !window.location.pathname.startsWith('/fork/reader/')
        ) {
            return;
        }
        const uniqueKey = `${window.location.pathname}#${currentPageIndex}`;
        if (lastEncounterRef.current === uniqueKey) return;
        lastEncounterRef.current = uniqueKey;
        ForkApi.recordEvent('page_encounter', { uniqueKey }).catch(() => undefined);
    }, [currentPageIndex, pages?.length]);

    useEffect(() => {
        let lastInteraction = Date.now();
        const active = () => {
            lastInteraction = Date.now();
        };
        window.addEventListener('pointerdown', active, { passive: true });
        window.addEventListener('keydown', active);
        window.addEventListener('scroll', active, { passive: true });
        const timer = window.setInterval(() => {
            const reading =
                window.location.href.includes('/chapter/') || window.location.pathname.startsWith('/fork/reader/');
            if (reading && document.visibilityState === 'visible' && Date.now() - lastInteraction <= 60_000) {
                ForkApi.recordEvent('active_reading', { value: 20 }).catch(() => undefined);
            }
        }, 20_000);
        return () => {
            window.clearInterval(timer);
            window.removeEventListener('pointerdown', active);
            window.removeEventListener('keydown', active);
            window.removeEventListener('scroll', active);
        };
    }, []);

    // Listen for custom event to open settings from LN Reader
    useEffect(() => {
        const handleOpenSettings = () => {
            openSettings();
        };
        window.addEventListener('openManatanSettings', handleOpenSettings);
        return () => {
            window.removeEventListener('openManatanSettings', handleOpenSettings);
        };
    }, [openSettings]);

    return (
        <>
            <GlobalDialog />

            {images.map((img) => (
                <ImageOverlay key={img.src} img={img} spreadData={getSpreadData(img.src)} />
            ))}

            <YomitanPopup />
        </>
    );
};
