import DeleteOutlineIcon from '@mui/icons-material/DeleteOutline';
import FileUploadOutlinedIcon from '@mui/icons-material/FileUploadOutlined';
import Alert from '@mui/material/Alert';
import Button from '@mui/material/Button';
import CircularProgress from '@mui/material/CircularProgress';
import FormControlLabel from '@mui/material/FormControlLabel';
import IconButton from '@mui/material/IconButton';
import MenuItem from '@mui/material/MenuItem';
import Stack from '@mui/material/Stack';
import Switch from '@mui/material/Switch';
import TextField from '@mui/material/TextField';
import Typography from '@mui/material/Typography';
import { ChangeEvent, useCallback, useEffect, useRef, useState } from 'react';
import { useOCR } from '@/Manatan/context/OCRContext';
import { DictionaryMeta, getDictionaries, manageDictionary } from '@/Manatan/utils/api';

export const ForkDictionarySettings = () => {
    const { settings, setSettings, showConfirm } = useOCR();
    const inputRef = useRef<HTMLInputElement>(null);
    const [dictionaries, setDictionaries] = useState<DictionaryMeta[]>([]);
    const [loading, setLoading] = useState(true);
    const [status, setStatus] = useState<string>();

    const refresh = useCallback(async () => {
        setLoading(true);
        const result = await getDictionaries();
        if (result) setDictionaries(result);
        setLoading(false);
    }, []);

    useEffect(() => {
        refresh();
    }, [refresh]);

    const importDictionaries = async (event: ChangeEvent<HTMLInputElement>) => {
        const files = Array.from(event.target.files ?? []);
        if (!files.length) return;
        setLoading(true);
        const results = await Promise.all(
            files.map(async (file) => {
                const body = new FormData();
                body.append('file', file);
                try {
                    const response = await fetch('/api/yomitan/import', { method: 'POST', body });
                    const result = await response.json();
                    if (!response.ok || result.status !== 'ok')
                        throw new Error(result.message || `HTTP ${response.status}`);
                    return undefined;
                } catch (cause) {
                    return `${file.name}: ${cause instanceof Error ? cause.message : String(cause)}`;
                }
            }),
        );
        const failures = results.filter((result): result is string => Boolean(result));
        const imported = files.length - failures.length;
        if (inputRef.current) inputRef.current.value = '';
        await refresh();
        setStatus(
            failures.length
                ? `Imported ${imported} of ${files.length}. ${failures.join(' ')}`
                : `Installed ${imported} dictionar${imported === 1 ? 'y' : 'ies'}.`,
        );
    };

    return (
        <Stack spacing={2}>
            <TextField
                select
                size="small"
                label="Popup theme"
                value={settings.yomitanPopupTheme}
                onChange={(event) =>
                    setSettings({ ...settings, yomitanPopupTheme: event.target.value as 'dark' | 'light' })
                }
            >
                <MenuItem value="dark">Dark</MenuItem>
                <MenuItem value="light">Light</MenuItem>
            </TextField>

            <Stack direction="row" alignItems="center" justifyContent="space-between" spacing={1}>
                <Stack spacing={0.25} sx={{ minWidth: 0 }}>
                    <Typography fontWeight={600}>Term and frequency dictionaries</Typography>
                    <Typography variant="caption" color="text.secondary">
                        Install Yomitan ZIP dictionaries. Their index identifies whether they contain terms or frequency
                        data.
                    </Typography>
                </Stack>
                <Button
                    size="small"
                    variant="outlined"
                    startIcon={<FileUploadOutlinedIcon />}
                    onClick={() => inputRef.current?.click()}
                    sx={{ flexShrink: 0 }}
                >
                    Install
                </Button>
                <input
                    ref={inputRef}
                    hidden
                    type="file"
                    accept=".zip,application/zip"
                    multiple
                    onChange={importDictionaries}
                />
            </Stack>

            {loading && <CircularProgress size={22} aria-label="Loading dictionaries" />}
            {!loading && dictionaries.length === 0 && (
                <Typography variant="body2" color="text.secondary">
                    No dictionaries installed.
                </Typography>
            )}
            {!loading && dictionaries.length > 0 && (
                <Stack divider={<span style={{ borderTop: '1px solid rgba(255,255,255,.18)' }} />}>
                    {dictionaries.map((dictionary) => (
                        <Stack key={dictionary.id} direction="row" alignItems="center" spacing={1}>
                            <FormControlLabel
                                sx={{ flex: 1, minWidth: 0, m: 0 }}
                                control={
                                    <Switch
                                        size="small"
                                        checked={dictionary.enabled}
                                        onChange={async () => {
                                            await manageDictionary('Toggle', {
                                                id: dictionary.id,
                                                enabled: !dictionary.enabled,
                                            });
                                            await refresh();
                                        }}
                                    />
                                }
                                label={
                                    <Typography variant="body2" noWrap>
                                        {dictionary.name}
                                    </Typography>
                                }
                            />
                            <IconButton
                                size="small"
                                color="error"
                                aria-label={`Delete ${dictionary.name}`}
                                onClick={() => {
                                    showConfirm('Delete dictionary?', dictionary.name, () => {
                                        manageDictionary('Delete', { id: dictionary.id })
                                            .then(refresh)
                                            .catch((error) =>
                                                setStatus(error instanceof Error ? error.message : String(error)),
                                            );
                                    });
                                }}
                            >
                                <DeleteOutlineIcon fontSize="small" />
                            </IconButton>
                        </Stack>
                    ))}
                </Stack>
            )}
            {status && (
                <Alert severity="info" onClose={() => setStatus(undefined)}>
                    {status}
                </Alert>
            )}
        </Stack>
    );
};
