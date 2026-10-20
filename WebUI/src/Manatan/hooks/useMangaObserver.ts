import { useEffect, useState } from 'react';
import { useOCR } from '@/Manatan/context/OCRContext';

export const useMangaObserver = () => {
    const { settings } = useOCR();
    const [images, setImages] = useState<HTMLImageElement[]>([]);

    useEffect(() => {
        const { imageContainerSelectors: selectors } = settings.site;
        // Overlays are keyed by `img.src`; an element reused for another page (new src) must be
        // reported as a change or its overlay keeps the previous page's OCR boxes.
        let lastForkSrcs: string[] = [];

        const scan = () => {
            const found: HTMLImageElement[] = [];
            document.querySelectorAll<HTMLImageElement>('img.manatan-reader-page').forEach((img) => found.push(img));
            const forkReaderPages = Array.from(document.querySelectorAll<HTMLImageElement>('img.fork-reader-page'));

            if (forkReaderPages.length > 0) {
                const viewportBuffer = window.innerHeight * 2;
                const nearbyPages = forkReaderPages.filter((img) => {
                    if (!img.isConnected || img.naturalHeight <= 200) return false;
                    const rect = img.getBoundingClientRect();
                    return rect.bottom >= -viewportBuffer && rect.top <= window.innerHeight + viewportBuffer;
                });
                const nearbySrcs = nearbyPages.map((img) => img.src);
                const srcsUnchanged =
                    lastForkSrcs.length === nearbySrcs.length &&
                    lastForkSrcs.every((src, index) => src === nearbySrcs[index]);
                lastForkSrcs = nearbySrcs;
                setImages((previous) => {
                    if (
                        srcsUnchanged &&
                        previous.length === nearbyPages.length &&
                        previous.every((img, index) => img === nearbyPages[index])
                    ) {
                        return previous;
                    }
                    return nearbyPages;
                });
                return;
            }

            selectors.forEach((sel) => {
                const nodes = document.querySelectorAll(sel);
                nodes.forEach((node) => {
                    if (node instanceof HTMLImageElement) found.push(node);
                    else node.querySelectorAll('img').forEach((img) => found.push(img));
                });
            });

            if (found.length === 0) {
                document.querySelectorAll('img[src*="/chapter/"]').forEach((img) => {
                    if (img instanceof HTMLImageElement && img.naturalHeight > 400) found.push(img);
                });
            }

            const unique = Array.from(new Set(found)).filter((img) => {
                if (!img.isConnected || img.naturalHeight <= 200) return false;

                if (img.src.includes('thumbnail')) return false;
                if (img.classList.contains('fork-reader-thumbnail')) return false;
                // Fork cover images and reader pages share the same API URL prefix.
                // The standard Manatan reader explicitly marks its full-size pages,
                // so retain those while continuing to keep library covers out of OCR.
                if (
                    img.src.includes('/api/fork/volumes/') &&
                    !img.classList.contains('manatan-reader-page')
                )
                    return false;

                // Exclude cropper images
                if (img.closest('.ReactCrop')) {
                    return false;
                }

                return true;
            });

            setImages((prev) => {
                if (prev.length === unique.length && prev.every((img, i) => img.src === unique[i].src)) return prev;
                return unique;
            });
        };

        scan();

        const observer = new MutationObserver((mutations) => {
            if (mutations.some((m) => m.addedNodes.length > 0 || m.attributeName === 'src')) scan();
        });

        observer.observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ['src'] });
        document.addEventListener('load', scan, true);
        let scheduledScan = 0;
        const scheduleScan = () => {
            if (scheduledScan) return;
            scheduledScan = window.requestAnimationFrame(() => {
                scheduledScan = 0;
                scan();
            });
        };
        window.addEventListener('scroll', scheduleScan, { capture: true, passive: true });
        window.addEventListener('resize', scheduleScan, { passive: true });
        const onVisibilityChange = () => {
            if (document.visibilityState === 'visible') scheduleScan();
        };
        document.addEventListener('visibilitychange', onVisibilityChange);
        window.addEventListener('pageshow', scheduleScan);
        window.addEventListener('focus', scheduleScan);

        return () => {
            observer.disconnect();
            document.removeEventListener('load', scan, true);
            window.removeEventListener('scroll', scheduleScan, true);
            window.removeEventListener('resize', scheduleScan);
            document.removeEventListener('visibilitychange', onVisibilityChange);
            window.removeEventListener('pageshow', scheduleScan);
            window.removeEventListener('focus', scheduleScan);
            if (scheduledScan) window.cancelAnimationFrame(scheduledScan);
        };
    }, [settings.site, settings.debugMode]);

    return images;
};
