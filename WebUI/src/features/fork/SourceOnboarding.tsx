import Button from '@mui/material/Button';
import Dialog from '@mui/material/Dialog';
import DialogContent from '@mui/material/DialogContent';
import DialogTitle from '@mui/material/DialogTitle';
import Stack from '@mui/material/Stack';
import Typography from '@mui/material/Typography';
import { useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { AppRoutes } from '@/base/AppRoute.constants';

const COMPLETE_KEY = 'manga_reader_source_onboarding_v1';

export function SourceOnboarding() {
    const navigate = useNavigate();
    const [open, setOpen] = useState(() => window.localStorage.getItem(COMPLETE_KEY) !== 'complete');
    const choose = (kind: 'local' | 'smb' | 'komga') => {
        window.localStorage.setItem(COMPLETE_KEY, 'complete');
        setOpen(false);
        navigate(`${AppRoutes.settings.path()}?addSource=${kind}`);
    };
    const later = () => {
        window.localStorage.setItem(COMPLETE_KEY, 'complete');
        setOpen(false);
    };
    return (
        <Dialog open={open} fullWidth maxWidth="sm" aria-labelledby="source-onboarding-title">
            <DialogTitle id="source-onboarding-title">Where is your manga?</DialogTitle>
            <DialogContent>
                <Stack spacing={1.5}>
                    <Typography color="text.secondary">
                        Add a source now, or start with an existing inherited library and configure one later.
                    </Typography>
                    <Button variant="outlined" size="large" onClick={() => choose('local')}>
                        On this phone — choose a folder or archives
                    </Button>
                    <Button variant="outlined" size="large" onClick={() => choose('smb')}>
                        Remote folder — connect to SMB or WebDAV
                    </Button>
                    <Button variant="outlined" size="large" onClick={() => choose('komga')}>
                        Komga — connect and choose server libraries
                    </Button>
                    <Button onClick={later}>Not now</Button>
                </Stack>
            </DialogContent>
        </Dialog>
    );
}
