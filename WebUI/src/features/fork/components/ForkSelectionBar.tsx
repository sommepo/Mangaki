import CloseIcon from '@mui/icons-material/Close';
import Box from '@mui/material/Box';
import IconButton from '@mui/material/IconButton';
import Paper from '@mui/material/Paper';
import Stack from '@mui/material/Stack';
import Typography from '@mui/material/Typography';
import { ReactNode } from 'react';

export const ForkSelectionBar = ({
    count,
    onClose,
    children,
}: {
    count: number;
    onClose: () => void;
    children: ReactNode;
}) => (
    <Paper
        role="toolbar"
        aria-label={`${count} selected`}
        elevation={10}
        sx={{
            position: 'fixed',
            zIndex: (theme) => theme.zIndex.drawer,
            left: { xs: 10, sm: '50%' },
            right: { xs: 10, sm: 'auto' },
            bottom: { xs: 'calc(68px + env(safe-area-inset-bottom))', sm: 20 },
            width: { sm: 'min(760px, calc(100vw - 40px))' },
            transform: { sm: 'translateX(-50%)' },
            bgcolor: '#050505',
            color: '#fff',
            border: '1px solid rgba(255,255,255,0.72)',
            borderRadius: 3,
            overflow: 'hidden',
        }}
    >
        <Stack direction="row" alignItems="center" sx={{ minHeight: 48, pl: 1.5, pr: 0.5 }}>
            <Typography fontWeight={700} sx={{ whiteSpace: 'nowrap', mr: 1 }}>
                {count} selected
            </Typography>
            <Box
                sx={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 0.5,
                    overflowX: 'auto',
                    flex: 1,
                    py: 0.5,
                    minWidth: 0,
                    scrollbarWidth: 'none',
                    '&::-webkit-scrollbar': { display: 'none' },
                    '& > *': { flex: '0 0 auto' },
                    '& .MuiButton-root': {
                        whiteSpace: 'nowrap',
                        minHeight: 36,
                        minWidth: 'max-content',
                        px: 1.25,
                        fontSize: '0.78rem',
                    },
                }}
            >
                {children}
            </Box>
            <IconButton color="inherit" aria-label="Exit selection mode" onClick={onClose}>
                <CloseIcon />
            </IconButton>
        </Stack>
    </Paper>
);
