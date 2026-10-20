import ChecklistIcon from '@mui/icons-material/Checklist';
import CheckCircleIcon from '@mui/icons-material/CheckCircle';
import DocumentScannerOutlinedIcon from '@mui/icons-material/DocumentScannerOutlined';
import DoneAllIcon from '@mui/icons-material/DoneAll';
import ErrorOutlineIcon from '@mui/icons-material/ErrorOutline';
import ReplayIcon from '@mui/icons-material/Replay';
import Alert from '@mui/material/Alert';
import Box from '@mui/material/Box';
import Button from '@mui/material/Button';
import Card from '@mui/material/Card';
import CardActionArea from '@mui/material/CardActionArea';
import Checkbox from '@mui/material/Checkbox';
import CircularProgress from '@mui/material/CircularProgress';
import IconButton from '@mui/material/IconButton';
import Stack from '@mui/material/Stack';
import Tooltip from '@mui/material/Tooltip';
import Typography from '@mui/material/Typography';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useNavigate, useParams } from 'react-router-dom';
import { ForkApi, ForkSeries, OcrState } from '@/features/fork/ForkApi';
import { useForkState } from '@/features/fork/useForkState';
import { useAppTitle } from '@/features/navigation-bar/hooks/useAppTitle';
import { ForkProgress } from '@/features/fork/components/ForkProgress';
import { ForkSelectionBar } from '@/features/fork/components/ForkSelectionBar';
import { useOCR } from '@/Manatan/context/OCRContext';
import { ReaderResumeMode } from '@/features/reader/Reader.types';
import { useBackDismiss } from '@/features/fork/useBackDismiss';

const messageOf = (cause: unknown) => (cause instanceof Error ? cause.message : String(cause));

const OCR_LABELS: Record<OcrState, string> = {
    not_prepared: 'OCR not prepared',
    queued: 'OCR queued',
    processing: 'OCR processing',
    partial: 'OCR partially processed',
    ready: 'OCR processed',
    failed: 'OCR failed',
    out_of_date: 'OCR needs reprocessing',
};

function VolumeOcrIndicator({ state }: { state: OcrState }) {
    const ready = state === 'ready';
    const active = state === 'queued' || state === 'processing';
    const problem = state === 'failed' || state === 'partial' || state === 'out_of_date';
    let color = 'text.disabled';
    if (ready) color = 'success.main';
    if (problem) color = 'warning.main';
    if (state === 'failed') color = 'error.main';

    return (
        <Tooltip title={OCR_LABELS[state]} placement="top" arrow>
            <Box
                component="span"
                role="img"
                aria-label={OCR_LABELS[state]}
                sx={{
                    position: 'relative',
                    display: 'inline-flex',
                    width: 18,
                    height: 18,
                    flex: '0 0 auto',
                    alignItems: 'center',
                    justifyContent: 'center',
                    color,
                    opacity: state === 'not_prepared' ? 0.42 : 0.9,
                }}
            >
                <DocumentScannerOutlinedIcon sx={{ fontSize: 16 }} />
                {ready && (
                    <CheckCircleIcon
                        sx={{
                            position: 'absolute',
                            right: -2,
                            bottom: -2,
                            fontSize: 8,
                            color: 'success.main',
                            bgcolor: '#000',
                            borderRadius: '50%',
                        }}
                    />
                )}
                {active && (
                    <CircularProgress
                        size={8}
                        thickness={6}
                        sx={{ position: 'absolute', right: -2, bottom: -2, bgcolor: '#000', borderRadius: '50%' }}
                    />
                )}
                {problem && (
                    <ErrorOutlineIcon
                        sx={{
                            position: 'absolute',
                            right: -2,
                            bottom: -2,
                            fontSize: 8,
                            bgcolor: '#000',
                            borderRadius: '50%',
                        }}
                    />
                )}
            </Box>
        </Tooltip>
    );
}

export function SeriesDetail() {
    const { seriesId = '' } = useParams<{ seriesId: string }>();
    const navigate = useNavigate();
    const { state } = useForkState();
    const { settings, showConfirm } = useOCR();
    // Render instantly from the Library's catalogue cache, then revalidate.
    const [series, setSeries] = useState<ForkSeries | undefined>(() =>
        ForkApi.cachedCatalogue()?.find((candidate) => candidate.id === seriesId),
    );
    const [selected, setSelected] = useState<string[]>([]);
    const [selecting, setSelecting] = useState(false);
    const [status, setStatus] = useState<string>();
    useAppTitle(series?.title ?? 'Volumes', [series?.title]);
    const latestLoad = useRef(0);
    const load = useCallback(async () => {
        // Ignore responses that finish after a newer load or after leaving this series.
        latestLoad.current += 1;
        const loadId = latestLoad.current;
        const item = (await ForkApi.catalogue()).find((candidate) => candidate.id === seriesId);
        if (loadId !== latestLoad.current) return;
        setSeries(item);
        if (!item) setStatus('This series is hidden or unavailable.');
    }, [seriesId]);
    useEffect(() => {
        load().catch((error) => setStatus(messageOf(error)));
        return () => {
            latestLoad.current += 1;
        };
    }, [load]);
    const toggle = (id: string) =>
        setSelected((current) => (current.includes(id) ? current.filter((value) => value !== id) : [...current, id]));
    const leaveSelection = () => {
        setSelected([]);
        setSelecting(false);
    };
    useBackDismiss(selecting, leaveSelection);
    const run = (work: () => Promise<unknown>) => {
        work().catch((error) => setStatus(messageOf(error)));
    };
    const applySensitive = async (sensitive: boolean) => {
        await ForkApi.setVolumeSensitive(selected, sensitive);
        setSelected([]);
        await load();
    };
    const setReadState = async (volumeIds: string[], read: boolean, clearSelection = false) => {
        if (!series) return;
        const volumes = series.volumes.filter(
            (volume) => volumeIds.includes(volume.id) && volume.pageCount > 0 && !volume.error,
        );
        await Promise.all(
            volumes.map((volume) => ForkApi.setProgress(volume.id, read ? Math.max(0, volume.pageCount - 1) : 0)),
        );
        if (clearSelection) setSelected([]);
        setStatus(
            `${read ? 'Marked read' : 'Marked unread'}: ${volumes.length} volume${volumes.length === 1 ? '' : 's'}.`,
        );
        await load();
    };
    const prepare = (reprocess = false) => {
        if (!series) return;
        const volumes = series.volumes.filter((volume) => selected.includes(volume.id) && !volume.error);
        const pages = volumes.reduce((count, volume) => count + volume.pageCount, 0);
        showConfirm(
            reprocess ? 'Reprocess OCR?' : 'Prepare OCR?',
            `${volumes.length} volume${volumes.length === 1 ? '' : 's'} · ${pages} pages`,
            () => {
                ForkApi.prepareVolumeOcr(
                    volumes.map((volume) => volume.id),
                    reprocess,
                    { ocrProvider: settings.ocrProvider, selfHostedUrl: settings.selfHostedOcrUrl },
                )
                    .then(async (job) => {
                        setStatus(`Queued ${job.totalPages ?? 0} pages. Progress is shown in Activity.`);
                        setSelected([]);
                        await load();
                    })
                    .catch((error) => setStatus(messageOf(error)));
            },
        );
    };
    const download = (prepareOcrAfter: boolean) => {
        if (!series) return;
        const volumes = series.volumes.filter((volume) => selected.includes(volume.id) && !volume.error);
        const pages = volumes.reduce((count, volume) => count + volume.pageCount, 0);
        showConfirm(
            prepareOcrAfter ? 'Make available offline and prepare OCR?' : 'Make available offline?',
            `${volumes.length} volume${volumes.length === 1 ? '' : 's'} · ${pages} pages`,
            () => {
                ForkApi.downloadVolumes(
                    volumes.map((volume) => volume.id),
                    prepareOcrAfter,
                    false,
                    { ocrProvider: settings.ocrProvider, selfHostedUrl: settings.selfHostedOcrUrl },
                )
                    .then(async (job) => {
                        setStatus(`Queued ${job.totalPages ?? 0} pages. Progress is shown in Activity.`);
                        setSelected([]);
                        await load();
                    })
                    .catch((error) => setStatus(messageOf(error)));
            },
        );
    };
    const remote = Boolean(series && state?.sources.find((source) => source.id === series.sourceId)?.kind !== 'local');
    const seriesProgress = useMemo(() => {
        const readable = series?.volumes.filter((volume) => volume.pageCount > 0 && !volume.error) ?? [];
        const read = readable.reduce((sum, volume) => sum + Math.min(volume.progressPage, volume.pageCount - 1), 0);
        const total = readable.reduce((sum, volume) => sum + volume.pageCount - 1, 0);
        return (read / Math.max(1, total)) * 100;
    }, [series]);
    return (
        <Stack spacing={2} sx={{ p: { xs: 1.5, sm: 3 }, pb: { xs: 11, sm: 10 }, maxWidth: 1000, mx: 'auto' }}>
            <Stack direction="row" spacing={1} alignItems="center">
                <Box sx={{ flex: 1, minWidth: 0 }}>
                    {series && <ForkProgress value={seriesProgress} label="Series reading progress" />}
                </Box>
                <IconButton
                    color={selecting ? 'primary' : 'default'}
                    aria-label={selecting ? 'Exit selection mode' : 'Select volumes'}
                    onClick={() => (selecting ? leaveSelection() : setSelecting(true))}
                >
                    <ChecklistIcon />
                </IconButton>
            </Stack>
            {selecting && selected.length > 0 && (
                <ForkSelectionBar count={selected.length} onClose={leaveSelection}>
                    {/* prepare/download open a confirmation and handle their own errors (they return void). */}
                    <Button variant="contained" onClick={() => prepare(false)}>
                        OCR
                    </Button>
                    <Button color="inherit" onClick={() => prepare(true)}>
                        Reprocess
                    </Button>
                    {remote && (
                        <Button color="inherit" onClick={() => download(false)}>
                            Offline
                        </Button>
                    )}
                    {remote && (
                        <Button color="inherit" onClick={() => download(true)}>
                            Offline + prepare OCR
                        </Button>
                    )}
                    <Button color="inherit" onClick={() => run(() => applySensitive(true))}>
                        Mark NSFW
                    </Button>
                    <Button color="inherit" onClick={() => run(() => applySensitive(false))}>
                        Remove NSFW
                    </Button>
                    <Button color="inherit" onClick={() => run(() => setReadState(selected, true, true))}>
                        Mark read
                    </Button>
                    <Button color="inherit" onClick={() => run(() => setReadState(selected, false, true))}>
                        Mark unread
                    </Button>
                </ForkSelectionBar>
            )}
            {status && (
                <Alert severity="info" onClose={() => setStatus(undefined)}>
                    {status}
                </Alert>
            )}
            <Box
                sx={{
                    display: 'grid',
                    gridTemplateColumns: 'repeat(2, minmax(0, 1fr))',
                    columnGap: { xs: 1.5, sm: 2 },
                    rowGap: 2.5,
                }}
            >
                {series?.volumes.map((volume) => {
                    const isSelected = selected.includes(volume.id);
                    const isReadable = !volume.error && volume.pageCount > 0;
                    const isRead = isReadable && volume.progressPage >= Math.max(0, volume.pageCount - 1);
                    return (
                        <Card
                            key={volume.id}
                            elevation={0}
                            sx={{ bgcolor: 'transparent', overflow: 'visible', borderRadius: 0 }}
                        >
                            <Box sx={{ position: 'relative' }}>
                                <CardActionArea
                                    disabled={!isReadable}
                                    onClick={() =>
                                        selecting
                                            ? toggle(volume.id)
                                            : navigate(`/fork/reader/${encodeURIComponent(volume.id)}`, {
                                                  state: { resumeMode: ReaderResumeMode.LAST_READ },
                                              })
                                    }
                                    onContextMenu={(event) => {
                                        event.preventDefault();
                                        setSelecting(true);
                                        toggle(volume.id);
                                    }}
                                >
                                    {isReadable ? (
                                        <Box
                                            component="img"
                                            src={`/api/fork/volumes/${encodeURIComponent(volume.id)}/thumbnail?fingerprint=${encodeURIComponent(volume.fingerprint)}`}
                                            alt=""
                                            loading="lazy"
                                            decoding="async"
                                            sx={{
                                                display: 'block',
                                                width: '100%',
                                                aspectRatio: '2 / 3',
                                                objectFit: 'cover',
                                                bgcolor: 'action.hover',
                                                border: isSelected
                                                    ? '2px solid #fff'
                                                    : '1px solid rgba(255,255,255,0.28)',
                                                borderRadius: 1.5,
                                            }}
                                        />
                                    ) : (
                                        <Box sx={{ width: '100%', aspectRatio: '2 / 3', bgcolor: 'action.hover' }} />
                                    )}
                                </CardActionArea>
                                {selecting && (
                                    <Checkbox
                                        checked={isSelected}
                                        onChange={() => toggle(volume.id)}
                                        inputProps={{ 'aria-label': `Select ${volume.title}` }}
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
                                value={(volume.progressPage / Math.max(1, volume.pageCount - 1)) * 100}
                                label={`${volume.title} reading progress`}
                            />
                            <Stack direction="row" alignItems="flex-start" spacing={0.25}>
                                <Box sx={{ flex: 1, minWidth: 0 }}>
                                    <Typography
                                        title={volume.title}
                                        sx={{ fontSize: '0.88rem', lineHeight: 1.25, mt: 0.2 }}
                                        noWrap
                                    >
                                        {volume.title}
                                    </Typography>
                                    <Stack direction="row" spacing={0.5} alignItems="center">
                                        <Typography
                                            variant="caption"
                                            color={volume.error ? 'warning.main' : 'text.secondary'}
                                            noWrap
                                            display="block"
                                            sx={{ flex: 1, minWidth: 0 }}
                                        >
                                            {volume.error ||
                                                `${isRead ? 'Read' : `Page ${Math.min(volume.progressPage + 1, volume.pageCount)}`} · ${volume.pageCount} pages${remote && volume.downloadState === 'downloaded' ? ' · offline' : ''}`}
                                        </Typography>
                                        {!volume.error && <VolumeOcrIndicator state={volume.ocrState} />}
                                    </Stack>
                                </Box>
                                {isReadable && !selecting && (
                                    <IconButton
                                        size="small"
                                        aria-label={`Mark ${volume.title} ${isRead ? 'unread' : 'read'}`}
                                        onClick={() => run(() => setReadState([volume.id], !isRead))}
                                    >
                                        {isRead ? <ReplayIcon fontSize="small" /> : <DoneAllIcon fontSize="small" />}
                                    </IconButton>
                                )}
                            </Stack>
                        </Card>
                    );
                })}
            </Box>
        </Stack>
    );
}
