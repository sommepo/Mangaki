import DeleteOutlineIcon from '@mui/icons-material/DeleteOutline';
import ChecklistIcon from '@mui/icons-material/Checklist';
import Alert from '@mui/material/Alert';
import Box from '@mui/material/Box';
import Card from '@mui/material/Card';
import CardContent from '@mui/material/CardContent';
import Button from '@mui/material/Button';
import Checkbox from '@mui/material/Checkbox';
import IconButton from '@mui/material/IconButton';
import Stack from '@mui/material/Stack';
import Tab from '@mui/material/Tab';
import Tabs from '@mui/material/Tabs';
import Typography from '@mui/material/Typography';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useAppTitle } from '@/features/navigation-bar/hooks/useAppTitle';
import { ForkApi, ForkReadingSession } from '@/features/fork/ForkApi';
import { useForkState } from '@/features/fork/useForkState';
import { ForkSelectionBar } from '@/features/fork/components/ForkSelectionBar';
import { useOCR } from '@/Manatan/context/OCRContext';

export const Stats = () => {
    useAppTitle('Stats');
    const [range, setRange] = useState<'day' | 'week' | 'month' | 'all'>('week');
    const [sessions, setSessions] = useState<ForkReadingSession[]>([]);
    const [selected, setSelected] = useState<string[]>([]);
    const [selecting, setSelecting] = useState(false);
    const [removing, setRemoving] = useState(false);
    const [actionMessage, setActionMessage] = useState<string>();
    const { showConfirm } = useOCR();
    const { state, error, refresh } = useForkState();
    const stats = state?.stats[range];
    const load = useCallback(
        () =>
            ForkApi.readingSessions(range)
                .then((result) => {
                    setSessions(result.sessions);
                    const available = new Set(result.sessions.map((session) => session.sessionId));
                    setSelected((current) => current.filter((id) => available.has(id)));
                })
                .catch(() => setSessions([])),
        [range],
    );
    useEffect(() => {
        load();
    }, [load, state]);
    const grouped = useMemo(
        () =>
            sessions.reduce<Record<string, ForkReadingSession[]>>(
                (result, session) => ({
                    ...result,
                    [session.seriesId]: [...(result[session.seriesId] ?? []), session],
                }),
                {},
            ),
        [sessions],
    );
    const toggle = (sessionId: string) => {
        setSelected((current) =>
            current.includes(sessionId)
                ? current.filter((candidate) => candidate !== sessionId)
                : [...current, sessionId],
        );
    };
    const selectSessions = (ids: string[]) => {
        setSelecting(true);
        setSelected((current) => [...new Set([...current, ...ids])]);
    };
    const performRemoveSessions = async (ids: string[]) => {
        setRemoving(true);
        setActionMessage(undefined);
        try {
            const results = await Promise.allSettled(ids.map((id) => ForkApi.removeReadingSession(id)));
            const failed = results.filter((result) => result.status === 'rejected').length;
            setSelected([]);
            await Promise.allSettled([load(), refresh()]);
            setActionMessage(
                failed === 0
                    ? `Removed ${ids.length} session${ids.length === 1 ? '' : 's'}.`
                    : `Removed ${ids.length - failed}; ${failed} could not be removed.`,
            );
        } finally {
            setRemoving(false);
        }
    };
    const removeSessions = (ids: string[]) => {
        if (ids.length === 0) return;
        showConfirm(
            'Remove reading sessions?',
            `${ids.length} session${ids.length === 1 ? '' : 's'} will be removed from Stats.`,
            () => {
                performRemoveSessions(ids).catch((cause) =>
                    setActionMessage(cause instanceof Error ? cause.message : String(cause)),
                );
            },
        );
    };
    return (
        <Stack spacing={2} sx={{ p: { xs: 1.5, sm: 3 }, pb: { xs: 11, sm: 10 }, maxWidth: 1000, mx: 'auto' }}>
            <Tabs
                value={range}
                onChange={(_, value) => setRange(value)}
                variant="fullWidth"
                aria-label="Statistics range"
                sx={{
                    minHeight: 40,
                    border: '1px solid rgba(255,255,255,0.72)',
                    borderRadius: 2.5,
                    p: 0.35,
                    '& .MuiTab-root': { minHeight: 34, py: 0, px: 0.5, fontSize: '0.78rem' },
                    '& .Mui-selected': { bgcolor: '#fff', color: '#000 !important', borderRadius: 2 },
                    '& .MuiTabs-indicator': { display: 'none' },
                }}
            >
                <Tab value="day" label="Day" />
                <Tab value="week" label="Week" />
                <Tab value="month" label="Month" />
                <Tab value="all" label="All time" />
            </Tabs>
            {error && <Alert severity="error">Statistics are unavailable: {error}</Alert>}
            {stats && (
                <Card
                    variant="outlined"
                    sx={{ bgcolor: 'transparent', borderColor: 'rgba(255,255,255,0.72)', borderRadius: 3 }}
                >
                    <CardContent sx={{ py: 2, '&:last-child': { pb: 2 } }}>
                        <Stack direction="row" justifyContent="space-around" textAlign="center">
                            <Box>
                                <Typography variant="h5" fontWeight={700}>
                                    {Math.floor(stats.activeReadingSeconds / 60)}m
                                </Typography>
                                <Typography variant="caption" color="text.secondary">
                                    Reading
                                </Typography>
                            </Box>
                            <Box>
                                <Typography variant="h5" fontWeight={700}>
                                    {stats.uniquePages}
                                </Typography>
                                <Typography variant="caption" color="text.secondary">
                                    Pages
                                </Typography>
                            </Box>
                        </Stack>
                    </CardContent>
                </Card>
            )}
            <Stack direction="row" alignItems="center" justifyContent="space-between">
                <Typography variant="h6" fontWeight={700}>
                    Sessions
                </Typography>
                {sessions.length > 0 && (
                    <Button
                        size="small"
                        startIcon={<ChecklistIcon />}
                        onClick={() => (selecting ? (setSelecting(false), setSelected([])) : setSelecting(true))}
                    >
                        {selecting ? 'Done' : 'Manage'}
                    </Button>
                )}
            </Stack>
            {selecting && selected.length > 0 && (
                <ForkSelectionBar
                    count={selected.length}
                    onClose={() => {
                        setSelected([]);
                        setSelecting(false);
                    }}
                >
                    <Button
                        color="inherit"
                        onClick={() => selectSessions(sessions.map((session) => session.sessionId))}
                    >
                        Select all
                    </Button>
                    <Button
                        color="error"
                        variant="contained"
                        disabled={removing}
                        onClick={() => removeSessions(selected)}
                    >
                        Remove
                    </Button>
                </ForkSelectionBar>
            )}
            {actionMessage && <Alert severity="info">{actionMessage}</Alert>}
            {!error && sessions.length === 0 && (
                <Typography color="text.secondary">No reading sessions yet.</Typography>
            )}
            {Object.entries(grouped).map(([seriesId, mangaSessions]) => (
                <Card
                    key={seriesId}
                    variant="outlined"
                    sx={{ bgcolor: 'transparent', borderColor: 'rgba(255,255,255,0.55)', borderRadius: 3 }}
                >
                    <CardContent sx={{ p: 1.75, '&:last-child': { pb: 1.75 } }}>
                        <Stack direction="row" alignItems="center" justifyContent="space-between" spacing={1}>
                            <Typography variant="subtitle1" fontWeight={600} gutterBottom>
                                {mangaSessions[0].title}
                            </Typography>
                            {selecting && (
                                <Button
                                    size="small"
                                    onClick={() => selectSessions(mangaSessions.map((session) => session.sessionId))}
                                >
                                    Select manga
                                </Button>
                            )}
                        </Stack>
                        <Stack spacing={1}>
                            {mangaSessions.map((session) => (
                                <Stack
                                    key={session.sessionId}
                                    direction="row"
                                    alignItems="center"
                                    justifyContent="space-between"
                                    spacing={1}
                                >
                                    <Stack direction="row" alignItems="center" spacing={0.5} sx={{ minWidth: 0 }}>
                                        {selecting && (
                                            <Checkbox
                                                size="small"
                                                checked={selected.includes(session.sessionId)}
                                                onChange={() => toggle(session.sessionId)}
                                                inputProps={{
                                                    'aria-label': `Select session from ${new Date(session.startedAtMs).toLocaleString()}`,
                                                }}
                                            />
                                        )}
                                        <Typography variant="body2">
                                            {new Date(session.startedAtMs).toLocaleString([], {
                                                dateStyle: 'medium',
                                                timeStyle: 'short',
                                            })}
                                            {' · '}
                                            {Math.max(1, Math.round(session.activeSeconds / 60))} min · {session.pages}{' '}
                                            pages
                                        </Typography>
                                    </Stack>
                                    <IconButton
                                        size="small"
                                        color="error"
                                        aria-label="Remove reading session"
                                        disabled={removing}
                                        onClick={() => removeSessions([session.sessionId])}
                                    >
                                        <DeleteOutlineIcon fontSize="small" />
                                    </IconButton>
                                </Stack>
                            ))}
                        </Stack>
                    </CardContent>
                </Card>
            ))}
            <Alert severity="info">A new session starts after 15 minutes away from a manga.</Alert>
        </Stack>
    );
};
