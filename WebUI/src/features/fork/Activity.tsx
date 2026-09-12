import Alert from '@mui/material/Alert';
import Box from '@mui/material/Box';
import Button from '@mui/material/Button';
import Card from '@mui/material/Card';
import CardContent from '@mui/material/CardContent';
import LinearProgress from '@mui/material/LinearProgress';
import Stack from '@mui/material/Stack';
import Typography from '@mui/material/Typography';
import { useAppTitle } from '@/features/navigation-bar/hooks/useAppTitle';
import { ForkApi } from '@/features/fork/ForkApi';
import { useForkState } from '@/features/fork/useForkState';

export const ActivityContent = ({ embedded = false }: { embedded?: boolean }) => {
    const { state, error, refresh } = useForkState(1500);
    const jobs = (state?.jobs ?? []).filter((job) => !['complete', 'cancelled'].includes(job.state));

    return (
        <Stack spacing={2} sx={embedded ? undefined : { p: { xs: 2, sm: 3 }, maxWidth: 900, mx: 'auto' }}>
            {error && <Alert severity="error">Activity storage is unavailable: {error}</Alert>}
            {!error && jobs.length === 0 && (
                <Box sx={{ py: 8, textAlign: 'center' }}>
                    <Typography variant="h6">No active OCR or offline jobs</Typography>
                    <Typography color="text.secondary">
                        Select manga in Library and choose Prepare OCR or Make available offline.
                    </Typography>
                </Box>
            )}
            {jobs.map((job) => {
                const total = job.totalPages ?? 0;
                const value = total ? (job.completedPages / total) * 100 : 0;
                const title = !state?.showSensitive && job.sensitive ? 'Hidden manga' : job.title;
                return (
                    <Card key={job.id} data-testid={`activity-job-${job.id}`}>
                        <CardContent>
                            <Stack spacing={1.5}>
                                <Stack direction="row" justifyContent="space-between" alignItems="center">
                                    <Typography variant="subtitle1">{title}</Typography>
                                    <Typography variant="body2" color="text.secondary">
                                        {job.state.replace('_', ' ')}
                                    </Typography>
                                </Stack>
                                <LinearProgress variant={total ? 'determinate' : 'indeterminate'} value={value} />
                                <Typography variant="body2">
                                    {total ? `${job.completedPages} / ${total} pages` : `${job.completedPages} pages`}
                                </Typography>
                                {job.error && <Alert severity="warning">{job.error}</Alert>}
                                <Stack direction="row" spacing={1} flexWrap="wrap">
                                    {job.state === 'processing' || job.state === 'queued' ? (
                                        <Button onClick={() => ForkApi.controlJob(job.id, 'pause').then(refresh)}>
                                            Pause
                                        </Button>
                                    ) : null}
                                    {job.state === 'paused' ? (
                                        <Button onClick={() => ForkApi.controlJob(job.id, 'resume').then(refresh)}>
                                            Resume
                                        </Button>
                                    ) : null}
                                    {job.state === 'failed' || job.state === 'partial' ? (
                                        <Button onClick={() => ForkApi.controlJob(job.id, 'retry').then(refresh)}>
                                            Retry missing
                                        </Button>
                                    ) : null}
                                    {!['complete', 'cancelled'].includes(job.state) ? (
                                        <Button
                                            color="error"
                                            onClick={() => ForkApi.controlJob(job.id, 'cancel').then(refresh)}
                                        >
                                            Cancel
                                        </Button>
                                    ) : null}
                                </Stack>
                            </Stack>
                        </CardContent>
                    </Card>
                );
            })}
        </Stack>
    );
};

export const Activity = () => {
    useAppTitle('Activity');
    return <ActivityContent />;
};
