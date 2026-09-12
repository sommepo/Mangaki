import React, { useEffect, useState, useRef, useCallback, memo, useLayoutEffect } from 'react';
import { createPortal } from 'react-dom';
import { useOCR } from '@/Manatan/context/OCRContext';
import { OcrBlock, OcrStatus } from '@/Manatan/types';
import { apiRequest, buildChapterBaseUrl } from '@/Manatan/utils/api';
import { isNoSpaceLanguage } from '@/Manatan/utils/language';
import { TextBox } from '@/Manatan/components/TextBox';
import { StatusIcon } from '@/Manatan/components/StatusIcon';
import { useReaderOverlayStore, useReaderSettingsStore } from '@/features/reader/stores/ReaderStore';
import { ReadingMode } from '@/features/reader/Reader.types.ts';

const getOverlayContainer = (): HTMLElement => {
    let container = document.getElementById('ocr-overlay-layer');
    if (!container) {
        container = document.createElement('div');
        container.id = 'ocr-overlay-layer';
        container.style.cssText = `
            position: fixed;
            top: 0;
            left: 0;
            width: 100vw;
            height: 100vh;
            pointer-events: none;
            z-index: 9999;
            overflow: visible;
        `;
        document.body.appendChild(container);
    }
    return container;
};

// --- INNER COMPONENT (MEMOIZED) ---
interface ImageOverlayInnerProps {
    data: OcrBlock[] | null;
    status: OcrStatus;
    img: HTMLImageElement;
    spreadData?: { leftSrc: string; rightSrc: string };
    onRetry: () => void;
    onUpdate: (index: number, text: string) => void;
    onMerge: (source: number, target: number) => void;
    onDelete: (index: number) => void;
    shouldShowChildren: boolean;
}

const BaseImageOverlayInner: React.FC<ImageOverlayInnerProps> = ({
    data,
    status,
    img,
    spreadData,
    onRetry,
    onUpdate,
    onMerge,
    onDelete,
    shouldShowChildren,
}) => {
    const wrapperRef = useRef<HTMLDivElement>(null);
    const innerRef = useRef<HTMLDivElement>(null);
    const [dimensions, setDimensions] = useState({ width: 0, height: 0 });
    const [isVisible, setIsVisible] = useState(false);
    const containerRef = useRef<HTMLElement>(getOverlayContainer());
    const isForkReaderPage = img.classList.contains('fork-reader-page');

    useLayoutEffect(() => {
        if (!wrapperRef.current) return undefined;

        const wrapper = wrapperRef.current;
        const inner = innerRef.current;

        const syncPosition = () => {
            if (!img.isConnected) {
                wrapper.style.display = 'none';
                return;
            }

            if (!img.offsetParent) {
                wrapper.style.display = 'none';
                return;
            }

            const rect = img.getBoundingClientRect();

            const isInViewport =
                rect.bottom > 0 &&
                rect.top < window.innerHeight &&
                rect.right > 0 &&
                rect.left < window.innerWidth &&
                rect.width > 10 &&
                rect.height > 10;

            if (!isInViewport) {
                wrapper.style.display = 'none';
                return;
            }

            wrapper.style.display = 'block';

            // Fork reader overlays belong to the page container so a drag or
            // pinch that starts on OCR text still reaches the reader's native
            // scroll/zoom handlers. Other Manatan pages retain the fixed portal.
            const width = isForkReaderPage ? img.offsetWidth : rect.width;
            const height = isForkReaderPage ? img.offsetHeight : rect.height;
            wrapper.style.transform = isForkReaderPage
                ? `translate3d(${img.offsetLeft}px, ${img.offsetTop}px, 0)`
                : `translate3d(${rect.left}px, ${rect.top}px, 0)`;

            if (inner) {
                inner.style.width = `${width}px`;
                inner.style.height = `${height}px`;
            }

            if (Math.abs(dimensions.width - width) > 1 || Math.abs(dimensions.height - height) > 1) {
                setDimensions({ width, height });
            }

            if (!isVisible) setIsVisible(true);
        };

        // Initial sync
        syncPosition();

        // IntersectionObserver for reliable visibility detection
        const intersectionObserver = new IntersectionObserver(
            (entries) => {
                entries.forEach((entry) => {
                    if (entry.isIntersecting) {
                        requestAnimationFrame(syncPosition);
                    } else {
                        wrapper.style.display = 'none';
                    }
                });
            },
            { threshold: [0, 0.1], rootMargin: '200px' },
        );
        intersectionObserver.observe(img);

        const resizeObserver = new ResizeObserver(() => {
            requestAnimationFrame(syncPosition);
        });
        resizeObserver.observe(img);

        // Scroll/resize handlers
        let ticking = false;
        const onUpdateEvent = () => {
            if (ticking) return;
            ticking = true;
            requestAnimationFrame(() => {
                syncPosition();
                ticking = false;
            });
        };

        window.addEventListener('scroll', onUpdateEvent, { capture: true, passive: true });
        window.addEventListener('resize', onUpdateEvent, { passive: true });
        window.addEventListener('mangaReaderZoomAction', onUpdateEvent);
        window.addEventListener('mangaReaderZoomGeometryChanged', onUpdateEvent);
        const onVisibilityChange = () => {
            if (document.visibilityState === 'visible') onUpdateEvent();
        };
        document.addEventListener('visibilitychange', onVisibilityChange);
        window.addEventListener('pageshow', onUpdateEvent);
        window.addEventListener('focus', onUpdateEvent);

        return () => {
            intersectionObserver.disconnect();
            resizeObserver.disconnect();
            window.removeEventListener('scroll', onUpdateEvent, true);
            window.removeEventListener('resize', onUpdateEvent);
            window.removeEventListener('mangaReaderZoomAction', onUpdateEvent);
            window.removeEventListener('mangaReaderZoomGeometryChanged', onUpdateEvent);
            document.removeEventListener('visibilitychange', onVisibilityChange);
            window.removeEventListener('pageshow', onUpdateEvent);
            window.removeEventListener('focus', onUpdateEvent);
        };
    }, [img, dimensions.width, dimensions.height, isForkReaderPage, isVisible]);

    const portalTarget = isForkReaderPage && img.parentElement ? img.parentElement : containerRef.current;

    return createPortal(
        <div
            ref={wrapperRef}
            className="ocr-overlay-wrapper"
            data-img-src={img.src.slice(-30)}
            style={{
                position: isForkReaderPage ? 'absolute' : 'fixed',
                top: 0,
                left: 0,
                pointerEvents: 'none',
                willChange: 'transform',
                display: 'none',
                zIndex: 10,
            }}
        >
            <div
                ref={innerRef}
                style={{
                    position: 'relative',
                    width: dimensions.width || 'auto',
                    height: dimensions.height || 'auto',
                }}
            >
                <div style={{ opacity: shouldShowChildren ? 1 : 0, transition: 'opacity 0.2s' }}>
                    <StatusIcon status={status} onRetry={onRetry} />
                </div>

                {isVisible &&
                    data
                        ?.filter((block) => block.text.replace(/[\s\u200B]/g, '').length > 0)
                        .map((block: OcrBlock, i: number) => (
                            <TextBox
                                key={`${i}-${block.text.substring(0, 5)}`}
                                index={i}
                                block={block}
                                imgSrc={img.src}
                                spreadData={spreadData}
                                containerWidth={dimensions.width}
                                containerHeight={dimensions.height}
                                onUpdate={onUpdate}
                                onMerge={onMerge}
                                onDelete={onDelete}
                                parentVisible={shouldShowChildren}
                            />
                        ))}
            </div>
        </div>,
        portalTarget,
    );
};

const ImageOverlayInner = memo(BaseImageOverlayInner);

const recognisedCharacterCount = (blocks: OcrBlock[]): number =>
    blocks.reduce(
        (total, block) => total + String(block.text ?? '').replace(/[\s\u200B\p{P}\p{S}]/gu, '').length,
        0,
    );

const selectBestForkOcr = (local: OcrBlock[], lens: OcrBlock[]): OcrBlock[] => {
    if (lens.length === 0) return local;
    if (local.length === 0) return lens;

    const lensCharacters = recognisedCharacterCount(lens);
    const localCharacters = recognisedCharacterCount(local);

    // Prefer Lens for comparable coverage, but do not blindly discard a
    // substantially more complete prepared on-device result. Keeping one
    // provider's coherent region set also avoids overlapping duplicate boxes.
    return localCharacters >= Math.max(lensCharacters + 8, Math.ceil(lensCharacters * 1.2))
        ? local
        : lens;
};

// Continuous reading can mount several nearby page overlays at once. The
// self-hosted model is intentionally single-inference, so serialize those live
// requests instead of making the server hold a burst of waiting page images.
let selfHostedOcrQueue: Promise<void> = Promise.resolve();
const runSelfHostedOcr = <T,>(request: () => Promise<T>): Promise<T> => {
    const result = selfHostedOcrQueue.then(request, request);
    selfHostedOcrQueue = result.then(() => undefined, () => undefined);
    return result;
};

// --- MAIN COMPONENT ---
export const ImageOverlay: React.FC<{
    img: HTMLImageElement;
    spreadData?: { leftSrc: string; rightSrc: string };
}> = ({ img, spreadData }) => {
    const {
        settings,
        serverSettings,
        ocrCache,
        updateOcrData,
        setActiveImageSrc,
        ocrStatusMap,
        setOcrStatus,
        dictPopup,
    } = useOCR();

    // Keep the page's recognised regions local to its overlay. The fork reader
    // continuously changes which pages are near the viewport while zooming;
    // relying on the shared cache as render state allowed a zoom reconciliation
    // to replace a complete page with a partial stale array.
    const [data, setData] = useState<OcrBlock[] | null>(() => ocrCache.get(img.src) || null);
    const currentStatus = ocrCache.has(img.src) ? 'success' : ocrStatusMap.get(img.src) || 'idle';
    const isReaderOverlayVisible = useReaderOverlayStore((state) => state.overlay.isVisible);
    const isForkReaderPage = img.classList.contains('fork-reader-page');

    const readingMode = useReaderSettingsStore((state) => state.settings.readingMode.value);

    const hideTimerRef = useRef<number | null>(null);
    const isPopupOpenRef = useRef(false);
    const fetchInFlightRef = useRef(false);

    useEffect(() => {
        const cached = ocrCache.get(img.src);
        if (!cached) return;
        setData((current) => {
            if (!current) return cached;
            const currentCoverage = recognisedCharacterCount(current);
            const cachedCoverage = recognisedCharacterCount(cached);
            return cached.length >= current.length && cachedCoverage >= currentCoverage
                ? cached
                : current;
        });
    }, [img.src, ocrCache]);

    useEffect(() => {
        isPopupOpenRef.current = dictPopup.visible;
    }, [dictPopup.visible]);

    const fetchOCR = useCallback(async () => {
        if (!img.src || ocrCache.has(img.src) || fetchInFlightRef.current) return;
        const isForkPage = img.src.includes('/api/fork/volumes/');
        const provider = settings.ocrProvider ?? (settings.allowGoogleLensOcr ? 'lens' : 'on_device');
        if (!isForkPage && provider !== 'lens') {
            // The inherited provider uploads the current page to Google Lens.
            // Never start it until the user explicitly opts in from OCR settings.
            setOcrStatus(img.src, 'error');
            return;
        }
        fetchInFlightRef.current = true;
        try {
            const requestedImgSrc = img.src;
            setOcrStatus(img.src, 'loading');
            let url = `/api/ocr/ocr?url=${encodeURIComponent(requestedImgSrc)}`;
            url += `&add_space_on_merge=${!isNoSpaceLanguage(settings.yomitanLanguage)}`;
            url += `&language=${encodeURIComponent(settings.yomitanLanguage)}`;
            const mangaMatch = window.location.pathname.match(/\/manga\/(\d+)/);
            const chapterMatch = window.location.pathname.match(/\/chapter\/(\d+)/);
            if (mangaMatch && chapterMatch) {
                const chapterPath = `/manga/${mangaMatch[1]}/chapter/${chapterMatch[1]}`;
                const baseUrl = buildChapterBaseUrl(chapterPath);
                url += `&base_url=${encodeURIComponent(baseUrl)}`;
            }
            if (serverSettings?.authUsername?.trim() && serverSettings?.authPassword?.trim()) {
                url += `&user=${encodeURIComponent(serverSettings.authUsername.trim())}`;
                url += `&pass=${encodeURIComponent(serverSettings.authPassword.trim())}`;
            }
            let result: OcrBlock[];
            if (isForkPage && provider === 'self_hosted') {
                if (!settings.selfHostedOcrUrl.trim()) throw new Error('Configure the self-hosted OCR URL in Settings');
                url += `&provider=self_hosted&remote_url=${encodeURIComponent(settings.selfHostedOcrUrl.trim())}`;
                result = await runSelfHostedOcr(() => apiRequest<OcrBlock[]>(url));
            } else if (isForkPage && provider === 'lens') {
                const [localResult, lensResult] = await Promise.allSettled([
                    apiRequest<OcrBlock[]>(`${url}&cache_only=true`),
                    apiRequest<OcrBlock[]>(`${url}&provider=lens`),
                ]);
                const local = localResult.status === 'fulfilled' && Array.isArray(localResult.value)
                    ? localResult.value
                    : [];
                const lens = lensResult.status === 'fulfilled' && Array.isArray(lensResult.value)
                    ? lensResult.value
                    : [];
                if (localResult.status === 'rejected' && lensResult.status === 'rejected') {
                    throw lensResult.reason;
                }
                result = selectBestForkOcr(local, lens);
            } else {
                result = await apiRequest<OcrBlock[]>(url);
            }
            if (Array.isArray(result)) {
                setData(result);
                updateOcrData(requestedImgSrc, result);
            } else {
                throw new Error('Invalid response format');
            }
        } catch (err) {
            console.error('OCR Failed:', err);
            setOcrStatus(img.src, 'error');
        } finally {
            fetchInFlightRef.current = false;
        }
    }, [
        img.src,
        ocrCache,
        setOcrStatus,
        updateOcrData,
        serverSettings,
        settings.yomitanLanguage,
        settings.allowGoogleLensOcr,
        settings.ocrProvider,
        settings.selfHostedOcrUrl,
    ]);

    useEffect(() => {
        if (!img.src) return undefined;
        if (ocrCache.has(img.src)) {
            if (ocrStatusMap.get(img.src) !== 'success') setOcrStatus(img.src, 'success');
            return undefined;
        }
        if (currentStatus === 'loading' || currentStatus === 'error') return undefined;
        const startFetch = () => void fetchOCR();
        if (img.complete && img.naturalWidth > 0) {
            startFetch();
            return undefined;
        }
        const handleLoad = () => startFetch();
        img.addEventListener('load', handleLoad, { once: true });
        // The image can complete between the check and listener registration.
        // Recheck synchronously so a fresh/resumed reader cannot miss OCR setup.
        if (img.complete && img.naturalWidth > 0) {
            img.removeEventListener('load', handleLoad);
            startFetch();
            return undefined;
        }
        return () => img.removeEventListener('load', handleLoad);
    }, [fetchOCR, img.complete, ocrCache, img.src, currentStatus, setOcrStatus, ocrStatusMap]);

    useEffect(() => {
        const retryAfterResume = () => {
            if (document.visibilityState !== 'visible' || ocrCache.has(img.src)) return;
            const rect = img.getBoundingClientRect();
            if (rect.bottom > -window.innerHeight * 2 && rect.top < window.innerHeight * 3) {
                void fetchOCR();
            }
        };
        document.addEventListener('visibilitychange', retryAfterResume);
        window.addEventListener('pageshow', retryAfterResume);
        window.addEventListener('focus', retryAfterResume);
        return () => {
            document.removeEventListener('visibilitychange', retryAfterResume);
            window.removeEventListener('pageshow', retryAfterResume);
            window.removeEventListener('focus', retryAfterResume);
        };
    }, [fetchOCR, img, img.src, ocrCache]);

    // Hover / Interaction
    useEffect(() => {
        const clearTimer = () => {
            if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
        };
        const show = () => {
            clearTimer();
            setActiveImageSrc(img.src);
        };
        const hide = () => {
            clearTimer();
            hideTimerRef.current = window.setTimeout(() => {}, 400);
        };

        img.addEventListener('mouseenter', show);
        img.addEventListener('mouseleave', hide);
        return () => {
            img.removeEventListener('mouseenter', show);
            img.removeEventListener('mouseleave', hide);
            clearTimer();
        };
    }, [img, setActiveImageSrc]);

    const handleUpdate = useCallback(
        (index: number, newText: string) => {
            if (!data) return;
            const newData = [...data];
            newData[index] = { ...newData[index], text: newText };
            setData(newData);
            updateOcrData(img.src, newData);
        },
        [data, img.src, updateOcrData],
    );

    const handleMerge = useCallback(
        (idx1: number, idx2: number) => {
            if (!data) return;
            const b1 = data[idx1];
            const b2 = data[idx2];
            const separator = isNoSpaceLanguage(settings.yomitanLanguage) ? '\u200B' : ' ';
            const newBlock: OcrBlock = {
                text: b1.text + separator + b2.text,
                tightBoundingBox: {
                    x: Math.min(b1.tightBoundingBox.x, b2.tightBoundingBox.x),
                    y: Math.min(b1.tightBoundingBox.y, b2.tightBoundingBox.y),
                    width:
                        Math.max(
                            b1.tightBoundingBox.x + b1.tightBoundingBox.width,
                            b2.tightBoundingBox.x + b2.tightBoundingBox.width,
                        ) - Math.min(b1.tightBoundingBox.x, b2.tightBoundingBox.x),
                    height:
                        Math.max(
                            b1.tightBoundingBox.y + b1.tightBoundingBox.height,
                            b2.tightBoundingBox.y + b2.tightBoundingBox.height,
                        ) - Math.min(b1.tightBoundingBox.y, b2.tightBoundingBox.y),
                },
                isMerged: true,
            };
            const newData = data.filter((_, i) => i !== idx1 && i !== idx2);
            newData.push(newBlock);
            setData(newData);
            updateOcrData(img.src, newData);
        },
        [data, img.src, settings.yomitanLanguage, updateOcrData],
    );

    const handleDelete = useCallback(
        (index: number) => {
            if (!data) return;
            const newData = data.filter((_, i) => i !== index);
            setData(newData);
            updateOcrData(img.src, newData);
        },
        [data, img.src, updateOcrData],
    );

    const isImgDisplayed = img.offsetParent !== null;

    const isPaginatedMode = readingMode === ReadingMode.SINGLE_PAGE || readingMode === ReadingMode.DOUBLE_PAGE;

    let isCorrectChapter = true;
    if (isPaginatedMode) {
        const currentChapterMatch = window.location.pathname.match(/\/chapter\/(\d+)/);
        const currentChapterId = currentChapterMatch ? currentChapterMatch[1] : null;

        const imgChapterMatch = img.src.match(/\/chapter\/(\d+)\//);
        const imgChapterId = imgChapterMatch ? imgChapterMatch[1] : null;

        if (currentChapterId && imgChapterId) {
            isCorrectChapter = currentChapterId === imgChapterId;
        }
    }

    const isGlobalEnabled =
        settings.enableOverlay && isImgDisplayed && (isForkReaderPage || !isReaderOverlayVisible) && isCorrectChapter;
    const shouldShowChildren =
        !settings.soloHoverMode ||
        settings.interactionMode === 'click' ||
        settings.debugMode ||
        currentStatus === 'loading' ||
        currentStatus === 'error';

    if (!isGlobalEnabled) return null;

    return (
        <ImageOverlayInner
            data={data}
            status={currentStatus}
            img={img}
            spreadData={spreadData}
            onRetry={fetchOCR}
            onUpdate={handleUpdate}
            onMerge={handleMerge}
            onDelete={handleDelete}
            shouldShowChildren={shouldShowChildren}
        />
    );
};
