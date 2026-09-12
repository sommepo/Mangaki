import ArrowBackIcon from '@mui/icons-material/ArrowBack';
import ArrowForwardIcon from '@mui/icons-material/ArrowForward';
import BookmarkBorderIcon from '@mui/icons-material/BookmarkBorder';
import BookmarkIcon from '@mui/icons-material/Bookmark';
import CheckRoundedIcon from '@mui/icons-material/CheckRounded';
import ErrorOutlineRoundedIcon from '@mui/icons-material/ErrorOutlineRounded';
import FitScreenIcon from '@mui/icons-material/FitScreen';
import ZoomInIcon from '@mui/icons-material/ZoomIn';
import ZoomOutIcon from '@mui/icons-material/ZoomOut';
import Alert from '@mui/material/Alert';
import Box from '@mui/material/Box';
import CircularProgress from '@mui/material/CircularProgress';
import IconButton from '@mui/material/IconButton';
import Slider from '@mui/material/Slider';
import Stack from '@mui/material/Stack';
import Typography from '@mui/material/Typography';
import useMediaQuery from '@mui/material/useMediaQuery';
import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { useNavigate, useParams } from 'react-router-dom';
import { ForkApi, ForkSeries, ForkVolume } from '@/features/fork/ForkApi';
import { useMobileZoomPan } from '@/features/reader/zoom/useMobileZoomPan';
import { useOCR } from '@/Manatan/context/OCRContext';
import {
    readReaderPreferences,
    subscribeReaderPreferences,
    writeReaderPreferences,
} from '@/features/fork/ReaderPreferences';

const BOOKMARK_KEY = 'manga_reader_bookmarks_v1';

export function ForkReader() {
    const { volumeId } = useParams();
    const navigate = useNavigate();
    const [catalogue, setCatalogue] = useState<ForkSeries[]>([]);
    const [page, setPage] = useState(0);
    const [controls, setControls] = useState(false);
    const [error, setError] = useState<string>();
    const [loadedPages, setLoadedPages] = useState<Set<number>>(() => new Set());
    const preferences = useSyncExternalStore(subscribeReaderPreferences, readReaderPreferences);
    const { ocrCache, ocrStatusMap, setDictPopup, settings } = useOCR();
    const wide = useMediaQuery('(min-width:800px)');
    const touchStart = useRef<{ x: number; y: number }>();
    const scrollContainerRef = useRef<HTMLDivElement>(null);
    const zoomSizerRef = useRef<HTMLDivElement>(null);
    const zoomContentRef = useRef<HTMLDivElement>(null);
    const pageRefs = useRef<(HTMLDivElement | null)[]>([]);
    const visibleRatios = useRef<Map<number, number>>(new Map());
    const zoomTracking = useRef(false);
    const zoomGeneration = useRef(0);
    const zoomSettleTimer = useRef<number>();
    const viewportSettleTimer = useRef<number>();
    const viewportMoving = useRef(false);
    const suppressReaderClickUntil = useRef(0);
    const zoomAnchor = useRef<{ page: number; ratio: number }>();
    const initialScrollKey = useRef('');
    const initialPositionReady = useRef(false);
    const initialPositionSettling = useRef(false);
    const resumePage = useRef<number>();
    const initialScrollFrame = useRef<number>();
    const initialSettleFrame = useRef<number>();
    const initialSettleTimer = useRef<number>();
    const lastInteraction = useRef(Date.now());
    const [bookmarks, setBookmarks] = useState<string[]>(() => {
        try {
            return JSON.parse(localStorage.getItem(BOOKMARK_KEY) ?? '[]');
        } catch {
            return [];
        }
    });
    const volume = useMemo<ForkVolume | undefined>(
        () => catalogue.flatMap((series) => series.volumes).find((item) => item.id === volumeId),
        [catalogue, volumeId],
    );
    const owner = useMemo(
        () => catalogue.find((series) => series.volumes.some((item) => item.id === volumeId)),
        [catalogue, volumeId],
    );
    const readableVolumes = useMemo(
        () => owner?.volumes.filter((item) => item.pageCount > 0 && !item.error) ?? [],
        [owner],
    );
    const spread = Boolean(preferences.twoPage && wide && preferences.direction !== 'vertical');
    const pageStep = spread ? 2 : 1;
    const bookmarkId = volume ? `${volume.id}:${page}` : '';
    const continuous = preferences.direction === 'vertical';
    const pageWidth = continuous || !spread ? '100%' : '50%';
    const visibleOcrPages = useMemo(
        () => [page, ...(spread && page + 1 < (volume?.pageCount ?? 0) ? [page + 1] : [])],
        [page, spread, volume?.pageCount],
    );
    const visibleOcrStatuses = useMemo(
        () =>
            visibleOcrPages.map((visiblePage) => {
                if (!volume) return 'idle' as const;
                const relativeSrc = `/api/fork/volumes/${encodeURIComponent(volume.id)}/pages/${visiblePage}?fingerprint=${encodeURIComponent(volume.fingerprint)}`;
                const imageSrc = new URL(relativeSrc, window.location.href).href;
                if (ocrCache.has(imageSrc)) return 'success' as const;
                return ocrStatusMap.get(imageSrc) ?? ('idle' as const);
            }),
        [ocrCache, ocrStatusMap, visibleOcrPages, volume],
    );
    const currentOcrStatus = visibleOcrStatuses.some((status) => status === 'loading')
        ? 'loading'
        : visibleOcrStatuses.some((status) => status === 'error')
          ? 'error'
          : visibleOcrStatuses.length > 0 && visibleOcrStatuses.every((status) => status === 'success')
            ? 'success'
            : 'idle';
    const currentOcrLabel =
        currentOcrStatus === 'success'
            ? 'OCR cache ready'
            : currentOcrStatus === 'error'
              ? 'OCR cache failed'
              : currentOcrStatus === 'loading'
                ? 'Loading OCR cache'
                : 'Waiting for OCR cache';
    const pageAtViewportCenter = useCallback(() => {
        const root = scrollContainerRef.current;
        if (!root) return undefined;
        const rootRect = root.getBoundingClientRect();
        const centerY = rootRect.top + rootRect.height / 2;
        let nearest: { page: number; ratio: number; distance: number } | undefined;
        pageRefs.current.forEach((element, pageNumber) => {
            if (!element) return;
            const rect = element.getBoundingClientRect();
            if (rect.height <= 0) return;
            let distance = 0;
            if (centerY < rect.top) distance = rect.top - centerY;
            else if (centerY > rect.bottom) distance = centerY - rect.bottom;
            if (!nearest || distance < nearest.distance) {
                nearest = {
                    page: pageNumber,
                    ratio: Math.max(0, Math.min(1, (centerY - rect.top) / rect.height)),
                    distance,
                };
            }
        });
        return nearest;
    }, []);
    const beginZoomInteraction = useCallback(() => {
        if (zoomSettleTimer.current) window.clearTimeout(zoomSettleTimer.current);
        zoomGeneration.current += 1;
        suppressReaderClickUntil.current = Date.now() + 400;
        zoomTracking.current = true;
        visibleRatios.current.clear();
        zoomAnchor.current = pageAtViewportCenter();
        document.documentElement.classList.add('ocr-zoom-gesture-active');
        window.getSelection()?.removeAllRanges();
        window.dispatchEvent(new Event('mangaReaderZoomAction'));
        setDictPopup((current) => {
            if (!current.visible) return current;
            return { ...current, visible: false };
        });
    }, [pageAtViewportCenter, setDictPopup]);
    const finishZoomInteraction = useCallback(() => {
        const generation = zoomGeneration.current;
        requestAnimationFrame(() => {
            requestAnimationFrame(() => {
                if (generation !== zoomGeneration.current) return;
                const anchor = zoomAnchor.current;
                const current = pageAtViewportCenter();
                const root = scrollContainerRef.current;
                if (continuous && anchor && current && root && Math.abs(current.page - anchor.page) > 1) {
                    const element = pageRefs.current[anchor.page];
                    if (element) {
                        const rootRect = root.getBoundingClientRect();
                        const rect = element.getBoundingClientRect();
                        const currentAnchorY = rect.top + rect.height * anchor.ratio;
                        root.scrollTop += currentAnchorY - (rootRect.top + rootRect.height / 2);
                    }
                }
                zoomAnchor.current = undefined;
                visibleRatios.current.clear();
                zoomSettleTimer.current = window.setTimeout(() => {
                    if (generation !== zoomGeneration.current) return;
                    zoomTracking.current = false;
                    document.documentElement.classList.remove('ocr-zoom-gesture-active');
                    window.dispatchEvent(new Event('mangaReaderZoomGeometryChanged'));
                }, 250);
            });
        });
    }, [continuous, pageAtViewportCenter]);

    const resetInterruptedOcrInteraction = useCallback(() => {
        zoomGeneration.current += 1;
        if (zoomSettleTimer.current) window.clearTimeout(zoomSettleTimer.current);
        if (viewportSettleTimer.current) window.clearTimeout(viewportSettleTimer.current);
        zoomTracking.current = false;
        viewportMoving.current = false;
        zoomAnchor.current = undefined;
        visibleRatios.current.clear();
        document.documentElement.classList.remove('ocr-zoom-gesture-active');
        document.documentElement.classList.remove('ocr-reader-motion-active');
        window.getSelection()?.removeAllRanges();
        window.dispatchEvent(new Event('mangaReaderZoomAction'));
        setDictPopup((current) => {
            if (!current.visible) return current;
            return { ...current, visible: false };
        });
        requestAnimationFrame(() => window.dispatchEvent(new Event('mangaReaderZoomGeometryChanged')));
    }, [setDictPopup]);
    const beginViewportMotion = useCallback(() => {
        if (!viewportMoving.current) {
            viewportMoving.current = true;
            document.documentElement.classList.add('ocr-reader-motion-active');
            window.getSelection()?.removeAllRanges();
            window.dispatchEvent(new Event('mangaReaderZoomAction'));
            setDictPopup((current) => {
                if (!current.visible) return current;
                return { ...current, visible: false };
            });
        }
        if (viewportSettleTimer.current) window.clearTimeout(viewportSettleTimer.current);
        viewportSettleTimer.current = window.setTimeout(() => {
            viewportMoving.current = false;
            document.documentElement.classList.remove('ocr-reader-motion-active');
            window.getSelection()?.removeAllRanges();
            window.dispatchEvent(new Event('mangaReaderZoomGeometryChanged'));
        }, 120);
    }, [setDictPopup]);
    const { scale, isZooming, resetZoom, zoomTo } = useMobileZoomPan(scrollContainerRef, zoomContentRef, zoomSizerRef, {
        enabled: Boolean(volume),
        isVertical: true,
        maxScale: 5,
        doubleTapZoomEnabled: false,
        onZoomStart: beginZoomInteraction,
        onZoomEnd: finishZoomInteraction,
    });

    useEffect(() => {
        (window as any).MangaReaderNative?.setImmersive?.(true);
        setLoadedPages(new Set());
        initialScrollKey.current = '';
        initialPositionReady.current = false;
        initialPositionSettling.current = false;
        resumePage.current = undefined;
        if (initialScrollFrame.current) window.cancelAnimationFrame(initialScrollFrame.current);
        if (initialSettleFrame.current) window.cancelAnimationFrame(initialSettleFrame.current);
        if (initialSettleTimer.current) window.clearTimeout(initialSettleTimer.current);
        if (viewportSettleTimer.current) window.clearTimeout(viewportSettleTimer.current);
        viewportMoving.current = false;
        ForkApi.catalogue()
            .then((items) => {
                setCatalogue(items);
                const selected = items.flatMap((series) => series.volumes).find((item) => item.id === volumeId);
                if (selected) {
                    const savedPage = Math.max(0, Math.min(selected.progressPage, selected.pageCount - 1));
                    resumePage.current = savedPage;
                    setPage(savedPage);
                } else setError('This volume is hidden or unavailable.');
            })
            .catch((cause) => setError(cause instanceof Error ? cause.message : String(cause)));
        return () => {
            (window as any).MangaReaderNative?.setImmersive?.(false);
            if (zoomSettleTimer.current) window.clearTimeout(zoomSettleTimer.current);
            if (initialScrollFrame.current) window.cancelAnimationFrame(initialScrollFrame.current);
            if (initialSettleFrame.current) window.cancelAnimationFrame(initialSettleFrame.current);
            if (initialSettleTimer.current) window.clearTimeout(initialSettleTimer.current);
            if (viewportSettleTimer.current) window.clearTimeout(viewportSettleTimer.current);
            document.documentElement.classList.remove('ocr-zoom-gesture-active');
            document.documentElement.classList.remove('ocr-reader-motion-active');
        };
    }, [volumeId]);

    useEffect(() => {
        resetZoom();
    }, [preferences.direction, resetZoom]);

    useEffect(() => {
        const onVisibilityChange = () => {
            if (document.visibilityState === 'visible') resetInterruptedOcrInteraction();
        };
        document.addEventListener('visibilitychange', onVisibilityChange);
        window.addEventListener('pageshow', resetInterruptedOcrInteraction);
        window.addEventListener('focus', resetInterruptedOcrInteraction);
        return () => {
            document.removeEventListener('visibilitychange', onVisibilityChange);
            window.removeEventListener('pageshow', resetInterruptedOcrInteraction);
            window.removeEventListener('focus', resetInterruptedOcrInteraction);
        };
    }, [resetInterruptedOcrInteraction]);

    useEffect(() => {
        if (!owner) return undefined;
        sessionStorage.setItem('manga_reader_active_series_id', owner.id);
        return () => sessionStorage.removeItem('manga_reader_active_series_id');
    }, [owner]);

    useEffect(() => {
        if (!volume || page >= volume.pageCount) return undefined;
        if (continuous && !initialPositionReady.current) return undefined;
        const timer = window.setTimeout(() => {
            ForkApi.setProgress(volume.id, page).catch(() => undefined);
            ForkApi.recordEvent('page_encounter', {
                uniqueKey: `${volume.fingerprint}:${page}`,
                seriesId: owner?.id,
                sensitive: Boolean(owner?.sensitive || volume.sensitive),
            }).catch(() => undefined);
        }, 250);
        return () => {
            window.clearTimeout(timer);
        };
    }, [continuous, owner?.sensitive, page, volume]);

    useEffect(() => {
        const interval = window.setInterval(() => {
            if (document.visibilityState === 'visible' && Date.now() - lastInteraction.current < 60_000) {
                ForkApi.recordEvent('active_reading', {
                    value: 20,
                    seriesId: owner?.id,
                    sensitive: Boolean(owner?.sensitive || volume?.sensitive),
                }).catch(() => undefined);
            }
        }, 20_000);
        return () => window.clearInterval(interval);
    }, [owner?.sensitive, volume?.sensitive]);

    const scrollToPage = useCallback((target: number, behavior: ScrollBehavior = 'smooth') => {
        const root = scrollContainerRef.current;
        const element = pageRefs.current[target];
        if (!root || !element) return;
        const rootRect = root.getBoundingClientRect();
        const elementRect = element.getBoundingClientRect();
        root.scrollTo({ top: root.scrollTop + elementRect.top - rootRect.top, behavior });
    }, []);

    const settleInitialPosition = useCallback(
        (loadedPage: number) => {
            if (
                !continuous ||
                initialPositionReady.current ||
                initialPositionSettling.current ||
                resumePage.current !== loadedPage
            )
                return;
            const key = `${volume?.id}:vertical`;
            if (initialScrollKey.current !== key) return;
            initialPositionSettling.current = true;
            if (initialScrollFrame.current) window.cancelAnimationFrame(initialScrollFrame.current);
            if (initialSettleFrame.current) window.cancelAnimationFrame(initialSettleFrame.current);
            if (initialSettleTimer.current) window.clearTimeout(initialSettleTimer.current);

            // An image replacing its estimated placeholder changes the document geometry.
            // Re-anchor after that layout has painted, then wait briefly for adjacent eager
            // pages before allowing the observer to update or persist progress.
            initialScrollFrame.current = requestAnimationFrame(() => {
                scrollToPage(loadedPage, 'auto');
                initialSettleTimer.current = window.setTimeout(() => {
                    scrollToPage(loadedPage, 'auto');
                    initialSettleFrame.current = requestAnimationFrame(() => {
                        if (initialScrollKey.current !== key) return;
                        visibleRatios.current.clear();
                        initialPositionReady.current = true;
                        initialPositionSettling.current = false;
                        resumePage.current = undefined;
                    });
                }, 300);
            });
        },
        [continuous, scrollToPage, volume?.id],
    );

    const turn = (delta: number) => {
        if (!volume) return;
        lastInteraction.current = Date.now();
        const target = page + delta;
        if (target >= volume.pageCount) {
            const index = readableVolumes.findIndex((item) => item.id === volume.id);
            const next = readableVolumes[index + 1];
            if (next) navigate(`/fork/reader/${next.id}`);
            else
                ForkApi.recordEvent('volume_complete', {
                    uniqueKey: volume.fingerprint,
                    seriesId: owner?.id,
                    sensitive: Boolean(owner?.sensitive || volume.sensitive),
                }).catch(() => undefined);
            return;
        }
        if (target < 0) {
            const index = readableVolumes.findIndex((item) => item.id === volume.id);
            const previous = readableVolumes[index - 1];
            if (previous) navigate(`/fork/reader/${previous.id}`);
            return;
        }
        if (continuous) {
            scrollToPage(target);
        } else {
            setPage(target);
            resetZoom();
        }
    };

    useEffect(() => {
        if (!continuous || !volume) return undefined;
        const root = scrollContainerRef.current;
        if (!root) return undefined;

        const observer = new IntersectionObserver(
            (entries) => {
                if (zoomTracking.current || !initialPositionReady.current) return;
                entries.forEach((entry) => {
                    const entryPage = Number((entry.target as HTMLElement).dataset.page);
                    if (entry.isIntersecting) visibleRatios.current.set(entryPage, entry.intersectionRatio);
                    else visibleRatios.current.delete(entryPage);
                });
                const mostVisible = [...visibleRatios.current.entries()].sort(
                    ([leftPage, leftRatio], [rightPage, rightRatio]) => rightRatio - leftRatio || leftPage - rightPage,
                )[0];
                if (mostVisible) {
                    lastInteraction.current = Date.now();
                    setPage((current) => (current === mostVisible[0] ? current : mostVisible[0]));
                }
            },
            { root, threshold: [0.15, 0.35, 0.55, 0.75] },
        );
        pageRefs.current.forEach((element) => {
            if (element) observer.observe(element);
        });
        return () => {
            observer.disconnect();
            visibleRatios.current.clear();
        };
    }, [continuous, volume]);

    useEffect(() => {
        if (!continuous) {
            initialScrollKey.current = '';
            initialPositionReady.current = true;
            initialPositionSettling.current = false;
            resumePage.current = undefined;
            return;
        }
        if (!volume) return;
        const key = `${volume.id}:vertical`;
        const target = resumePage.current ?? page;
        if (initialScrollKey.current !== key) {
            initialScrollKey.current = key;
            initialPositionReady.current = false;
            initialScrollFrame.current = requestAnimationFrame(() => {
                scrollToPage(target, 'auto');
            });
        }
        // Cached images can finish before this effect first runs. In that case
        // their load event cannot perform the initial-position hand-off.
        if (loadedPages.has(target)) settleInitialPosition(target);
    }, [continuous, loadedPages, page, scrollToPage, settleInitialPosition, volume]);

    useEffect(() => {
        const keyboard = (event: KeyboardEvent) => {
            if (event.key === 'ArrowRight') turn(preferences.direction === 'rtl' ? -pageStep : pageStep);
            if (event.key === 'ArrowLeft') turn(preferences.direction === 'rtl' ? pageStep : -pageStep);
            if (event.key === 'ArrowDown' && continuous)
                scrollContainerRef.current?.scrollBy({ top: window.innerHeight * 0.85, behavior: 'smooth' });
            if (event.key === 'ArrowUp' && continuous)
                scrollContainerRef.current?.scrollBy({ top: -window.innerHeight * 0.85, behavior: 'smooth' });
            if (event.key === 'Escape') navigate('/library');
        };
        window.addEventListener('keydown', keyboard);
        return () => window.removeEventListener('keydown', keyboard);
    });

    const toggleBookmark = () => {
        const next = bookmarks.includes(bookmarkId)
            ? bookmarks.filter((item) => item !== bookmarkId)
            : [...bookmarks, bookmarkId];
        setBookmarks(next);
        localStorage.setItem(BOOKMARK_KEY, JSON.stringify(next));
    };

    return (
        <Box
            sx={{ position: 'fixed', inset: 0, bgcolor: '#090909', color: 'white', overflow: 'hidden', zIndex: 2000 }}
            onClick={() => {
                if (Date.now() < suppressReaderClickUntil.current) return;
                lastInteraction.current = Date.now();
                setControls((value) => !value);
            }}
        >
            {error && <Alert severity="error">{error}</Alert>}
            {volume && (
                <Box
                    ref={scrollContainerRef}
                    sx={{
                        width: '100%',
                        height: '100%',
                        overflow: continuous || scale > 1 ? 'auto' : 'hidden',
                        overscrollBehavior: 'contain',
                        WebkitOverflowScrolling: 'touch',
                        textAlign: 'center',
                    }}
                    onTouchStart={(event) => {
                        if (event.touches.length === 1) {
                            touchStart.current = { x: event.touches[0].clientX, y: event.touches[0].clientY };
                        } else {
                            touchStart.current = undefined;
                        }
                    }}
                    onScroll={beginViewportMotion}
                    onTouchEnd={(event) => {
                        if (touchStart.current && event.changedTouches.length === 1 && scale === 1 && !continuous) {
                            const dx = event.changedTouches[0].clientX - touchStart.current.x;
                            const dy = event.changedTouches[0].clientY - touchStart.current.y;
                            if (Math.abs(dx) > 60 && Math.abs(dx) > Math.abs(dy)) {
                                const forward = preferences.direction === 'rtl' ? dx > 0 : dx < 0;
                                turn(forward ? pageStep : -pageStep);
                            }
                        }
                        touchStart.current = undefined;
                    }}
                >
                    <Box ref={zoomSizerRef} sx={{ width: scale > 1 ? `${scale * 100}%` : '100%', minHeight: '100%' }}>
                        <Stack
                            ref={zoomContentRef}
                            direction={spread ? 'row' : 'column'}
                            sx={{
                                width: scale > 1 ? `${100 / scale}%` : '100%',
                                minHeight: '100%',
                                height: continuous ? 'auto' : '100%',
                                transform: `scale(${scale})`,
                                transformOrigin: 'top left',
                                transition: isZooming ? 'none' : 'transform 150ms ease-out',
                                willChange: isZooming ? 'transform' : 'auto',
                            }}
                        >
                            {(continuous
                                ? Array.from({ length: volume.pageCount }, (_, index) => index)
                                : [page, ...(spread && page + 1 < volume.pageCount ? [page + 1] : [])]
                            ).map((visiblePage) => (
                                <Box
                                    key={visiblePage}
                                    ref={(element: HTMLDivElement | null) => {
                                        pageRefs.current[visiblePage] = element;
                                    }}
                                    data-page={visiblePage}
                                    sx={{
                                        position: 'relative',
                                        width: pageWidth,
                                        minHeight:
                                            continuous && !loadedPages.has(visiblePage) ? 'min(150vw, 100vh)' : 0,
                                        flex: continuous ? '0 0 auto' : 1,
                                        display: 'flex',
                                        alignItems: continuous ? 'flex-start' : 'center',
                                        justifyContent: 'center',
                                        bgcolor: '#090909',
                                    }}
                                >
                                    <Box
                                        component="img"
                                        className="fork-reader-page"
                                        loading={continuous && Math.abs(visiblePage - page) > 2 ? 'lazy' : 'eager'}
                                        decoding="async"
                                        src={`/api/fork/volumes/${encodeURIComponent(volume.id)}/pages/${visiblePage}?fingerprint=${encodeURIComponent(volume.fingerprint)}`}
                                        alt={`Page ${visiblePage + 1} of ${volume.pageCount}`}
                                        onLoad={() => {
                                            setLoadedPages((current) => {
                                                if (current.has(visiblePage)) return current;
                                                return new Set(current).add(visiblePage);
                                            });
                                            settleInitialPosition(visiblePage);
                                        }}
                                        sx={{
                                            display: 'block',
                                            width: continuous || preferences.fit === 'width' ? '100%' : 'auto',
                                            height: continuous || preferences.fit === 'width' ? 'auto' : '100%',
                                            maxWidth: '100%',
                                            maxHeight: continuous ? 'none' : '100%',
                                            objectFit: 'contain',
                                            userSelect: 'none',
                                            WebkitUserDrag: 'none',
                                        }}
                                    />
                                </Box>
                            ))}
                        </Stack>
                    </Box>
                </Box>
            )}
            {volume && !settings.disableStatusIcon && (
                <Stack
                    role="status"
                    aria-live="polite"
                    aria-label={`${currentOcrLabel} for page ${page + 1}`}
                    direction="row"
                    alignItems="center"
                    spacing={0.5}
                    sx={{
                        position: 'absolute',
                        right: 10,
                        bottom: controls ? 70 : 10,
                        zIndex: 20,
                        height: 24,
                        px: 0.75,
                        borderRadius: 1.5,
                        border: '1px solid rgba(255,255,255,.2)',
                        bgcolor: 'rgba(0,0,0,.72)',
                        color: currentOcrStatus === 'error' ? '#ffb4ab' : '#fff',
                        pointerEvents: 'none',
                        backdropFilter: 'blur(6px)',
                        transition: 'bottom 150ms ease-out',
                    }}
                >
                    <Typography sx={{ fontSize: 10, fontWeight: 700, letterSpacing: '.08em', lineHeight: 1 }}>
                        OCR
                    </Typography>
                    {currentOcrStatus === 'success' ? (
                        <CheckRoundedIcon sx={{ fontSize: 14 }} />
                    ) : currentOcrStatus === 'error' ? (
                        <ErrorOutlineRoundedIcon sx={{ fontSize: 14 }} />
                    ) : (
                        <CircularProgress color="inherit" size={11} thickness={5} />
                    )}
                </Stack>
            )}
            {volume && controls && (
                <>
                    <Stack
                        direction="row"
                        alignItems="center"
                        spacing={wide ? 1 : 0}
                        sx={{
                            position: 'absolute',
                            top: 0,
                            left: 0,
                            right: 0,
                            p: wide ? 1 : 0.5,
                            bgcolor: 'rgba(0,0,0,.72)',
                        }}
                        onClick={(event) => event.stopPropagation()}
                    >
                        <IconButton
                            size="small"
                            color="inherit"
                            aria-label="Back to manga library"
                            onClick={() => navigate('/library')}
                        >
                            <ArrowBackIcon />
                        </IconButton>
                        <Box sx={{ minWidth: 0, flex: 1, display: wide ? 'block' : 'none' }}>
                            <Typography noWrap>{owner?.title}</Typography>
                            <Typography variant="caption" noWrap>
                                {volume.title}
                            </Typography>
                        </Box>
                        <IconButton
                            size="small"
                            color="inherit"
                            aria-label="Zoom out"
                            onClick={() => zoomTo(scale - 0.5)}
                        >
                            <ZoomOutIcon />
                        </IconButton>
                        <IconButton size="small" color="inherit" aria-label="Fit page" onClick={() => zoomTo(1)}>
                            <FitScreenIcon />
                        </IconButton>
                        <IconButton size="small" color="inherit" aria-label="Toggle bookmark" onClick={toggleBookmark}>
                            {bookmarks.includes(bookmarkId) ? <BookmarkIcon /> : <BookmarkBorderIcon />}
                        </IconButton>
                        <IconButton
                            size="small"
                            color="inherit"
                            aria-label={`Reading direction: ${preferences.direction}`}
                            onClick={() => {
                                const order = ['rtl', 'ltr', 'vertical'] as const;
                                const direction = order[(order.indexOf(preferences.direction) + 1) % order.length];
                                writeReaderPreferences({ ...preferences, direction });
                            }}
                        >
                            <Typography variant="caption">
                                {preferences.direction === 'vertical' ? '↕' : preferences.direction.toUpperCase()}
                            </Typography>
                        </IconButton>
                        <IconButton
                            size="small"
                            color="inherit"
                            aria-label="Zoom in"
                            onClick={() => zoomTo(scale + 0.5)}
                        >
                            <ZoomInIcon />
                        </IconButton>
                    </Stack>
                    <Stack
                        sx={{
                            position: 'absolute',
                            bottom: 0,
                            left: 0,
                            right: 0,
                            px: 1,
                            py: 0.75,
                            bgcolor: 'rgba(0,0,0,.9)',
                            borderTop: '1px solid rgba(255,255,255,.48)',
                            backdropFilter: 'blur(8px)',
                        }}
                        onClick={(event) => event.stopPropagation()}
                    >
                        <Stack direction="row-reverse" alignItems="center" spacing={2}>
                            <IconButton
                                color="inherit"
                                disabled={
                                    page === 0 && readableVolumes.findIndex((item) => item.id === volume.id) === 0
                                }
                                aria-label="Previous page"
                                onClick={() => turn(-pageStep)}
                            >
                                <ArrowForwardIcon />
                            </IconButton>
                            <Slider
                                value={page}
                                min={0}
                                max={Math.max(0, volume.pageCount - 1)}
                                onChange={(_, value) => {
                                    const target = value as number;
                                    if (continuous) scrollToPage(target, 'auto');
                                    else {
                                        setPage(target);
                                        resetZoom();
                                    }
                                }}
                                valueLabelDisplay="auto"
                                valueLabelFormat={(value) => `${value + 1}`}
                                aria-label="Page scrubber"
                                sx={{
                                    flex: 1,
                                    height: 3,
                                    color: '#fff',
                                    '& .MuiSlider-rail': { opacity: 0.3, height: 3 },
                                    '& .MuiSlider-track': { border: 0, height: 3 },
                                    '& .MuiSlider-thumb': {
                                        width: 12,
                                        height: 12,
                                        boxShadow: '0 0 0 1px #000',
                                        '&:before': { boxShadow: 'none' },
                                    },
                                    '& .MuiSlider-valueLabel': {
                                        bgcolor: '#fff',
                                        color: '#000',
                                        borderRadius: 1.5,
                                        fontWeight: 700,
                                    },
                                }}
                            />
                            <Typography variant="body2" fontWeight={700} sx={{ whiteSpace: 'nowrap' }}>
                                {page + 1} / {volume.pageCount}
                            </Typography>
                            <IconButton
                                color="inherit"
                                disabled={
                                    page + pageStep >= volume.pageCount &&
                                    readableVolumes.findIndex((item) => item.id === volume.id) + 1 >=
                                        readableVolumes.length
                                }
                                aria-label="Next page"
                                onClick={() => turn(pageStep)}
                            >
                                <ArrowBackIcon />
                            </IconButton>
                        </Stack>
                    </Stack>
                </>
            )}
        </Box>
    );
}
