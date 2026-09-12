import Box from '@mui/material/Box';
import Slider from '@mui/material/Slider';
import Typography from '@mui/material/Typography';
import { useEffect, useState } from 'react';
import { ReadingDirection, ReadingMode } from '@/features/reader/Reader.types';
import { ReaderControls } from '@/features/reader/services/ReaderControls';
import {
    useReaderPagesStore,
    useReaderOverlayStore,
    useReaderScrollbarStore,
    useReaderSettingsStore,
} from '@/features/reader/stores/ReaderStore';

const valueFromSlider = (value: number | number[]) => (Array.isArray(value) ? value[0] : value);

export const ForkReaderPageScrubber = () => {
    const isOverlayVisible = useReaderOverlayStore((state) => state.overlay.isVisible);
    const { currentPageIndex, totalPages } = useReaderPagesStore((state) => ({
        currentPageIndex: state.pages.currentPageIndex,
        totalPages: state.pages.totalPages,
    }));
    const { readingMode, readingDirection } = useReaderSettingsStore((state) => ({
        readingMode: state.settings.readingMode.value,
        readingDirection: state.settings.readingDirection.value,
    }));
    const scrollbar = useReaderScrollbarStore((state) => state.scrollbar);
    const pageIndex = Math.max(0, currentPageIndex);
    const lastPageIndex = Math.max(0, totalPages - 1);
    const isVertical = [ReadingMode.CONTINUOUS_VERTICAL, ReadingMode.WEBTOON].includes(readingMode);
    const isReversed = isVertical || readingDirection === ReadingDirection.RTL;
    const toDisplayValue = (index: number) => (isReversed ? lastPageIndex - index : index);
    const [displayValue, setDisplayValue] = useState(toDisplayValue(Math.max(0, pageIndex)));

    useEffect(() => {
        setDisplayValue(toDisplayValue(Math.max(0, pageIndex)));
    }, [pageIndex, lastPageIndex, isReversed]);

    if (!isOverlayVisible || totalPages < 2) return null;

    const currentPageNumber = Math.min(totalPages, pageIndex + 1);

    const openSelectedPage = (rawValue: number | number[]) => {
        const selectedDisplayIndex = Math.round(valueFromSlider(rawValue));
        const selectedPageIndex = isReversed ? lastPageIndex - selectedDisplayIndex : selectedDisplayIndex;
        ReaderControls.openPage(Math.max(0, Math.min(lastPageIndex, selectedPageIndex)), undefined, false);
    };

    const sharedSliderStyles = {
        color: '#fff',
        p: 0,
        pointerEvents: 'auto',
        '& .MuiSlider-rail': { opacity: 0.34, bgcolor: '#fff' },
        '& .MuiSlider-track': { border: 0, bgcolor: '#fff' },
        '& .MuiSlider-thumb': {
            width: 12,
            height: 12,
            bgcolor: '#fff',
            boxShadow: '0 0 0 1px rgba(0,0,0,.75)',
            '&::before': { boxShadow: 'none' },
        },
    } as const;

    return (
        <Box
            sx={{
                position: 'fixed',
                zIndex: 3,
                pointerEvents: 'none',
                boxSizing: 'border-box',
                bgcolor: 'rgba(0,0,0,0.62)',
                border: '1px solid rgba(255,255,255,0.3)',
                borderRadius: 99,
                backdropFilter: 'blur(3px)',
                ...(isVertical
                    ? {
                          top: 88,
                          right: `max(${scrollbar.ySize + 9}px, env(safe-area-inset-right))`,
                          bottom: `max(${scrollbar.xSize + 88}px, env(safe-area-inset-bottom))`,
                          width: 30,
                          py: 1.5,
                      }
                    : {
                          right: `max(${scrollbar.ySize + 18}px, env(safe-area-inset-right))`,
                          bottom: `max(${scrollbar.xSize + 54}px, env(safe-area-inset-bottom))`,
                          left: 'max(18px, env(safe-area-inset-left))',
                          height: 30,
                          px: 1.5,
                      }),
            }}
        >
            <Typography
                aria-live="polite"
                sx={{
                    position: 'absolute',
                    color: '#fff',
                    bgcolor: 'rgba(0,0,0,0.78)',
                    border: '1px solid rgba(255,255,255,0.3)',
                    borderRadius: 99,
                    px: 0.8,
                    py: 0.2,
                    fontSize: 12,
                    fontWeight: 600,
                    lineHeight: 1.25,
                    whiteSpace: 'nowrap',
                    userSelect: 'none',
                    ...(isVertical
                        ? { top: 0, right: 'calc(100% + 8px)' }
                        : { top: -8, right: 8, transform: 'translateY(-100%)' }),
                }}
            >
                {currentPageNumber} / {totalPages}
            </Typography>
            <Slider
                aria-label="Page navigation"
                orientation={isVertical ? 'vertical' : 'horizontal'}
                min={0}
                max={lastPageIndex}
                step={1}
                value={Math.max(0, Math.min(lastPageIndex, displayValue))}
                onChange={(_, value) => setDisplayValue(valueFromSlider(value))}
                onChangeCommitted={(_, value) => openSelectedPage(value)}
                sx={{
                    ...sharedSliderStyles,
                    ...(isVertical ? { height: '100%' } : { width: '100%' }),
                }}
            />
        </Box>
    );
};
