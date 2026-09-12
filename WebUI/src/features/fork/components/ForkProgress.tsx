import Box from '@mui/material/Box';
import Typography from '@mui/material/Typography';

export const ForkProgress = ({ value, label }: { value: number; label: string }) => {
    const percent = Math.max(0, Math.min(100, value));
    return (
        <Box sx={{ display: 'flex', alignItems: 'center', gap: 0.75, mt: 0.75 }}>
            <Box
                role="progressbar"
                aria-label={label}
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={Math.round(percent)}
                sx={{ height: 3, bgcolor: 'rgba(255,255,255,0.28)', flex: 1, borderRadius: 2, overflow: 'hidden' }}
            >
                <Box sx={{ height: '100%', width: `${percent}%`, bgcolor: '#fff', borderRadius: 2 }} />
            </Box>
            <Typography variant="caption" sx={{ color: 'rgba(255,255,255,0.82)', minWidth: 34, textAlign: 'right' }}>
                {percent.toFixed(percent < 10 && percent > 0 ? 1 : 0)}%
            </Typography>
        </Box>
    );
};
