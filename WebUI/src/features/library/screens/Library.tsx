import SearchIcon from '@mui/icons-material/Search';
import TuneIcon from '@mui/icons-material/Tune';
import ChecklistIcon from '@mui/icons-material/Checklist';
import RefreshIcon from '@mui/icons-material/Refresh';
import Alert from '@mui/material/Alert';
import Box from '@mui/material/Box';
import Button from '@mui/material/Button';
import Card from '@mui/material/Card';
import CardActionArea from '@mui/material/CardActionArea';
import Checkbox from '@mui/material/Checkbox';
import Chip from '@mui/material/Chip';
import CircularProgress from '@mui/material/CircularProgress';
import Collapse from '@mui/material/Collapse';
import IconButton from '@mui/material/IconButton';
import MenuItem from '@mui/material/MenuItem';
import Stack from '@mui/material/Stack';
import TextField from '@mui/material/TextField';
import Typography from '@mui/material/Typography';
import { TouchEvent, useEffect, useMemo, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { ForkApi, ForkSeries } from '@/features/fork/ForkApi';
import { useForkState } from '@/features/fork/useForkState';
import { useAppTitle } from '@/features/navigation-bar/hooks/useAppTitle';
import { ForkProgress } from '@/features/fork/components/ForkProgress';
import { ForkSelectionBar } from '@/features/fork/components/ForkSelectionBar';
import { useOCR } from '@/Manatan/context/OCRContext';

const FILTER_KEY = 'manga_reader_library_filter_v1';
const SORT_KEY = 'manga_reader_library_sort_v1';
const SOURCE_KEY = 'manga_reader_library_source_v1';
const READING_KEY = 'manga_reader_library_reading_v1';
const CATEGORY_KEY = 'manga_reader_library_category_v1';

export function Library() {
    useAppTitle('Manga', 'Manga');
    const navigate = useNavigate();
    const { state } = useForkState();
    const { settings, showConfirm } = useOCR();
    const cachedCatalogue = ForkApi.cachedCatalogue();
    const [series, setSeries] = useState<ForkSeries[]>(cachedCatalogue ?? []);
    const [loading, setLoading] = useState(!cachedCatalogue);
    const [error, setError] = useState<string>();
    const [query, setQuery] = useState(() => localStorage.getItem(FILTER_KEY) ?? '');
    const [selected, setSelected] = useState<string[]>([]);
    const [selecting, setSelecting] = useState(false);
    const [selectedCategory, setSelectedCategory] = useState('');
    const [sort, setSort] = useState(() => localStorage.getItem(SORT_KEY) ?? 'title');
    const [sourceFilter, setSourceFilter] = useState(() => localStorage.getItem(SOURCE_KEY) ?? 'all');
    const [readingFilter, setReadingFilter] = useState(() => localStorage.getItem(READING_KEY) ?? 'all');
    const [categoryFilter, setCategoryFilter] = useState(() => localStorage.getItem(CATEGORY_KEY) ?? 'all');
    const [filtersOpen, setFiltersOpen] = useState(false);
    const [actionStatus, setActionStatus] = useState<string>();
    const [refreshingFolders, setRefreshingFolders] = useState(false);
    const pullStartY = useRef<number>();

    const load = async (showSpinner = true) => {
        if (showSpinner) setLoading(true);
        try {
            setSeries(await ForkApi.catalogue());
            setError(undefined);
        } catch (cause) {
            setError(cause instanceof Error ? cause.message : String(cause));
        } finally {
            setLoading(false);
        }
    };
    useEffect(() => {
        load(!ForkApi.cachedCatalogue());
    }, [state?.showSensitive]);
    const refreshLocalFolders = async () => {
        if (refreshingFolders) return;
        const localSources = state?.sources.filter((source) => source.kind === 'local') ?? [];
        if (localSources.length === 0) {
            setActionStatus('Add an on-device folder in Settings before refreshing.');
            return;
        }
        setRefreshingFolders(true);
        try {
            const results = await Promise.all(localSources.map((source) => ForkApi.refreshSource(source.id)));
            const newSeries = results.reduce((total, result) => total + result.newSeries, 0);
            const newVolumes = results.reduce((total, result) => total + result.newVolumes, 0);
            await load(false);
            setActionStatus(
                newVolumes === 0
                    ? 'Your folders are up to date — no new volumes found.'
                    : `Added ${newVolumes} new volume${newVolumes === 1 ? '' : 's'}${newSeries ? ` across ${newSeries} new series` : ''}.`,
            );
        } catch (cause) {
            setActionStatus(cause instanceof Error ? cause.message : String(cause));
        } finally {
            setRefreshingFolders(false);
        }
    };
    const startPullToRefresh = (event: TouchEvent) => {
        pullStartY.current = window.scrollY <= 0 ? event.touches[0]?.clientY : undefined;
    };
    const finishPullToRefresh = (event: TouchEvent) => {
        const startY = pullStartY.current;
        pullStartY.current = undefined;
        if (startY !== undefined && event.changedTouches[0] && event.changedTouches[0].clientY - startY >= 80) {
            void refreshLocalFolders();
        }
    };
    const filtered = useMemo(() => {
        const needle = query.trim().toLocaleLowerCase();
        const result = series.filter((item) => {
            if (needle && !item.title.toLocaleLowerCase().includes(needle)) return false;
            if (sourceFilter !== 'all' && item.sourceId !== sourceFilter) return false;
            if (categoryFilter !== 'all' && !item.categoryIds.includes(categoryFilter)) return false;
            const pages = item.volumes.reduce((count, volume) => count + volume.pageCount, 0);
            const progress = item.volumes.reduce((count, volume) => count + volume.progressPage, 0);
            if (readingFilter === 'unread' && progress > 0) return false;
            if (readingFilter === 'reading' && (progress === 0 || progress >= Math.max(0, pages - item.volumes.length)))
                return false;
            if (readingFilter === 'complete' && progress < Math.max(0, pages - item.volumes.length)) return false;
            return true;
        });
        result.sort((left, right) => {
            if (sort === 'progress') {
                const progress = (item: ForkSeries) =>
                    item.volumes.reduce((count, volume) => count + volume.progressPage, 0);
                return progress(right) - progress(left);
            }
            if (sort === 'volumes') return right.volumes.length - left.volumes.length;
            return left.title.localeCompare(right.title, undefined, { numeric: true, sensitivity: 'base' });
        });
        return result;
    }, [categoryFilter, query, readingFilter, series, sort, sourceFilter]);
    const toggle = (id: string) =>
        setSelected((old) => (old.includes(id) ? old.filter((value) => value !== id) : [...old, id]));
    const leaveSelection = () => {
        setSelected([]);
        setSelecting(false);
    };
    const prepareSelected = (reprocess: boolean) => {
        const selectedItems = series.filter((item) => selected.includes(item.id));
        const pages = selectedItems.reduce(
            (count, item) =>
                count +
                item.volumes.reduce((volumeCount, volume) => volumeCount + (volume.error ? 0 : volume.pageCount), 0),
            0,
        );
        showConfirm(
            reprocess ? 'Reprocess OCR?' : 'Prepare OCR?',
            `${selectedItems.length} series · ${pages} pages`,
            () => {
                ForkApi.prepareOcr(selected, reprocess, {
                    ocrProvider: settings.ocrProvider,
                    selfHostedUrl: settings.selfHostedOcrUrl,
                })
                    .then(async (job) => {
                        setActionStatus(
                            `Queued ${job.totalPages ?? 0} pages for ${reprocess ? 'fresh recognition' : 'OCR'}. Progress is available in Settings → Background activity.`,
                        );
                        setSelected([]);
                        await load();
                    })
                    .catch((cause) => setActionStatus(cause instanceof Error ? cause.message : String(cause)));
            },
        );
    };

    const downloadSelected = () => {
        const selectedItems = series.filter((item) => selected.includes(item.id));
        const remoteItems = selectedItems.filter(
            (item) => state?.sources.find((source) => source.id === item.sourceId)?.kind !== 'local',
        );
        const pages = remoteItems.reduce(
            (count, item) =>
                count + item.volumes.reduce((sum, volume) => sum + (volume.error ? 0 : volume.pageCount), 0),
            0,
        );
        showConfirm('Make available offline?', `${remoteItems.length} remote series · ${pages} pages`, () => {
            ForkApi.downloadSeries(remoteItems.map((item) => item.id))
                .then(async (job) => {
                    setActionStatus(
                        `Queued ${job.totalPages ?? 0} pages. Progress is available in Settings → Background activity.`,
                    );
                    setSelected([]);
                    await load();
                })
                .catch((cause) => setActionStatus(cause instanceof Error ? cause.message : String(cause)));
        });
    };

    return (
        <Stack
            spacing={2}
            sx={{ p: { xs: 1.5, sm: 3 }, pb: { xs: 11, sm: 10 }, maxWidth: 1200, mx: 'auto' }}
            onTouchStart={startPullToRefresh}
            onTouchEnd={finishPullToRefresh}
        >
            <Stack direction="row" spacing={1} alignItems="center">
                <TextField
                    size="small"
                    fullWidth
                    value={query}
                    onChange={(event) => {
                        setQuery(event.target.value);
                        localStorage.setItem(FILTER_KEY, event.target.value);
                    }}
                    placeholder="Search library"
                    InputProps={{
                        startAdornment: <SearchIcon fontSize="small" sx={{ mr: 1 }} />,
                        inputProps: { 'aria-label': 'Search your manga library' },
                    }}
                />
                <IconButton
                    color={filtersOpen ? 'primary' : 'default'}
                    aria-label={filtersOpen ? 'Hide library filters' : 'Show library filters'}
                    aria-expanded={filtersOpen}
                    onClick={() => setFiltersOpen((open) => !open)}
                >
                    <TuneIcon />
                </IconButton>
                <IconButton
                    color={selecting ? 'primary' : 'default'}
                    aria-label={selecting ? 'Exit selection mode' : 'Select manga'}
                    onClick={() => (selecting ? leaveSelection() : setSelecting(true))}
                >
                    <ChecklistIcon />
                </IconButton>
                <IconButton
                    color="default"
                    aria-label="Check local folders for new manga"
                    disabled={refreshingFolders}
                    onClick={() => void refreshLocalFolders()}
                >
                    {refreshingFolders ? <CircularProgress size={20} /> : <RefreshIcon />}
                </IconButton>
            </Stack>
            <Collapse in={filtersOpen} unmountOnExit>
                <Stack spacing={1}>
                    <Stack direction="row" spacing={0.75}>
                        <TextField
                            select
                            size="small"
                            label="Source"
                            value={sourceFilter}
                            onChange={(event) => {
                                setSourceFilter(event.target.value);
                                localStorage.setItem(SOURCE_KEY, event.target.value);
                            }}
                            sx={{ flex: 1, minWidth: 0 }}
                        >
                            <MenuItem value="all">All</MenuItem>
                            {state?.sources.map((source) => (
                                <MenuItem key={source.id} value={source.id}>
                                    {source.name}
                                </MenuItem>
                            ))}
                        </TextField>
                        <TextField
                            select
                            size="small"
                            label="Status"
                            value={readingFilter}
                            onChange={(event) => {
                                setReadingFilter(event.target.value);
                                localStorage.setItem(READING_KEY, event.target.value);
                            }}
                            sx={{ flex: 1, minWidth: 0 }}
                        >
                            <MenuItem value="all">Any</MenuItem>
                            <MenuItem value="unread">Unread</MenuItem>
                            <MenuItem value="reading">Reading</MenuItem>
                            <MenuItem value="complete">Done</MenuItem>
                        </TextField>
                        <TextField
                            select
                            size="small"
                            label="Sort"
                            value={sort}
                            onChange={(event) => {
                                setSort(event.target.value);
                                localStorage.setItem(SORT_KEY, event.target.value);
                            }}
                            sx={{ flex: 1, minWidth: 0 }}
                        >
                            <MenuItem value="title">Title</MenuItem>
                            <MenuItem value="progress">Continue</MenuItem>
                            <MenuItem value="volumes">Volumes</MenuItem>
                        </TextField>
                    </Stack>
                    {(state?.categories.length ?? 0) > 0 && (
                        <Stack direction="row" spacing={1} flexWrap="wrap" useFlexGap>
                            <Chip
                                size="small"
                                label="All shelves"
                                color={categoryFilter === 'all' ? 'primary' : 'default'}
                                onClick={() => {
                                    setCategoryFilter('all');
                                    localStorage.setItem(CATEGORY_KEY, 'all');
                                }}
                            />
                            {state?.categories.map((category) => (
                                <Chip
                                    size="small"
                                    key={category.id}
                                    label={category.name}
                                    color={categoryFilter === category.id ? 'primary' : 'default'}
                                    onClick={() => {
                                        setCategoryFilter(category.id);
                                        localStorage.setItem(CATEGORY_KEY, category.id);
                                    }}
                                />
                            ))}
                        </Stack>
                    )}
                </Stack>
            </Collapse>
            {selecting && selected.length > 0 && (
                <ForkSelectionBar count={selected.length} onClose={leaveSelection}>
                    <Button variant="contained" onClick={() => prepareSelected(false)}>
                        OCR
                    </Button>
                    <Button color="inherit" onClick={() => prepareSelected(true)}>
                        Reprocess
                    </Button>
                    <Button color="inherit" onClick={downloadSelected}>
                        Offline
                    </Button>
                    {(state?.categories?.length ?? 0) > 0 && (
                        <TextField
                            select
                            size="small"
                            label="Category"
                            value={selectedCategory}
                            onChange={(event) => setSelectedCategory(event.target.value)}
                            sx={{ minWidth: 150 }}
                        >
                            {state?.categories.map((category) => (
                                <MenuItem key={category.id} value={category.id}>
                                    {category.name}
                                </MenuItem>
                            ))}
                        </TextField>
                    )}
                    <Button
                        color="inherit"
                        disabled={!selectedCategory}
                        onClick={() => ForkApi.setSeriesCategory(selected, selectedCategory, true).then(() => load())}
                    >
                        Add shelf
                    </Button>
                    <Button
                        color="inherit"
                        onClick={() =>
                            ForkApi.setSeriesSensitive(selected, true).then(() => {
                                setSelected([]);
                                return load();
                            })
                        }
                    >
                        Mark hidden
                    </Button>
                    <Button
                        color="inherit"
                        onClick={() =>
                            ForkApi.setSeriesSensitive(selected, false).then(() => {
                                setSelected([]);
                                return load();
                            })
                        }
                    >
                        Remove hidden
                    </Button>
                    <Button
                        color="inherit"
                        disabled={!selectedCategory}
                        onClick={() => ForkApi.setSeriesCategory(selected, selectedCategory, false).then(() => load())}
                    >
                        Remove shelf
                    </Button>
                </ForkSelectionBar>
            )}
            {actionStatus && (
                <Alert severity="info" onClose={() => setActionStatus(undefined)}>
                    {actionStatus}
                </Alert>
            )}
            {error && (
                <Alert severity="error" action={<Button onClick={() => load()}>Retry</Button>}>
                    {error}
                </Alert>
            )}
            {loading && (
                <Box sx={{ py: 8, textAlign: 'center' }}>
                    <CircularProgress aria-label="Loading library" />
                </Box>
            )}
            {!loading && !error && series.length === 0 && (
                <Box sx={{ py: 7, textAlign: 'center' }}>
                    <Typography variant="h6">Your manga library is empty</Typography>
                    <Typography color="text.secondary" sx={{ mb: 2 }}>
                        Add a source in Settings, then scan it. An unavailable source will not erase imported entries.
                    </Typography>
                    <Button variant="contained" onClick={() => navigate('/settings')}>
                        Add or scan a source
                    </Button>
                </Box>
            )}
            <Box
                sx={{
                    display: 'grid',
                    gridTemplateColumns: 'repeat(2, minmax(0, 1fr))',
                    columnGap: { xs: 1.5, sm: 2 },
                    rowGap: 2.5,
                }}
            >
                {filtered.map((item) => {
                    const firstVolume = item.volumes.find((volume) => volume.pageCount > 0 && !volume.error);
                    const readablePages = item.volumes.reduce(
                        (total, volume) => total + Math.max(0, volume.pageCount - 1),
                        0,
                    );
                    const readPages = item.volumes.reduce((total, volume) => total + volume.progressPage, 0);
                    const isSelected = selected.includes(item.id);
                    return (
                        <Card
                            key={item.id}
                            elevation={0}
                            sx={{ bgcolor: 'transparent', overflow: 'visible', borderRadius: 0 }}
                        >
                            <Box sx={{ position: 'relative' }}>
                                <CardActionArea
                                    onContextMenu={(event) => {
                                        event.preventDefault();
                                        setSelecting(true);
                                        toggle(item.id);
                                    }}
                                    onClick={() =>
                                        selecting
                                            ? toggle(item.id)
                                            : firstVolume && navigate(`/fork/series/${encodeURIComponent(item.id)}`)
                                    }
                                    aria-label={
                                        firstVolume ? `Open ${item.title}` : `${item.title} has no readable volumes`
                                    }
                                >
                                    <Box
                                        component="img"
                                        src={firstVolume ? `/api/fork/volumes/${firstVolume.id}/pages/0` : undefined}
                                        alt=""
                                        sx={{
                                            width: '100%',
                                            aspectRatio: '2 / 3',
                                            objectFit: 'cover',
                                            bgcolor: 'action.hover',
                                            display: 'block',
                                            border: isSelected ? '2px solid #fff' : '1px solid rgba(255,255,255,0.28)',
                                            borderRadius: 1.5,
                                        }}
                                    />
                                </CardActionArea>
                                {selecting && (
                                    <Checkbox
                                        checked={isSelected}
                                        onChange={() => toggle(item.id)}
                                        inputProps={{ 'aria-label': `Select ${item.title}` }}
                                        sx={{
                                            position: 'absolute',
                                            top: 5,
                                            right: 5,
                                            bgcolor: 'rgba(0,0,0,0.78)',
                                            p: 0.35,
                                            borderRadius: 99,
                                            color: '#fff',
                                            '&.Mui-checked': { color: '#fff' },
                                        }}
                                    />
                                )}
                            </Box>
                            <ForkProgress
                                value={(readPages / Math.max(1, readablePages)) * 100}
                                label={`${item.title} reading progress`}
                            />
                            <Typography
                                title={item.title}
                                sx={{
                                    mt: 0.25,
                                    fontSize: '0.91rem',
                                    lineHeight: 1.3,
                                    display: '-webkit-box',
                                    WebkitLineClamp: 2,
                                    WebkitBoxOrient: 'vertical',
                                    overflow: 'hidden',
                                }}
                            >
                                {item.title}
                            </Typography>
                        </Card>
                    );
                })}
            </Box>
        </Stack>
    );
}
