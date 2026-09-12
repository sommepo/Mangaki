/*
 * Copyright (C) Contributors to the Suwayomi project
 *
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

import AddIcon from '@mui/icons-material/Add';
import BackupOutlinedIcon from '@mui/icons-material/BackupOutlined';
import BarChartOutlinedIcon from '@mui/icons-material/BarChartOutlined';
import FolderOpenOutlinedIcon from '@mui/icons-material/FolderOpenOutlined';
import HideImageOutlinedIcon from '@mui/icons-material/HideImageOutlined';
import ImageSearchOutlinedIcon from '@mui/icons-material/ImageSearchOutlined';
import Inventory2OutlinedIcon from '@mui/icons-material/Inventory2Outlined';
import ManageSearchOutlinedIcon from '@mui/icons-material/ManageSearchOutlined';
import SyncOutlinedIcon from '@mui/icons-material/SyncOutlined';
import Alert from '@mui/material/Alert';
import Accordion from '@mui/material/Accordion';
import AccordionDetails from '@mui/material/AccordionDetails';
import AccordionSummary from '@mui/material/AccordionSummary';
import Button from '@mui/material/Button';
import Box from '@mui/material/Box';
import Chip from '@mui/material/Chip';
import Checkbox from '@mui/material/Checkbox';
import Dialog from '@mui/material/Dialog';
import DialogActions from '@mui/material/DialogActions';
import DialogContent from '@mui/material/DialogContent';
import DialogTitle from '@mui/material/DialogTitle';
import FormControlLabel from '@mui/material/FormControlLabel';
import MenuItem from '@mui/material/MenuItem';
import Stack from '@mui/material/Stack';
import Switch from '@mui/material/Switch';
import TextField from '@mui/material/TextField';
import Typography from '@mui/material/Typography';
import ExpandMoreIcon from '@mui/icons-material/ExpandMore';
import { ReactNode, useEffect, useMemo, useRef, useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import { useAppTitle } from '@/features/navigation-bar/hooks/useAppTitle';
import { useOCR } from '@/Manatan/context/OCRContext';
import { ForkApi, ForkCategory, ForkScanPreview, ForkSource } from '@/features/fork/ForkApi';
import {
    readReaderPreferences,
    ReaderDirection,
    ReaderFit,
    writeReaderPreferences,
} from '@/features/fork/ReaderPreferences';
import { useForkState } from '@/features/fork/useForkState';
import { ActivityContent } from '@/features/fork/Activity';
import { ForkDictionarySettings } from '@/features/settings/components/ForkDictionarySettings';
import { ForkAnkiSettings } from '@/features/settings/components/ForkAnkiSettings';
import { GLOBAL_READER_SETTINGS_MANGA } from '@/features/manga/Manga.constants';
import { IReaderSettings } from '@/features/reader/Reader.types';
import { TapZoneLayouts } from '@/features/reader/tap-zones/TapZoneLayout.types';
import {
    updateReaderSettings,
    useDefaultReaderSettingsWithDefaultFlag,
} from '@/features/reader/settings/ReaderSettingsMetadata';

type SourceDraft = {
    kind: ForkSource['kind'];
    name: string;
    address: string;
    username: string;
    password: string;
    apiKey: string;
    allowHttp: boolean;
    libraryIds: string[];
};
const emptySource: SourceDraft = {
    kind: 'local',
    name: '',
    address: '',
    username: '',
    password: '',
    apiKey: '',
    allowHttp: false,
    libraryIds: [],
};

const pickLocalFolder = (): Promise<string> =>
    new Promise((resolve, reject) => {
        const native = (window as any).MangaReaderNative;
        if (!native?.pickFolder) {
            reject(new Error('Folder selection is available in the Android app'));
            return;
        }
        const callbackId = crypto.randomUUID();
        const nativeWindow = window as any;
        const callbacks: Map<string, (uri?: string) => void> = nativeWindow.mangaReaderFolderCallbacks ?? new Map();
        nativeWindow.mangaReaderFolderCallbacks = callbacks;
        if (!nativeWindow.mangaReaderFolderPicked)
            nativeWindow.mangaReaderFolderPicked = (id: string, uri?: string) => {
                const callback = callbacks.get(id);
                callbacks.delete(id);
                callback?.(uri);
            };
        callbacks.set(callbackId, (uri?: string) => (uri ? resolve(uri) : reject(new Error('No folder was selected'))));
        native.pickFolder(callbackId);
    });

const SourceDialog = ({
    open,
    initialKind,
    onClose,
    onSaved,
}: {
    open: boolean;
    initialKind?: ForkSource['kind'];
    onClose: () => void;
    onSaved: () => void;
}) => {
    const [draft, setDraft] = useState<SourceDraft>({ ...emptySource, kind: initialKind ?? 'local' });
    const [status, setStatus] = useState<string>();
    const [komgaLibraries, setKomgaLibraries] = useState<{ id: string; name: string }[]>([]);
    const [busy, setBusy] = useState(false);
    const update = (key: keyof SourceDraft, value: string | boolean) => setDraft((old) => ({ ...old, [key]: value }));
    const payload = useMemo(
        () => ({ ...draft, password: draft.password || undefined, apiKey: draft.apiKey || undefined }),
        [draft],
    );
    useEffect(() => {
        if (open) setDraft({ ...emptySource, kind: initialKind ?? 'local' });
    }, [initialKind, open]);
    const test = async () => {
        setBusy(true);
        setStatus(undefined);
        try {
            const result = await ForkApi.testSource(payload);
            setStatus(result.detail);
            setKomgaLibraries(result.libraries ?? []);
        } catch (error) {
            setStatus(error instanceof Error ? error.message : String(error));
        } finally {
            setBusy(false);
        }
    };
    const save = async () => {
        setBusy(true);
        setStatus(undefined);
        try {
            const sourcePayload = draft.kind === 'local' ? { ...payload, address: await pickLocalFolder() } : payload;
            await ForkApi.addSource(sourcePayload);
            onSaved();
            onClose();
            setDraft(emptySource);
        } catch (error) {
            setStatus(error instanceof Error ? error.message : String(error));
        } finally {
            setBusy(false);
        }
    };
    return (
        <Dialog open={open} onClose={onClose} fullWidth maxWidth="sm">
            <DialogTitle>Where is your manga?</DialogTitle>
            <DialogContent>
                <Stack spacing={2} sx={{ pt: 1 }}>
                    <TextField
                        select
                        label="Source type"
                        value={draft.kind}
                        onChange={(event) => update('kind', event.target.value)}
                    >
                        <MenuItem value="local">On this phone — choose a folder or archives</MenuItem>
                        <MenuItem value="smb">Remote folder — SMB 2/3</MenuItem>
                        <MenuItem value="webdav">Remote folder — WebDAV</MenuItem>
                        <MenuItem value="komga">Komga — select server libraries</MenuItem>
                    </TextField>
                    <TextField
                        label="Name"
                        value={draft.name}
                        onChange={(event) => update('name', event.target.value)}
                        required
                    />
                    {draft.kind === 'local' ? (
                        <Alert severity="info">
                            Android’s folder picker opens after Save. The app keeps persisted read access and never
                            moves or deletes originals.
                        </Alert>
                    ) : (
                        <>
                            <TextField
                                label={draft.kind === 'smb' ? 'Server and share' : 'Base URL'}
                                value={draft.address}
                                onChange={(event) => update('address', event.target.value)}
                                required
                            />
                            <TextField
                                label="Username"
                                value={draft.username}
                                onChange={(event) => update('username', event.target.value)}
                            />
                            {draft.kind === 'komga' && (
                                <TextField
                                    label="API key (preferred)"
                                    type="password"
                                    value={draft.apiKey}
                                    onChange={(event) => update('apiKey', event.target.value)}
                                />
                            )}
                            <TextField
                                label="Password"
                                type="password"
                                value={draft.password}
                                onChange={(event) => update('password', event.target.value)}
                            />
                            {draft.kind !== 'smb' && (
                                <FormControlLabel
                                    control={
                                        <Switch
                                            checked={draft.allowHttp}
                                            onChange={(_, checked) => update('allowHttp', checked)}
                                        />
                                    }
                                    label="Allow insecure HTTP for this source"
                                />
                            )}
                            {draft.kind === 'komga' &&
                                komgaLibraries.map((library) => (
                                    <FormControlLabel
                                        key={library.id}
                                        control={
                                            <Checkbox
                                                checked={draft.libraryIds.includes(library.id)}
                                                onChange={(_, checked) =>
                                                    setDraft((old) => ({
                                                        ...old,
                                                        libraryIds: checked
                                                            ? [...old.libraryIds, library.id]
                                                            : old.libraryIds.filter((id) => id !== library.id),
                                                    }))
                                                }
                                            />
                                        }
                                        label={library.name}
                                    />
                                ))}
                        </>
                    )}
                    {status && (
                        <Alert severity={status.toLowerCase().includes('success') ? 'success' : 'info'}>{status}</Alert>
                    )}
                </Stack>
            </DialogContent>
            <DialogActions>
                <Button onClick={onClose}>Cancel</Button>
                {draft.kind !== 'local' && (
                    <Button disabled={busy} onClick={test}>
                        Test connection
                    </Button>
                )}
                <Button
                    variant="contained"
                    disabled={
                        busy ||
                        !draft.name ||
                        (draft.kind !== 'local' && !draft.address) ||
                        (draft.kind === 'komga' && draft.libraryIds.length === 0)
                    }
                    onClick={save}
                >
                    Save
                </Button>
            </DialogActions>
        </Dialog>
    );
};

export function Settings() {
    useAppTitle('Settings');
    const { settings, setSettings } = useOCR();
    const { state, setState, error, refresh } = useForkState();
    const [sourceDialog, setSourceDialog] = useState(false);
    const [sourceToRemove, setSourceToRemove] = useState<ForkSource>();
    const restoreInput = useRef<HTMLInputElement>(null);
    const [backupStatus, setBackupStatus] = useState<string>();
    const [scanStatus, setScanStatus] = useState<string>();
    const [scanPreview, setScanPreview] = useState<{ source: ForkSource; preview: ForkScanPreview }>();
    const [scanTitleOverrides, setScanTitleOverrides] = useState<Record<string, string>>({});
    const [categoryName, setCategoryName] = useState('');
    const [categoryToRemove, setCategoryToRemove] = useState<ForkCategory>();
    const [categoryToRename, setCategoryToRename] = useState<ForkCategory>();
    const [categoryRenameName, setCategoryRenameName] = useState('');
    const [readerPreferences, setReaderPreferences] = useState(readReaderPreferences);
    const [ocrServerStatus, setOcrServerStatus] = useState<string>();
    const [testingOcrServer, setTestingOcrServer] = useState(false);
    const {
        settings: defaultReaderSettings,
        loading: defaultReaderSettingsLoading,
        request: defaultReaderSettingsRequest,
    } = useDefaultReaderSettingsWithDefaultFlag();
    const [readerSettingsStatus, setReaderSettingsStatus] = useState<string>();
    const [searchParams, setSearchParams] = useSearchParams();
    const requestedKind = searchParams.get('addSource');
    const initialKind: ForkSource['kind'] | undefined =
        requestedKind === 'local' || requestedKind === 'smb' || requestedKind === 'webdav' || requestedKind === 'komga'
            ? requestedKind
            : undefined;
    useEffect(() => {
        if (initialKind) setSourceDialog(true);
    }, [initialKind]);
    const closeSourceDialog = () => {
        setSourceDialog(false);
        if (requestedKind) setSearchParams({}, { replace: true });
    };
    const updateDefaultReaderSetting = async <Setting extends keyof IReaderSettings>(
        key: Setting,
        value: IReaderSettings[Setting],
    ) => {
        setReaderSettingsStatus(undefined);
        try {
            await updateReaderSettings(GLOBAL_READER_SETTINGS_MANGA, key, value, true);
            await defaultReaderSettingsRequest.refetch();
        } catch (cause) {
            setReaderSettingsStatus(cause instanceof Error ? cause.message : String(cause));
        }
    };
    const exportBackup = async () => {
        try {
            const fork = await ForkApi.exportBackup();
            const content = JSON.stringify(
                { product: 'manga-reader-fork', schemaVersion: 1, fork, settings, readerPreferences },
                null,
                2,
            );
            const native = (window as any).MangaReaderNative;
            if (native?.saveFile)
                native.saveFile(
                    `mangaki-backup-${new Date().toISOString().slice(0, 10)}.json`,
                    'application/json',
                    content,
                );
            else {
                const link = document.createElement('a');
                link.href = URL.createObjectURL(new Blob([content], { type: 'application/json' }));
                link.download = 'mangaki-backup.json';
                link.click();
                URL.revokeObjectURL(link.href);
            }
            setBackupStatus('Backup created. Credentials and Android folder grants were excluded.');
        } catch (cause) {
            setBackupStatus(cause instanceof Error ? cause.message : String(cause));
        }
    };
    const restoreBackup = async (file?: File) => {
        if (!file) return;
        try {
            const backup = JSON.parse(await file.text());
            if (
                backup?.product !== 'manga-reader-fork' ||
                backup?.schemaVersion !== 1 ||
                !backup?.fork ||
                !backup?.settings
            )
                throw new Error('This is not a supported Mangaki backup');
            await ForkApi.restoreBackup(backup.fork);
            setSettings({ ...settings, ...backup.settings });
            if (backup.readerPreferences) {
                setReaderPreferences(backup.readerPreferences);
                writeReaderPreferences(backup.readerPreferences);
            }
            await refresh();
            setBackupStatus('Backup restored. Re-authorise folders and remote credentials before scanning.');
        } catch (cause) {
            setBackupStatus(cause instanceof Error ? cause.message : String(cause));
        } finally {
            if (restoreInput.current) restoreInput.current.value = '';
        }
    };
    const sections = [
        [
            'Sources & storage',
            <Stack key="sources" spacing={1.5}>
                {state?.sources.map((source) => (
                    <Stack key={source.id} direction="row" justifyContent="space-between" alignItems="center">
                        <span>
                            {source.name} · {source.kind.toUpperCase()}
                        </span>
                        <Stack direction="row" spacing={1} alignItems="center">
                            <Chip
                                size="small"
                                label={source.status.replace('_', ' ')}
                                color={source.status === 'connected' ? 'success' : 'default'}
                            />
                            <Button
                                size="small"
                                onClick={async () => {
                                    setScanStatus(`Inspecting ${source.name}…`);
                                    try {
                                        const preview = await ForkApi.previewSource(source.id);
                                        setScanTitleOverrides({});
                                        setScanPreview({ source, preview });
                                        setScanStatus(undefined);
                                    } catch (cause) {
                                        setScanStatus(cause instanceof Error ? cause.message : String(cause));
                                    }
                                }}
                            >
                                Preview scan
                            </Button>
                            <Button size="small" color="error" onClick={() => setSourceToRemove(source)}>
                                Remove
                            </Button>
                        </Stack>
                    </Stack>
                ))}
                <Button startIcon={<AddIcon />} onClick={() => setSourceDialog(true)}>
                    Add source
                </Button>
                <Typography variant="body2" color="text.secondary">
                    Local manga is read from the folder you selected. “Available offline” means remote pages have also
                    been copied into this fork’s private storage for reading without the server.
                </Typography>
                {scanStatus && <Alert severity="info">{scanStatus}</Alert>}
            </Stack>,
        ],
        [
            'Manga reader',
            <Stack key="manga-reader" spacing={1.5}>
                <TextField
                    select
                    size="small"
                    label="Tap zones"
                    value={defaultReaderSettings.tapZoneLayout.value}
                    disabled={defaultReaderSettingsLoading}
                    onChange={(event) =>
                        void updateDefaultReaderSetting(
                            'tapZoneLayout',
                            Number(event.target.value) as TapZoneLayouts,
                        )
                    }
                    helperText="Controls page-turn and menu tap areas in the reader. Choose Disabled to turn off edge page turns."
                >
                    <MenuItem value={TapZoneLayouts.EDGE}>Edges</MenuItem>
                    <MenuItem value={TapZoneLayouts.KINDLE}>Kindle</MenuItem>
                    <MenuItem value={TapZoneLayouts.L_SHAPE}>L shape</MenuItem>
                    <MenuItem value={TapZoneLayouts.RIGHT_LEFT}>Left and right</MenuItem>
                    <MenuItem value={TapZoneLayouts.DISABLED}>Disabled</MenuItem>
                </TextField>
                {readerSettingsStatus && <Alert severity="error">{readerSettingsStatus}</Alert>}
                <TextField
                    select
                    size="small"
                    label="Reading direction"
                    value={readerPreferences.direction}
                    onChange={(event) => {
                        const value = { ...readerPreferences, direction: event.target.value as ReaderDirection };
                        setReaderPreferences(value);
                        writeReaderPreferences(value);
                    }}
                >
                    <MenuItem value="rtl">Right to left (manga default)</MenuItem>
                    <MenuItem value="ltr">Left to right</MenuItem>
                    <MenuItem value="vertical">Vertical</MenuItem>
                </TextField>
                <TextField
                    select
                    size="small"
                    label="Page fit"
                    value={readerPreferences.fit}
                    onChange={(event) => {
                        const value = { ...readerPreferences, fit: event.target.value as ReaderFit };
                        setReaderPreferences(value);
                        writeReaderPreferences(value);
                    }}
                >
                    <MenuItem value="contain">Fit whole page</MenuItem>
                    <MenuItem value="width">Fit width</MenuItem>
                </TextField>
                <FormControlLabel
                    control={
                        <Switch
                            checked={readerPreferences.twoPage}
                            onChange={(_, checked) => {
                                const value = { ...readerPreferences, twoPage: checked };
                                setReaderPreferences(value);
                                writeReaderPreferences(value);
                            }}
                        />
                    }
                    label="Two-page spread on wide screens"
                />
                <TextField
                    select
                    size="small"
                    label="OCR provider"
                    value={settings.ocrProvider ?? (settings.allowGoogleLensOcr ? 'lens' : 'on_device')}
                    onChange={(event) => {
                        const provider = event.target.value as 'on_device' | 'self_hosted' | 'lens';
                        setSettings({ ...settings, ocrProvider: provider, allowGoogleLensOcr: provider === 'lens' });
                        setOcrServerStatus(undefined);
                    }}
                >
                    <MenuItem value="on_device">On-device (private, offline)</MenuItem>
                    <MenuItem value="self_hosted">Self-hosted Manga OCR</MenuItem>
                    <MenuItem value="lens">Google Lens (uploads pages)</MenuItem>
                </TextField>
                {settings.ocrProvider === 'self_hosted' && (
                    <>
                        <TextField
                            size="small"
                            label="Self-hosted OCR URL"
                            placeholder="http://100.x.y.z:7332"
                            value={settings.selfHostedOcrUrl}
                            onChange={(event) => {
                                setSettings({ ...settings, selfHostedOcrUrl: event.target.value });
                                setOcrServerStatus(undefined);
                            }}
                            helperText="Use the server's Tailscale address. New reader requests and bulk OCR use this provider."
                        />
                        <Button
                            variant="outlined"
                            disabled={testingOcrServer || !settings.selfHostedOcrUrl.trim()}
                            onClick={async () => {
                                setTestingOcrServer(true);
                                setOcrServerStatus(undefined);
                                try {
                                    const result = await ForkApi.testSelfHostedOcr(settings.selfHostedOcrUrl.trim());
                                    setOcrServerStatus(`Connected: ${result.engine} (${result.modelVersion})`);
                                } catch (cause) {
                                    setOcrServerStatus(cause instanceof Error ? cause.message : String(cause));
                                } finally {
                                    setTestingOcrServer(false);
                                }
                            }}
                        >
                            {testingOcrServer ? 'Testing…' : 'Test OCR server'}
                        </Button>
                        {ocrServerStatus && (
                            <Alert severity={ocrServerStatus.startsWith('Connected') ? 'success' : 'error'}>
                                {ocrServerStatus}
                            </Alert>
                        )}
                    </>
                )}
                {settings.ocrProvider === 'lens' && (
                    <Alert severity="warning">Google Lens sends each requested manga page to Google.</Alert>
                )}
                {settings.ocrProvider === 'on_device' && (
                    <Typography variant="body2" color="text.secondary">
                        The bundled ML Kit model works offline but is less accurate on manga and furigana.
                    </Typography>
                )}
                <TextField
                    select
                    size="small"
                    label="OCR text interaction"
                    value={settings.ocrBoxlessMode ? 'artwork' : 'boxes'}
                    onChange={(event) => setSettings({ ...settings, ocrBoxlessMode: event.target.value === 'artwork' })}
                >
                    <MenuItem value="boxes">Show OCR text boxes</MenuItem>
                    <MenuItem value="artwork">Tap original artwork (experimental)</MenuItem>
                </TextField>
                <Typography variant="body2" color="text.secondary">
                    Reader direction, fit and OCR interaction are also reflected by the reader’s quick controls.
                </Typography>
            </Stack>,
        ],
        ['Dictionaries', <ForkDictionarySettings key="dictionaries" />],
        ['Anki & mining', <ForkAnkiSettings key="anki" />],
        [
            'Library & privacy',
            <Stack key="privacy" spacing={1.5}>
                <FormControlLabel
                    control={
                        <Switch
                            checked={state?.showSensitive ?? false}
                            onChange={async (_, checked) => setState(await ForkApi.setSensitiveVisibility(checked))}
                        />
                    }
                    label="Show hidden manga"
                />
                <Typography variant="subtitle2">Categories</Typography>
                {state?.categories?.map((category, index) => (
                    <Stack key={category.id} direction="row" alignItems="center" justifyContent="space-between">
                        <Typography>{category.name}</Typography>
                        <Stack direction="row" spacing={0.5}>
                            <Button
                                size="small"
                                disabled={index === 0}
                                onClick={async () => {
                                    const ids = state.categories.map((item) => item.id);
                                    [ids[index - 1], ids[index]] = [ids[index], ids[index - 1]];
                                    await ForkApi.reorderCategories(ids);
                                    await refresh();
                                }}
                            >
                                Up
                            </Button>
                            <Button
                                size="small"
                                disabled={index + 1 === state.categories.length}
                                onClick={async () => {
                                    const ids = state.categories.map((item) => item.id);
                                    [ids[index], ids[index + 1]] = [ids[index + 1], ids[index]];
                                    await ForkApi.reorderCategories(ids);
                                    await refresh();
                                }}
                            >
                                Down
                            </Button>
                            <Button
                                size="small"
                                onClick={() => {
                                    setCategoryToRename(category);
                                    setCategoryRenameName(category.name);
                                }}
                            >
                                Rename
                            </Button>
                            <Button color="error" size="small" onClick={() => setCategoryToRemove(category)}>
                                Remove
                            </Button>
                        </Stack>
                    </Stack>
                ))}
                <Stack direction="row" spacing={1}>
                    <TextField
                        size="small"
                        label="New category"
                        value={categoryName}
                        onChange={(event) => setCategoryName(event.target.value)}
                        inputProps={{ maxLength: 80 }}
                    />
                    <Button
                        disabled={!categoryName.trim()}
                        onClick={async () => {
                            await ForkApi.addCategory(categoryName);
                            setCategoryName('');
                            await refresh();
                        }}
                    >
                        Add
                    </Button>
                </Stack>
            </Stack>,
        ],
        ['Background activity', <ActivityContent key="activity" embedded />],
        [
            'Statistics',
            <Typography key="statistics" color="text.secondary">
                Local-only reading sessions grouped by manga. Sessions can be selected and removed individually or in
                bulk from Stats.
            </Typography>,
        ],
        [
            'Backup / About',
            <Stack key="backup" spacing={1}>
                <Typography color="text.secondary">
                    Exports settings, source references, job history, visibility, and local statistics. Credentials are
                    always excluded; folder and server access must be re-authorised after restore.
                </Typography>
                <Stack direction="row" spacing={1}>
                    <Button onClick={exportBackup}>Export backup</Button>
                    <Button onClick={() => restoreInput.current?.click()}>Restore backup</Button>
                </Stack>
                <input
                    ref={restoreInput}
                    hidden
                    type="file"
                    accept="application/json,.json"
                    onChange={(event) => restoreBackup(event.target.files?.[0])}
                />
                {backupStatus && <Alert severity="info">{backupStatus}</Alert>}
            </Stack>,
        ],
    ] as const;
    const sectionIcons: Record<string, ReactNode> = {
        'Sources & storage': <FolderOpenOutlinedIcon />,
        'Manga reader': <ImageSearchOutlinedIcon />,
        Dictionaries: <ManageSearchOutlinedIcon />,
        'Anki & mining': <Inventory2OutlinedIcon />,
        'Library & privacy': <HideImageOutlinedIcon />,
        'Background activity': <SyncOutlinedIcon />,
        Statistics: <BarChartOutlinedIcon />,
        'Backup / About': <BackupOutlinedIcon />,
    };
    return (
        <Stack
            spacing={2}
            sx={{ p: { xs: 1.5, sm: 3 }, pb: { xs: 10, sm: 3 }, maxWidth: 900, mx: 'auto', width: '100%' }}
        >
            {error && <Alert severity="error">Fork settings storage is unavailable: {error}</Alert>}
            <Box
                sx={{
                    border: '1px solid rgba(255,255,255,0.72)',
                    borderRadius: 3,
                    overflow: 'hidden',
                    bgcolor: 'transparent',
                }}
            >
                {sections.map(([title, content], index) => (
                    <Accordion
                        key={title}
                        disableGutters
                        elevation={0}
                        sx={{
                            bgcolor: 'transparent',
                            backgroundImage: 'none',
                            borderBottom: index + 1 === sections.length ? 0 : '1px solid rgba(255,255,255,0.42)',
                            '&:before': { display: 'none' },
                            '&.Mui-expanded': { m: 0 },
                        }}
                    >
                        <AccordionSummary
                            expandIcon={<ExpandMoreIcon />}
                            sx={{
                                px: 2,
                                minHeight: 62,
                                '&.Mui-expanded': { minHeight: 62 },
                                '& .MuiAccordionSummary-content': { my: 1.25, alignItems: 'center', gap: 1.5 },
                                '& .MuiAccordionSummary-content.Mui-expanded': { my: 1.25 },
                            }}
                        >
                            <Box sx={{ display: 'flex', color: 'text.primary' }}>{sectionIcons[title]}</Box>
                            <Typography variant="subtitle1" fontWeight={500}>
                                {title}
                            </Typography>
                        </AccordionSummary>
                        <AccordionDetails sx={{ px: 2, pt: 0.5, pb: 2.25 }}>{content}</AccordionDetails>
                    </Accordion>
                ))}
            </Box>
            <SourceDialog open={sourceDialog} initialKind={initialKind} onClose={closeSourceDialog} onSaved={refresh} />
            <Dialog open={Boolean(scanPreview)} onClose={() => setScanPreview(undefined)} fullWidth maxWidth="sm">
                <DialogTitle>Scan preview</DialogTitle>
                <DialogContent>
                    <Typography color="text.secondary" sx={{ mb: 2 }}>
                        Review detected groups before importing. Edit names to correct labels, or give two detected
                        groups the same name to merge them. Original files and server metadata are never renamed.
                    </Typography>
                    <Stack spacing={1.5} sx={{ maxHeight: '55vh', overflowY: 'auto' }}>
                        {scanPreview?.preview.series.map((series) => (
                            <Stack key={series.id} spacing={0.5}>
                                <TextField
                                    size="small"
                                    label="Detected series"
                                    value={scanTitleOverrides[series.id] ?? series.title}
                                    inputProps={{ maxLength: 200 }}
                                    onChange={(event) =>
                                        setScanTitleOverrides((current) => ({
                                            ...current,
                                            [series.id]: event.target.value,
                                        }))
                                    }
                                />
                                <Typography variant="caption" color="text.secondary">
                                    {series.volumeCount} volumes · {series.pageCount} pages
                                    {series.warnings ? ` · ${series.warnings} warnings` : ''}
                                </Typography>
                            </Stack>
                        ))}
                    </Stack>
                </DialogContent>
                <DialogActions>
                    <Button onClick={() => setScanPreview(undefined)}>Cancel</Button>
                    <Button
                        variant="contained"
                        disabled={Object.values(scanTitleOverrides).some((title) => !title.trim())}
                        onClick={async () => {
                            if (!scanPreview) return;
                            setScanStatus(`Importing ${scanPreview.source.name}…`);
                            try {
                                const catalogue = await ForkApi.applySourceScan(
                                    scanPreview.source.id,
                                    scanTitleOverrides,
                                );
                                const sourceSeries = catalogue.filter(
                                    (item) => item.sourceId === scanPreview.source.id,
                                );
                                setScanStatus(
                                    `Imported ${scanPreview.source.name}: ${sourceSeries.length} series, ${sourceSeries.reduce((count, item) => count + item.volumes.length, 0)} volumes.`,
                                );
                                setScanPreview(undefined);
                                await refresh();
                            } catch (cause) {
                                setScanStatus(cause instanceof Error ? cause.message : String(cause));
                            }
                        }}
                    >
                        Import catalogue
                    </Button>
                </DialogActions>
            </Dialog>
            <Dialog open={Boolean(sourceToRemove)} onClose={() => setSourceToRemove(undefined)}>
                <DialogTitle>Remove source?</DialogTitle>
                <DialogContent>
                    <Typography>
                        {sourceToRemove?.name} will be removed from Mangaki. Original manga on the phone or server will
                        not be changed or deleted.
                    </Typography>
                </DialogContent>
                <DialogActions>
                    <Button onClick={() => setSourceToRemove(undefined)}>Cancel</Button>
                    <Button
                        color="error"
                        onClick={async () => {
                            if (!sourceToRemove) return;
                            await ForkApi.removeSource(sourceToRemove.id);
                            setSourceToRemove(undefined);
                            await refresh();
                        }}
                    >
                        Remove source
                    </Button>
                </DialogActions>
            </Dialog>
            <Dialog open={Boolean(categoryToRemove)} onClose={() => setCategoryToRemove(undefined)}>
                <DialogTitle>Remove category?</DialogTitle>
                <DialogContent>
                    <Typography>
                        “{categoryToRemove?.name}” will be removed. Manga and their reading progress will remain.
                    </Typography>
                </DialogContent>
                <DialogActions>
                    <Button onClick={() => setCategoryToRemove(undefined)}>Cancel</Button>
                    <Button
                        color="error"
                        onClick={async () => {
                            if (!categoryToRemove) return;
                            await ForkApi.removeCategory(categoryToRemove.id);
                            setCategoryToRemove(undefined);
                            await refresh();
                        }}
                    >
                        Remove category
                    </Button>
                </DialogActions>
            </Dialog>
            <Dialog
                open={Boolean(categoryToRename)}
                onClose={() => setCategoryToRename(undefined)}
                fullWidth
                maxWidth="xs"
            >
                <DialogTitle>Rename category</DialogTitle>
                <DialogContent>
                    <TextField
                        autoFocus
                        fullWidth
                        sx={{ mt: 1 }}
                        label="Category name"
                        value={categoryRenameName}
                        inputProps={{ maxLength: 80 }}
                        onChange={(event) => setCategoryRenameName(event.target.value)}
                    />
                </DialogContent>
                <DialogActions>
                    <Button onClick={() => setCategoryToRename(undefined)}>Cancel</Button>
                    <Button
                        disabled={!categoryRenameName.trim()}
                        onClick={async () => {
                            if (!categoryToRename) return;
                            await ForkApi.renameCategory(categoryToRename.id, categoryRenameName);
                            setCategoryToRename(undefined);
                            await refresh();
                        }}
                    >
                        Rename
                    </Button>
                </DialogActions>
            </Dialog>
        </Stack>
    );
}
