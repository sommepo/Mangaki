import RefreshIcon from '@mui/icons-material/Refresh';
import Alert from '@mui/material/Alert';
import Button from '@mui/material/Button';
import FormControlLabel from '@mui/material/FormControlLabel';
import MenuItem from '@mui/material/MenuItem';
import Stack from '@mui/material/Stack';
import Switch from '@mui/material/Switch';
import TextField from '@mui/material/TextField';
import Typography from '@mui/material/Typography';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useOCR } from '@/Manatan/context/OCRContext';
import { getDeckNames, getModelFields, getModelNames, getAnkiVersion } from '@/Manatan/utils/anki';
import { getDictionaries } from '@/Manatan/utils/api';

const BASE_MAPPINGS = [
    'None',
    'Target Word',
    'Reading',
    'Furigana',
    'Glossary',
    'Sentence',
    'Sentence Furigana',
    'Image',
    'Word Audio',
    'Sentence Audio',
    'Frequency',
    'Harmonic Frequency',
    'Pitch Accent',
    'Pitch Accent Positions',
    'Pitch Accent Categories',
];

export const ForkAnkiSettings = () => {
    const { settings, setSettings } = useOCR();
    const [connection, setConnection] = useState<'idle' | 'loading' | 'connected' | 'error'>('idle');
    const [decks, setDecks] = useState<string[]>([]);
    const [models, setModels] = useState<string[]>([]);
    const [fields, setFields] = useState<string[]>([]);
    const [dictionaryNames, setDictionaryNames] = useState<string[]>([]);

    const connect = useCallback(async () => {
        if (!settings.ankiConnectEnabled) return;
        setConnection('loading');
        const url = settings.ankiConnectUrl || 'http://127.0.0.1:18765';
        try {
            const result = await getAnkiVersion(url);
            if (!result.ok) throw new Error('AnkiConnect did not respond');
            const [nextDecks, nextModels] = await Promise.all([getDeckNames(url), getModelNames(url)]);
            setDecks(nextDecks);
            setModels(nextModels);
            setConnection('connected');
        } catch {
            setConnection('error');
        }
    }, [settings.ankiConnectEnabled, settings.ankiConnectUrl]);

    useEffect(() => {
        if (settings.ankiConnectEnabled) connect();
        else setConnection('idle');
    }, [connect, settings.ankiConnectEnabled]);

    useEffect(() => {
        if (connection !== 'connected' || !settings.ankiModel) {
            setFields([]);
            return;
        }
        getModelFields(settings.ankiConnectUrl, settings.ankiModel)
            .then(setFields)
            .catch(() => setFields([]));
    }, [connection, settings.ankiConnectUrl, settings.ankiModel]);

    useEffect(() => {
        getDictionaries().then((items) => setDictionaryNames(items?.map((item) => item.name) ?? []));
    }, []);

    const mappings = useMemo(
        () => [...BASE_MAPPINGS, ...dictionaryNames.map((name) => `Single Glossary ${name}`)],
        [dictionaryNames],
    );

    const enableOnDevice = () => {
        const next = { ...settings, ankiConnectEnabled: true, ankiConnectUrl: 'http://127.0.0.1:18765' };
        setSettings(next);
        (window as any).MangaReaderNative?.requestAnkiPermission?.();
    };

    const connectionLabel = {
        idle: 'Not checked',
        loading: 'Connecting…',
        connected: 'Connected',
        error: 'Not connected',
    }[connection];

    const updateMapping = (field: string, value: string) => {
        const nextMap = { ...(settings.ankiFieldMap ?? {}), [field]: value };
        if (value === 'Target Word') {
            Object.keys(nextMap).forEach((candidate) => {
                if (candidate !== field && nextMap[candidate] === 'Target Word') nextMap[candidate] = 'None';
            });
        }
        setSettings({ ...settings, ankiFieldMap: nextMap });
    };

    return (
        <Stack spacing={1.5}>
            <Stack direction="row" alignItems="center" justifyContent="space-between" spacing={1}>
                <FormControlLabel
                    sx={{ m: 0 }}
                    control={
                        <Switch
                            checked={settings.ankiConnectEnabled}
                            onChange={(_, checked) => setSettings({ ...settings, ankiConnectEnabled: checked })}
                        />
                    }
                    label="Enable AnkiConnect"
                />
                <Button size="small" variant="outlined" onClick={enableOnDevice}>
                    Use AnkiDroid
                </Button>
            </Stack>

            {settings.ankiConnectEnabled && (
                <>
                    <TextField
                        size="small"
                        label="AnkiConnect address"
                        value={settings.ankiConnectUrl}
                        onChange={(event) => setSettings({ ...settings, ankiConnectUrl: event.target.value })}
                    />
                    <Stack direction="row" alignItems="center" justifyContent="space-between">
                        <Typography
                            variant="body2"
                            color={connection === 'connected' ? 'success.main' : 'text.secondary'}
                        >
                            {connectionLabel}
                        </Typography>
                        <Button
                            size="small"
                            startIcon={<RefreshIcon />}
                            disabled={connection === 'loading'}
                            onClick={connect}
                        >
                            Retry
                        </Button>
                    </Stack>
                    {connection === 'error' && (
                        <Alert severity="info">Open AnkiDroid and allow access, then tap Retry.</Alert>
                    )}
                    <TextField
                        select
                        size="small"
                        label="Deck"
                        value={settings.ankiDeck ?? ''}
                        disabled={connection !== 'connected'}
                        onChange={(event) => setSettings({ ...settings, ankiDeck: event.target.value })}
                    >
                        {decks.map((deck) => (
                            <MenuItem key={deck} value={deck}>
                                {deck}
                            </MenuItem>
                        ))}
                    </TextField>
                    <TextField
                        select
                        size="small"
                        label="Note type"
                        value={settings.ankiModel ?? ''}
                        disabled={connection !== 'connected'}
                        onChange={(event) =>
                            setSettings({ ...settings, ankiModel: event.target.value, ankiFieldMap: {} })
                        }
                    >
                        {models.map((model) => (
                            <MenuItem key={model} value={model}>
                                {model}
                            </MenuItem>
                        ))}
                    </TextField>
                    {fields.length > 0 && (
                        <Stack spacing={1}>
                            <Typography variant="subtitle2">Field mappings</Typography>
                            {fields.map((field) => (
                                <TextField
                                    key={field}
                                    select
                                    size="small"
                                    label={field}
                                    value={settings.ankiFieldMap?.[field] ?? 'None'}
                                    onChange={(event) => updateMapping(field, event.target.value)}
                                >
                                    {mappings.map((mapping) => (
                                        <MenuItem key={mapping} value={mapping}>
                                            {mapping}
                                        </MenuItem>
                                    ))}
                                </TextField>
                            ))}
                        </Stack>
                    )}
                </>
            )}
        </Stack>
    );
};
