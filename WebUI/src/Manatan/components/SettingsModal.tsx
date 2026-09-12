import React, { useCallback, useMemo, useState, useRef, useEffect } from 'react';
import IconButton from '@mui/material/IconButton';
import RestartAltIcon from '@mui/icons-material/RestartAlt';
import { useOCR } from '@/Manatan/context/OCRContext';
import { COLOR_THEMES, DEFAULT_SETTINGS } from '@/Manatan/types';
import { apiRequest, getFrequencyDictionaries, getDictionaries } from '@/Manatan/utils/api';
import { DictionaryManager } from './DictionaryManager';
import {
    addNote,
    canAddNotes,
    getAnkiVersion,
    getDeckNames,
    findNotes,
    getModelNames,
    getModelFields,
    logAnkiError,
    notesInfo,
} from '@/Manatan/utils/anki';
import { ResetButton } from '@/base/components/buttons/ResetButton.tsx';

const checkboxLabelStyle: React.CSSProperties = {
    display: 'flex', alignItems: 'center', marginBottom: '8px', cursor: 'pointer', textAlign: 'left', 
};

const checkboxInputStyle: React.CSSProperties = {
    width: 'auto', marginRight: '10px', flexShrink: 0, cursor: 'pointer',
};

const sectionBoxStyle: React.CSSProperties = {
    backgroundColor: 'rgba(0, 0, 0, 0.2)',
    padding: 'var(--settings-section-padding, 15px)',
    borderRadius: '8px',
    border: '1px solid rgba(255, 255, 255, 0.1)',
    marginBottom: 'var(--settings-section-margin, 20px)',
};

const statusDotStyle = (connected: boolean): React.CSSProperties => ({
    width: '10px',
    height: '10px',
    borderRadius: '50%',
    backgroundColor: connected ? '#2ecc71' : '#e74c3c',
    display: 'inline-block',
    marginRight: '8px',
    boxShadow: connected ? '0 0 5px #2ecc71' : 'none'
});

const inlineInputWrapperStyle: React.CSSProperties = {
    position: 'relative',
    display: 'flex',
    alignItems: 'center',
};

const inlineInputActionsStyle: React.CSSProperties = {
    position: 'absolute',
    right: 6,
    top: '50%',
    transform: 'translateY(-50%)',
    display: 'flex',
    gap: '4px',
    alignItems: 'center',
};

const inlineInputStyle: React.CSSProperties = {
    paddingRight: '94px',
};

const inlineInputCompactStyle: React.CSSProperties = {
    paddingRight: '42px',
};

const BASE_MAPPING_OPTIONS = [
    'None',
    'Sentence',
    'Sentence Furigana',
    'Sentence Audio',
    'Word Audio',
    'Image',
    'Furigana',
    'Reading',
    'Target Word',
    'Word (Again)',
    'Glossary',
    'Frequency',
    'Harmonic Frequency',
    'Pitch Accent',
    'Pitch Accent Positions',
    'Pitch Accent Categories',
    'x',
];

const SINGLE_GLOSSARY_PREFIX = 'Single Glossary ';

const getSingleGlossaryName = (value: string): string | null => {
    if (value.startsWith(SINGLE_GLOSSARY_PREFIX)) {
        const name = value.slice(SINGLE_GLOSSARY_PREFIX.length).trim();
        return name ? name : null;
    }
    if (value.startsWith('Single Glossary:')) {
        const name = value.replace('Single Glossary:', '').trim();
        return name ? name : null;
    }
    return null;
};

const DOWNSCALE_OPTIONS = [
    { value: undefined, label: 'Original Size' },
    { value: '240', label: '240' },
    { value: '360', label: '360' },
    { value: '480', label: '480' },
    { value: '720', label: '720' },
    { value: '900', label: '900' },
    { value: '1080', label: '1080' },
    { value: '1200', label: '1200' },
    { value: '1440', label: '1440' },
    { value: '1600', label: '1600' },
    { value: '1920', label: '1920' },
    { value: '2560', label: '2560' },
    { value: '3840', label: '3840' },
];

const ACCEPTANCE_TEST_TAG = 'codex_manga_reader_acceptance_20260908';
const ACCEPTANCE_TEST_MARKER = 'MangaReaderSyntheticAcceptance20260908';
const ACCEPTANCE_TEST_IMAGE_BASE64 =
    'iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAFElEQVR42mP8z8DAwMDAxAADCBYAOxgCAfFzB9sAAAAASUVORK5CYII=';

const syntheticValueForMapping = (mapping: string | undefined): string => {
    switch (mapping) {
        case 'Target Word': return '合成試験';
        case 'Word (Again)': return '合成試験';
        case 'Reading': return 'ごうせいしけん';
        case 'Furigana': return '合成[ごうせい] 試験[しけん]';
        case 'Sentence': return 'これは合成された受け入れ試験です。';
        case 'Sentence Furigana': return 'これは 合成[ごうせい] された 受[う]け 入[い]れ 試験[しけん] です。';
        case 'Glossary':
        case 'Definition': return 'Synthetic Mangaki acceptance test; no manga content.';
        case 'Frequency':
        case 'Harmonic Frequency': return '999999';
        case 'Pitch Accent': return 'synthetic';
        case 'Pitch Accent Positions': return '0';
        case 'Pitch Accent Categories': return 'synthetic';
        case 'x': return 'x';
        default: return '';
    }
};

export const SettingsModal: React.FC<{ onClose: () => void }> = ({ onClose }) => {
    const { settings, setSettings, showConfirm, showAlert, showProgress, closeDialog, showDialog } = useOCR();
    const [localSettings, setLocalSettings] = useState(settings);
    const [showAdvancedAnki, setShowAdvancedAnki] = useState(false);
    const fileInputRef = useRef<HTMLInputElement>(null);
    const [dictManagerKey, setDictManagerKey] = useState(0);
    const [dictionaryNames, setDictionaryNames] = useState<string[]>([]);
    const yomitanPopupWidthValue = Number.isFinite(localSettings.yomitanPopupWidthPx)
        ? localSettings.yomitanPopupWidthPx
        : DEFAULT_SETTINGS.yomitanPopupWidthPx;
    const yomitanPopupHeightValue = Number.isFinite(localSettings.yomitanPopupHeightPx)
        ? localSettings.yomitanPopupHeightPx
        : DEFAULT_SETTINGS.yomitanPopupHeightPx;
    const yomitanPopupScaleValue = Number.isFinite(localSettings.yomitanPopupScalePercent)
        ? localSettings.yomitanPopupScalePercent
        : DEFAULT_SETTINGS.yomitanPopupScalePercent;
    const yomitanPopupThemeValue = localSettings.yomitanPopupTheme || 'dark';
    const persistSettings = useCallback((nextSettings: typeof settings) => {
        setSettings(nextSettings);
    }, [setSettings]);

    useEffect(() => {
        let cancelled = false;
        const fetchDictionaries = async () => {
            const list = await getDictionaries();
            if (!list || cancelled) {
                if (!cancelled) {
                    setTimeout(fetchDictionaries, 1000);
                }
                return;
            }
            const names = Array.from(new Set(list.map((dict) => dict.name).filter(Boolean)));
            setDictionaryNames(names);
            setLocalSettings((prev) => {
                if (!prev.ankiFieldMap) {
                    return prev;
                }
                let changed = false;
                const nextMap = { ...prev.ankiFieldMap };
                Object.entries(nextMap).forEach(([field, value]) => {
                    if (typeof value !== 'string') {
                        return;
                    }
                    const name = getSingleGlossaryName(value);
                    if (name && !names.includes(name)) {
                        nextMap[field] = 'None';
                        changed = true;
                    }
                });
                if (!changed) {
                    return prev;
                }
                const next = { ...prev, ankiFieldMap: nextMap };
                persistSettings(next);
                return next;
            });
        };
        fetchDictionaries();
        return () => {
            cancelled = true;
        };
    }, [dictManagerKey, persistSettings]);

    const mappingOptions = useMemo(() => {
        const baseOptions = BASE_MAPPING_OPTIONS.map((option) => ({ value: option, label: option }));
        const glossaryOptions = dictionaryNames.map((name) => ({
            value: `${SINGLE_GLOSSARY_PREFIX}${name}`,
            label: `${SINGLE_GLOSSARY_PREFIX}${name}`,
        }));

        return [...baseOptions, ...glossaryOptions];
    }, [dictionaryNames, localSettings.ankiFieldMap]);
    const [availableFreqDicts, setAvailableFreqDicts] = useState<string[]>([]);

    // --- ANKI STATE ---
    const [ankiStatus, setAnkiStatus] = useState<'idle' | 'loading' | 'connected' | 'error'>('idle');
    const [ankiDecks, setAnkiDecks] = useState<string[]>([]);
    const [ankiModels, setAnkiModels] = useState<string[]>([]);
    const [currentModelFields, setCurrentModelFields] = useState<string[]>([]);

    useEffect(() => {
        const fetchFreqDicts = async () => {
            const dicts = await getFrequencyDictionaries();
            setAvailableFreqDicts(dicts);
        };
        fetchFreqDicts();
    }, []);
    // --- ANKI EFFECT ---
    const fetchAnkiData = async () => {
        if (!localSettings.ankiConnectEnabled) return;
        
        const url = localSettings.ankiConnectUrl || 'http://127.0.0.1:18765';
        setAnkiStatus('loading');
        
        const status = await getAnkiVersion(url);
        if (status.ok) {
            setAnkiStatus('connected');
            try {
                const [d, m] = await Promise.all([
                    getDeckNames(url),
                    getModelNames(url)
                ]);
                setAnkiDecks(d);
                setAnkiModels(m);
            } catch (e) {
                logAnkiError("Failed to fetch anki metadata", e);
            }
        } else {
            setAnkiStatus('error');
        }
    };

    useEffect(() => {
        if (localSettings.ankiConnectEnabled) {
            fetchAnkiData();
        }
    }, [localSettings.ankiConnectEnabled]); 

    // Fetch fields when model changes or when connection is established with a pre-selected model
    useEffect(() => {
        const fetchFields = async () => {
             const url = localSettings.ankiConnectUrl || 'http://127.0.0.1:18765';
             if (ankiStatus === 'connected' && localSettings.ankiModel) {
                 try {
                     const f = await getModelFields(url, localSettings.ankiModel);
                     setCurrentModelFields(f);
                 } catch (e) {
                     logAnkiError("Failed to fetch anki model fields", e);
                 }
             }
        };
        fetchFields();
    }, [localSettings.ankiModel, ankiStatus]);


    // --- SETTINGS LOGIC ---
    const handleChange = (key: keyof typeof settings | string, value: any) => {
        setLocalSettings((prev) => {
            const next = { ...prev, [key]: value };
            persistSettings(next);
            return next;
        });
    };

    const resetYomitanPopupWidth = () => {
        handleChange('yomitanPopupWidthPx', DEFAULT_SETTINGS.yomitanPopupWidthPx);
    };

    const resetYomitanPopupHeight = () => {
        handleChange('yomitanPopupHeightPx', DEFAULT_SETTINGS.yomitanPopupHeightPx);
    };

    const resetYomitanPopupScale = () => {
        handleChange('yomitanPopupScalePercent', DEFAULT_SETTINGS.yomitanPopupScalePercent);
    };

    const resetYomitanPopupCustomCss = () => {
        handleChange('yomitanPopupCustomCss', DEFAULT_SETTINGS.yomitanPopupCustomCss);
    };

    const handleFieldMapChange = (ankiField: string, mapValue: string) => {
        const currentMap = (localSettings.ankiFieldMap as Record<string, string>) || {};
        const newMap = { ...currentMap, [ankiField]: mapValue };

        // Ensure "Target Word" is unique
        if (mapValue === 'Target Word') {
            Object.keys(newMap).forEach(key => {
                if (key !== ankiField && newMap[key] === 'Target Word') {
                    newMap[key] = 'None';
                }
            });
        }

        handleChange('ankiFieldMap', newMap);
    };

    const handleUpdateModeChange = (field: string, mode: string) => {
        const currentModes = localSettings.ankiFieldUpdateModes || {};
        handleChange('ankiFieldUpdateModes', { ...currentModes, [field]: mode });
    };

    const updateModes = [
        { value: 'overwrite', label: 'Overwrite' },
        { value: 'overwrite-if-available', label: 'Overwrite if available' },
        { value: 'append', label: 'Append' },
        { value: 'prepend', label: 'Prepend' },
        { value: 'fill-if-empty', label: 'Fill if empty' },
        { value: 'skip', label: 'Skip' },
    ];

    // New helper to handle the inverted selection (Content -> Field)
    const handleContentToFieldChange = (contentType: string, targetField: string) => {
        const newMap = { ...localSettings.ankiFieldMap };

        // 1. Remove this content type from any other fields to prevent duplicates
        Object.keys(newMap).forEach(key => {
            if (newMap[key] === contentType) {
                delete newMap[key]; 
            }
        });

        // 2. Assign the content type to the new target field
        if (targetField) {
            newMap[targetField] = contentType;
        }

        handleChange('ankiFieldMap', newMap);
    };

    // Helper to find the field currently mapped to a specific content type
    const getFieldForContent = (contentType: string) => {
        return Object.keys(localSettings.ankiFieldMap || {}).find(key => localSettings.ankiFieldMap?.[key] === contentType) || '';
    };

    const runSyntheticAnkiTest = async () => {
        const deckName = localSettings.ankiDeck;
        const modelName = localSettings.ankiModel;
        if (!deckName || !modelName || currentModelFields.length === 0) {
            showAlert('Anki test unavailable', 'Select a target deck and card type, then retry the connection.');
            return;
        }

        const fieldMap = localSettings.ankiFieldMap || {};
        const fields: Record<string, string> = {};
        for (const field of currentModelFields) {
            fields[field] = syntheticValueForMapping(fieldMap[field]);
        }
        const firstField = currentModelFields[0];
        fields[firstField] = fields[firstField]
            ? `${ACCEPTANCE_TEST_MARKER} ${fields[firstField]}`
            : ACCEPTANCE_TEST_MARKER;

        const imageField = currentModelFields.find((field) => fieldMap[field] === 'Image');
        const picture = imageField ? {
            data: ACCEPTANCE_TEST_IMAGE_BASE64,
            filename: 'manga_reader_synthetic_acceptance_20260908.png',
            fields: [imageField],
        } : undefined;
        const url = localSettings.ankiConnectUrl || 'http://127.0.0.1:18765';

        const verifyNote = (created: any): boolean => {
            const createdTags: string[] = created?.tags || [];
            const createdFields = created?.fields || {};
            const markerPersisted = String(createdFields[firstField]?.value || '').includes(ACCEPTANCE_TEST_MARKER);
            const imagePersisted = !imageField
                || /<img src=["'][^"']+["']>/.test(String(createdFields[imageField]?.value || ''));
            return created?.modelName === modelName
                && createdTags.includes(ACCEPTANCE_TEST_TAG)
                && markerPersisted
                && imagePersisted;
        };

        showProgress('Checking the configured test note…');
        try {
            const eligibility = await canAddNotes(url, [{ deckName, modelName, fields }]);
            if (!eligibility[0]) {
                const escapedDeck = deckName.replace(/(["\\])/g, '\\$1');
                const existingIds = await findNotes(url, `tag:${ACCEPTANCE_TEST_TAG} deck:"${escapedDeck}"`);
                const existingNotes = existingIds.length ? await notesInfo(url, existingIds) : [];
                closeDialog();
                if (existingNotes.some(verifyNote)) {
                    showAlert(
                        'Anki test passed',
                        `The existing synthetic note was verified in “${deckName}”. No second note was added.`,
                    );
                } else {
                    showAlert(
                        'Synthetic note already exists',
                        'No note was added, but the duplicate could not be verified as this tagged acceptance note.',
                    );
                }
                return;
            }

            const noteId = await addNote(
                url,
                deckName,
                modelName,
                fields,
                [ACCEPTANCE_TEST_TAG],
                picture,
                undefined,
                { allowDuplicate: false, duplicateScope: 'deck', recordMiningEvent: false },
            );
            if (typeof noteId !== 'number' || noteId <= 0) {
                throw new Error('Anki did not return a valid note identifier.');
            }

            const [created] = await notesInfo(url, [noteId]);
            if (!verifyNote(created)) {
                throw new Error(
                    'The note was created, but its returned fields, tag, model, or media did not match the preview.',
                );
            }

            closeDialog();
            showAlert(
                'Anki test passed',
                `One synthetic note was created and verified in “${deckName}”. It is tagged ${ACCEPTANCE_TEST_TAG}.`,
            );
        } catch (error) {
            closeDialog();
            showAlert('Anki test failed', error instanceof Error ? error.message : String(error));
        }
    };

    const previewSyntheticAnkiTest = () => {
        if (!localSettings.ankiDeck || !localSettings.ankiModel) {
            showAlert('Anki test unavailable', 'Select a target deck and card type first.');
            return;
        }
        const mappedFields = currentModelFields
            .filter((field) => localSettings.ankiFieldMap?.[field] && localSettings.ankiFieldMap?.[field] !== 'None')
            .map((field) => `${field} ← ${localSettings.ankiFieldMap?.[field]}`);
        showDialog({
            type: 'confirm',
            title: 'Create synthetic Anki test note?',
            message: (
                <div style={{ textAlign: 'left' }}>
                    <p>This creates exactly one synthetic note and verifies it through the on-device bridge.</p>
                    <p>
                        <strong>Deck:</strong> {localSettings.ankiDeck}<br />
                        <strong>Card type:</strong> {localSettings.ankiModel}
                    </p>
                    <p style={{ whiteSpace: 'pre-line' }}>
                        <strong>Mapped fields:</strong><br />
                        {mappedFields.length
                            ? mappedFields.join('\n')
                            : 'None (the first field receives only a synthetic marker)'}
                    </p>
                    <p><strong>Tag:</strong> {ACCEPTANCE_TEST_TAG}</p>
                    <p>No manga image, OCR text, or reading activity is used.</p>
                </div>
            ),
            confirmText: 'Create and verify',
            cancelText: 'Cancel',
            onConfirm: runSyntheticAnkiTest,
        });
    };

    const resetToDefaults = () => {
        showConfirm('Reset?', 'Revert to defaults?', () => {
            const isMobile = /Android|webOS|iPhone|iPad|iPod|BlackBerry|IEMobile|Opera Mini/i.test(navigator.userAgent);
            const next = { ...DEFAULT_SETTINGS, mobileMode: isMobile };
            setLocalSettings(next);
            persistSettings(next);
            closeDialog();
        });
    };

    const purgeCache = () => {
        showConfirm('Purge Cache?', 'Delete server cache?', async () => {
            try {
                showProgress('Purging...');
                await apiRequest(`/api/ocr/purge-cache`, { method: 'POST' });
                closeDialog(); 
                showAlert('Success', 'Cache deleted.');
            } catch (e) { closeDialog(); showAlert('Error', 'Failed.'); }
        });
    };

    const resetYomitanDB = () => {
        showConfirm('Reset DB?', 'Delete all dictionaries?', async () => {
            try {
                showProgress('Resetting...');
                const res = await apiRequest<{status: string}>(`/api/yomitan/reset`, {
                    method: 'POST',
                    body: { language: localSettings.yomitanLanguage || 'japanese' },
                });
                if (res.status === 'ok') {
                    closeDialog(); 
                    showAlert('Success', 'Reset complete.');
                    setDictManagerKey(p => p + 1);
                } else throw new Error();
            } catch (e) { closeDialog(); showAlert('Error', 'Failed.'); }
        });
    };

    const handleImportClick = () => fileInputRef.current?.click();

    const handleFileChange = async (e: React.ChangeEvent<HTMLInputElement>) => {
        const files = e.target.files ? Array.from(e.target.files) : [];
        if (!files.length) {
            return;
        }

        let successCount = 0;
        let failCount = 0;
        const failureMessages: string[] = [];

        for (let i = 0; i < files.length; i += 1) {
            const file = files[i];
            const formData = new FormData();
            formData.append('file', file);
            try {
                showProgress(`Importing ${i + 1}/${files.length}...`);
                const res = await fetch('/api/yomitan/import', { method: 'POST', body: formData });
                const json = await res.json();
                if (json.status === 'ok') {
                    successCount += 1;
                } else {
                    failCount += 1;
                    const message = json.message ? String(json.message) : 'Unknown error.';
                    failureMessages.push(`${file.name}: ${message}`);
                }
            } catch (err) {
                failCount += 1;
                failureMessages.push(`${file.name}: ${String(err)}`);
            }
        }

        closeDialog();
        if (successCount > 0) {
            setDictManagerKey((p) => p + 1);
        }
        if (failCount === 0) {
            showAlert('Success', `Imported ${successCount} dictionaries.`);
        } else if (successCount === 0) {
            showAlert('Failed', failureMessages.join('\n') || 'No dictionaries were imported.');
        } else {
            const detail = failureMessages.length
                ? `Failed dictionaries:\n${failureMessages.join('\n')}`
                : 'Some dictionaries failed to import.';
            showAlert(
                'Partial Import',
                `Imported ${successCount} dictionary${successCount === 1 ? '' : 'ies'}.
Failed ${failCount}.
${detail}`,
            );
        }

        if (fileInputRef.current) {
            fileInputRef.current.value = '';
        }
    };

    const isNativeApp = typeof navigator !== 'undefined'
        && (navigator.userAgent.includes('MangatanNative') || navigator.userAgent.includes('ManatanNative'));
    const isiOS = typeof navigator !== 'undefined' && /iPhone|iPad|iPod/i.test(navigator.userAgent);

    const showDicts = isNativeApp || localSettings.enableYomitan;

    return (
        <div
            className="ocr-modal-overlay"
            onClick={(event) => {
                if (event.target === event.currentTarget) {
                    onClose();
                }
            }}
        >
            <div className="ocr-modal settings-modal" onClick={(e) => e.stopPropagation()}>
                <div className="ocr-modal-content">
                    <h2>Settings</h2>
                    <input type="file" ref={fileInputRef} style={{ display: 'none' }} accept=".zip" multiple onChange={handleFileChange} />

                    {/* --- POPUP DICTIONARY SECTION --- */}
                    <h3>Popup Dictionary</h3>
                    <div style={sectionBoxStyle}>
                        <label style={checkboxLabelStyle}>
                            <input 
                                type="checkbox" 
                                checked={localSettings.enableYomitan} 
                                onChange={e => handleChange('enableYomitan', e.target.checked)} 
                                style={checkboxInputStyle} 
                            />
                            <div>
                                Enable Popup Dictionary
                                <div style={{ opacity: 0.6, fontSize: '0.85em' }}>
                                    Shows dictionary popups on hover or tap.
                                </div>
                            </div>
                        </label>

                        <div style={{ marginTop: '12px', display: 'flex', flexDirection: 'column', gap: '5px' }}>
                            <label htmlFor="yomitanLanguage" style={{fontSize: '0.9em', color: '#ccc'}}>Dictionary Language</label>
                            <select
                                id="yomitanLanguage"
                                value={localSettings.yomitanLanguage || 'japanese'}
                                onChange={(e) => handleChange('yomitanLanguage', e.target.value)}
                                style={{ padding: '6px', borderRadius: '4px', border: '1px solid #444', background: '#222', color: 'white' }}
                            >
                                <option value="japanese">Japanese</option>
                                <option value="english">English</option>
                                <option value="chinese">Chinese</option>
                                <option value="korean">Korean</option>
                                <option value="arabic">Arabic</option>
                                <option value="spanish">Spanish</option>
                                <option value="french">French</option>
                                <option value="german">German</option>
                                <option value="portuguese">Portuguese</option>
                                <option value="bulgarian">Bulgarian</option>
                                <option value="cantonese">Cantonese</option>
                                <option value="czech">Czech</option>
                                <option value="danish">Danish</option>
                                <option value="estonian">Estonian</option>
                                <option value="finnish">Finnish</option>
                                <option value="georgian">Georgian</option>
                                <option value="greek">Greek</option>
                                <option value="hebrew">Hebrew</option>
                                <option value="hindi">Hindi</option>
                                <option value="hungarian">Hungarian</option>
                                <option value="indonesian">Indonesian</option>
                                <option value="italian">Italian</option>
                                <option value="kannada">Kannada</option>
                                <option value="khmer">Khmer</option>
                                <option value="lao">Lao</option>
                                <option value="latin">Latin</option>
                                <option value="latvian">Latvian</option>
                                <option value="maltese">Maltese</option>
                                <option value="mongolian">Mongolian</option>
                                <option value="dutch">Dutch</option>
                                <option value="norwegian">Norwegian</option>
                                <option value="persian">Persian</option>
                                <option value="polish">Polish</option>
                                <option value="romanian">Romanian</option>
                                <option value="russian">Russian</option>
                                <option value="swedish">Swedish</option>
                                <option value="tagalog">Tagalog</option>
                                <option value="thai">Thai</option>
                                <option value="turkish">Turkish</option>
                                <option value="ukrainian">Ukrainian</option>
                                <option value="vietnamese">Vietnamese</option>
                                <option value="welsh">Welsh</option>
                            </select>
                            <div style={{ fontSize: '0.85em', color: '#aaa' }}>
                                Used when installing or resetting default dictionaries.
                            </div>
                            <div style={{ fontSize: '0.85em', color: '#aaa' }}>
                                Import dictionaries explicitly below; reading does not require completing a setup wizard.
                            </div>
                        </div>

                        <div style={{ marginTop: '12px', display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '12px' }}>
                            <div style={{ display: 'flex', flexDirection: 'column', gap: '6px' }}>
                                <label htmlFor="yomitanPopupWidthValue">Popup Width (px)</label>
                                <div style={inlineInputWrapperStyle}>
                                    <input
                                        id="yomitanPopupWidthValue"
                                        type="number"
                                        step="10"
                                        min="280"
                                        max="1920"
                                        value={yomitanPopupWidthValue}
                                        onChange={(e) => handleChange(
                                            'yomitanPopupWidthPx',
                                            parseInt(e.target.value, 10),
                                        )}
                                        style={inlineInputCompactStyle}
                                    />
                                    <div style={inlineInputActionsStyle}>
                                        <IconButton
                                            size="small"
                                            onClick={resetYomitanPopupWidth}
                                            aria-label="Reset popup dictionary width"
                                            style={{ padding: 4 }}
                                        >
                                            <RestartAltIcon fontSize="small" />
                                        </IconButton>
                                    </div>
                                </div>
                            </div>
                            <div style={{ display: 'flex', flexDirection: 'column', gap: '6px' }}>
                                <label htmlFor="yomitanPopupHeightValue">Popup Height (px)</label>
                                <div style={inlineInputWrapperStyle}>
                                    <input
                                        id="yomitanPopupHeightValue"
                                        type="number"
                                        step="10"
                                        min="200"
                                        max="1080"
                                        value={yomitanPopupHeightValue}
                                        onChange={(e) => handleChange(
                                            'yomitanPopupHeightPx',
                                            parseInt(e.target.value, 10),
                                        )}
                                        style={inlineInputCompactStyle}
                                    />
                                    <div style={inlineInputActionsStyle}>
                                        <IconButton
                                            size="small"
                                            onClick={resetYomitanPopupHeight}
                                            aria-label="Reset popup dictionary height"
                                            style={{ padding: 4 }}
                                        >
                                            <RestartAltIcon fontSize="small" />
                                        </IconButton>
                                    </div>
                                </div>
                            </div>
                        </div>
                        <div style={{ marginTop: '12px', display: 'flex', flexDirection: 'column', gap: '6px' }}>
                            <label htmlFor="yomitanPopupScaleValue">Popup Scale (%)</label>
                            <div style={inlineInputWrapperStyle}>
                                <input
                                    id="yomitanPopupScaleValue"
                                    type="number"
                                    step="5"
                                    min="50"
                                    max="200"
                                    value={yomitanPopupScaleValue}
                                    onChange={(e) => handleChange(
                                        'yomitanPopupScalePercent',
                                        parseInt(e.target.value, 10),
                                    )}
                                    style={inlineInputCompactStyle}
                                />
                                <div style={inlineInputActionsStyle}>
                                    <IconButton
                                        size="small"
                                        onClick={resetYomitanPopupScale}
                                        aria-label="Reset popup dictionary scale"
                                        style={{ padding: 4 }}
                                    >
                                        <RestartAltIcon fontSize="small" />
                                    </IconButton>
                                </div>
                            </div>
                        </div>
                        <div style={{ fontSize: '0.85em', color: '#aaa', marginTop: '6px' }}>
                            Adjust the popup dictionary size and scale.
                        </div>
                        <div style={{ marginTop: '12px', display: 'flex', flexDirection: 'column', gap: '6px' }}>
                            <label htmlFor="yomitanPopupTheme">Popup Theme</label>
                            <select
                                id="yomitanPopupTheme"
                                value={yomitanPopupThemeValue}
                                onChange={(e) => handleChange('yomitanPopupTheme', e.target.value)}
                                style={{ ...inlineInputCompactStyle, padding: '6px 8px' }}
                            >
                                <option value="dark">Dark</option>
                                <option value="light">Light</option>
                            </select>
                        </div>

                        <div style={{ marginTop: '12px', display: 'flex', flexDirection: 'column', gap: '6px' }}>
                            <label htmlFor="yomitanPopupCustomCss">Custom Popup CSS</label>
                            <div style={inlineInputWrapperStyle}>
                                <textarea
                                    id="yomitanPopupCustomCss"
                                    rows={5}
                                    value={localSettings.yomitanPopupCustomCss ?? ''}
                                    onChange={(e) => handleChange('yomitanPopupCustomCss', e.target.value)}
                                    placeholder={"color: #f5f5f5;\nbackground: rgba(10,10,10,0.96);\n\n/* or full CSS: */\n.yomitan-popup .entry { font-size: 15px; }"}
                                    style={{
                                        ...inlineInputStyle,
                                        width: '100%',
                                        minHeight: '110px',
                                        resize: 'vertical',
                                        fontFamily: 'monospace',
                                        lineHeight: 1.4,
                                    }}
                                />
                                <div style={inlineInputActionsStyle}>
                                    <IconButton
                                        size="small"
                                        onClick={resetYomitanPopupCustomCss}
                                        aria-label="Reset popup dictionary custom CSS"
                                        style={{ padding: 4 }}
                                    >
                                        <RestartAltIcon fontSize="small" />
                                    </IconButton>
                                </div>
                            </div>
                            <div style={{ fontSize: '0.85em', color: '#aaa' }}>
                                Applies to <code>.yomitan-popup</code>. Enter CSS declarations or full CSS rules.
                            </div>
                        </div>

                        {/* Pitch Accent Display Settings */}
                        <div style={{ marginTop: '16px', paddingTop: '12px', borderTop: '1px solid #333' }}>
                            <div style={{ fontSize: '0.9em', fontWeight: 'bold', marginBottom: '10px', color: '#ccc' }}>
                                Pitch Accent Display
                            </div>
                            <div style={{ display: 'flex', flexDirection: 'column', gap: '8px' }}>
                                <label style={{ display: 'flex', alignItems: 'center', gap: '8px', cursor: 'pointer' }}>
                                    <input
                                        type="checkbox"
                                        checked={localSettings.yomitanShowPitchText ?? true}
                                        onChange={(e) => handleChange('yomitanShowPitchText', e.target.checked)}
                                        style={{ width: '16px', height: '16px' }}
                                    />
                                    <span style={{ fontSize: '0.85em', color: '#aaa' }}>Show pitch text (character high/low)</span>
                                </label>
                                <label style={{ display: 'flex', alignItems: 'center', gap: '8px', cursor: 'pointer' }}>
                                    <input
                                        type="checkbox"
                                        checked={localSettings.yomitanShowPitchNotation ?? true}
                                        onChange={(e) => handleChange('yomitanShowPitchNotation', e.target.checked)}
                                        style={{ width: '16px', height: '16px' }}
                                    />
                                    <span style={{ fontSize: '0.85em', color: '#aaa' }}>Show pitch notation ([0], [1], etc.)</span>
                                </label>
                                <label style={{ display: 'flex', alignItems: 'center', gap: '8px', cursor: 'pointer' }}>
                                    <input
                                        type="checkbox"
                                        checked={localSettings.yomitanShowPitchGraph ?? false}
                                        onChange={(e) => handleChange('yomitanShowPitchGraph', e.target.checked)}
                                        style={{ width: '16px', height: '16px' }}
                                    />
                                    <span style={{ fontSize: '0.85em', color: '#aaa' }}>Show pitch graph (SVG diagram)</span>
                                </label>
                            </div>
                        </div>
                        
                        <div style={{
                            maxHeight: showDicts ? '800px' : '0px',
                            opacity: showDicts ? 1 : 0,
                            overflow: 'hidden',
                            transition: 'max-height 0.4s cubic-bezier(0.4, 0, 0.2, 1), opacity 0.3s ease-in-out',
                        }}>
                             <div style={{ paddingTop: '15px' }}>
                                 {/* Result Grouping Dropdown */}
                                 <div style={{ marginBottom: '15px', display: 'flex', flexDirection: 'column', gap: '5px' }}>
                                     <label htmlFor="groupingMode" style={{fontSize: '0.9em', color: '#ccc'}}>Result Grouping</label>
                                      <select
                                          id="groupingMode"
                                          value={localSettings.resultGroupingMode || 'grouped'}
                                          onChange={(e) => handleChange('resultGroupingMode', e.target.value)}
                                          style={{ padding: '6px', borderRadius: '4px', border: '1px solid #444', background: '#222', color: 'white' }}
                                      >
                                          <option value="grouped">Group by Term</option>
                                          <option value="flat">No Grouping</option>
                                      </select>
                                       <div style={{ fontSize: '0.85em', color: '#aaa' }}>
                                           Group results by term or list every entry.
                                       </div>
                                    </div>
                                    {/* Lookup Navigation Mode */}
                                    <div style={{ marginBottom: '15px', display: 'flex', flexDirection: 'column', gap: '5px' }}>
                                        <label htmlFor="lookupNavMode" style={{fontSize: '0.9em', color: '#ccc'}}>Lookup Navigation</label>
                                        <select
                                            id="lookupNavMode"
                                            value={localSettings.yomitanLookupNavigationMode || 'stacked'}
                                            onChange={(e) => handleChange('yomitanLookupNavigationMode', e.target.value)}
                                            style={{ padding: '6px', borderRadius: '4px', border: '1px solid #444', background: '#222', color: 'white' }}
                                        >
                                            <option value="stacked">Stacked (Back button)</option>
                                            <option value="tabs">Tabs (Browser-like)</option>
                                        </select>
                                        <div style={{ fontSize: '0.85em', color: '#aaa' }}>
                                            How to navigate between lookups in the popup.
                                        </div>
                                    </div>
                                    {/* Max History */}
                                    <div style={{ marginBottom: '15px', display: 'flex', flexDirection: 'column', gap: '5px' }}>
                                        <label htmlFor="maxHistory" style={{fontSize: '0.9em', color: '#ccc'}}>Max Lookup History</label>
                                        <input
                                            id="maxHistory"
                                            type="number"
                                            min="1"
                                            max="50"
                                            value={localSettings.yomitanLookupMaxHistory ?? 10}
                                            onChange={(e) => handleChange('yomitanLookupMaxHistory', parseInt(e.target.value) || 10)}
                                            style={{ padding: '6px', borderRadius: '4px', border: '1px solid #444', background: '#222', color: 'white' }}
                                        />
                                        <div style={{ fontSize: '0.85em', color: '#aaa' }}>
                                            Maximum number of lookups to keep in history.
                                        </div>
                                    </div>
                                    <label style={checkboxLabelStyle}>
                                        <input
                                            type="checkbox"
                                            checked={localSettings.yomitanShowKanjiInNormalLookup}
                                            onChange={(e) => handleChange('yomitanShowKanjiInNormalLookup', e.target.checked)}
                                            style={checkboxInputStyle}
                                        />
                                        <div>
                                            Show Kanji in Normal Lookup
                                            <div style={{ opacity: 0.6, fontSize: '0.85em' }}>
                                                Always shows kanji results at the bottom of the popup.
                                            </div>
                                        </div>
                                    </label>

                                    <label style={checkboxLabelStyle}>
                                        <input
                                            type="checkbox"
                                            checked={localSettings.autoPlayWordAudio}
                                            onChange={(e) => handleChange('autoPlayWordAudio', e.target.checked)}
                                            style={checkboxInputStyle}
                                        />
                                        <div>
                                            Auto-play Word Audio
                                            <div style={{ opacity: 0.6, fontSize: '0.85em' }}>
                                                Plays word audio automatically when search results appear.
                                            </div>
                                        </div>
                                    </label>

                                <label style={checkboxLabelStyle}>
                                    <input
                                        type="checkbox"
                                        checked={localSettings.showHarmonicMeanFreq}
                                        onChange={e => handleChange('showHarmonicMeanFreq', e.target.checked)}
                                        style={checkboxInputStyle}
                                    />
                                    <div>
                                        Show Harmonic Mean Frequency
                                        <div style={{ opacity: 0.6, fontSize: '0.85em' }}>
                                            Displays a single harmonic mean value instead of individual frequency dictionaries.
                                        </div>
                                    </div>
                                </label>

                                <DictionaryManager key={dictManagerKey} onImportClick={handleImportClick} />
                            </div>
                        </div>
                    </div>

                    {/* --- ANKI CONNECT SECTION --- */}
                    {!isiOS && (
                        <>
                        <h3>AnkiConnect Integration</h3>
                        <div style={sectionBoxStyle}>
                            <label style={checkboxLabelStyle}>
                                <input 
                                    type="checkbox" 
                                    checked={localSettings.ankiConnectEnabled ?? false} 
                                    onChange={(e) => handleChange('ankiConnectEnabled', e.target.checked)} 
                                    style={checkboxInputStyle} 
                                />
                                <div>
                                    Enable AnkiConnect
                                    <div style={{ opacity: 0.5, fontSize: '0.9em' }}>
                                        {localSettings.enableYomitan 
                                            ? "Automatically add cards via the Popup Dictionary" 
                                            : "Right-click (desktop) or hold (mobile) to update the last card (useful for third-party dictionaries)"
                                        }
                                    </div>
                                </div>
                            </label>

                            {/* Collapsible Anki Settings */}
                            <div style={{
                                maxHeight: localSettings.ankiConnectEnabled ? 'none' : '0px',
                                opacity: localSettings.ankiConnectEnabled ? 1 : 0,
                                overflow: 'hidden',
                                transition: 'max-height 0.4s cubic-bezier(0.4, 0, 0.2, 1), opacity 0.3s ease-in-out',
                            }}>
                                <div style={{ marginTop: '10px', paddingLeft: '5px' }}>
                                    {/* Connection Status & URL */}
                                    <div style={{ marginBottom: '15px', display: 'flex', alignItems: 'center', justifyContent: 'space-between', flexWrap: 'wrap', gap: '10px' }}>
                                        <div style={{display:'flex', alignItems:'center'}}>
                                            <span style={statusDotStyle(ankiStatus === 'connected')}></span>
                                            <span style={{color: ankiStatus === 'connected' ? '#2ecc71' : '#e74c3c', fontWeight: 'bold'}}>
                                                {ankiStatus === 'connected' ? 'Connected' : ankiStatus === 'loading' ? 'Connecting...' : 'Not Connected'}
                                            </span>
                                        </div>
                                        <button 
                                            onClick={fetchAnkiData}
                                            disabled={ankiStatus === 'loading'}
                                            style={{
                                                padding: '5px 10px', fontSize: '0.85em', cursor: 'pointer',
                                                backgroundColor: 'rgba(255,255,255,0.1)', border: '1px solid rgba(255,255,255,0.2)', color: 'white', borderRadius: '4px'
                                            }}
                                        >
                                            Retry Connection
                                        </button>
                                    </div>

                                    <div className="grid">
                                        <label htmlFor="ankiUrl">AnkiConnect URL</label>
                                        <input 
                                            id="ankiUrl" 
                                            value={localSettings.ankiConnectUrl ?? 'http://127.0.0.1:18765'}
                                            onChange={(e) => handleChange('ankiConnectUrl', e.target.value)} 
                                            placeholder="http://127.0.0.1:18765"
                                        />
                                        <div style={{ gridColumn: '1 / -1', fontSize: '0.85em', color: '#aaa' }}>
                                            Address where AnkiConnect is listening.
                                        </div>
                                        
                                        <label htmlFor="ankiQuality">Image Quality</label>
                                        <input 
                                            id="ankiQuality" 
                                            type="number"
                                            step="0.01"
                                            min="0"
                                            max="1"
                                            value={localSettings.ankiImageQuality ?? 0.92} 
                                            onChange={(e) => handleChange('ankiImageQuality', parseFloat(e.target.value))} 
                                            placeholder="0.92"
                                        />
                                        <div style={{ gridColumn: '1 / -1', fontSize: '0.85em', color: '#aaa' }}>
                                            Image compression quality for screenshots sent to Anki (0-1).
                                        </div>

                                        {/* Downscale settings */}
                                        <div style={{ gridColumn: '1 / -1', display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '12px' }}>
                                            <div style={{ display: 'flex', flexDirection: 'column', gap: '6px' }}>
                                                <label htmlFor="ankiDownscaleMaxWidth">Max Image Width (px)</label>
                                                    <select 
                                                        id="ankiDownscaleMaxWidth"
                                                        value={localSettings.ankiDownscaleMaxWidth ?? ''}
                                                        onChange={(e) => handleChange('ankiDownscaleMaxWidth', e.target.value ? parseInt(e.target.value, 10) : undefined)}
                                                    >
                                                        {DOWNSCALE_OPTIONS.map(opt => (
                                                            <option key={`width-${opt.value}`} value={opt.value}>
                                                                {opt.label}
                                                            </option>
                                                        ))}
                                                    </select>
                                            </div>
                                            <div style={{ display: 'flex', flexDirection: 'column', gap: '6px' }}>
                                                <label htmlFor="ankiDownscaleMaxHeight">Max Image Height (px)</label>
                                                    <select 
                                                        id="ankiDownscaleMaxHeight"
                                                        value={localSettings.ankiDownscaleMaxHeight ?? ''}
                                                        onChange={(e) => handleChange('ankiDownscaleMaxHeight', e.target.value ? parseInt(e.target.value, 10) : undefined)}
                                                    >
                                                        {DOWNSCALE_OPTIONS.map(opt => (
                                                            <option key={`height-${opt.value}`} value={opt.value}>
                                                                {opt.label}
                                                            </option>
                                                        ))}
                                                    </select>
                                            </div>
                                            <div style={{ gridColumn: '1 / -1', fontSize: '0.85em', color: '#aaa' }}>
                                                If the screenshot exceeds the max width or height, it will be downscaled.
                                            </div>
                                        </div>
                                    </div>

                                    <div style={{ marginBottom: '15px', marginTop: '10px', padding: '10px', backgroundColor: 'rgba(255,255,255,0.05)', borderRadius: '6px', border: '1px solid rgba(255,255,255,0.1)' }}>
                                        <label style={{ ...checkboxLabelStyle, marginBottom: '0' }}>
                                            <input 
                                                type="checkbox" 
                                                checked={localSettings.ankiEnableCropper ?? false} 
                                                onChange={(e) => handleChange('ankiEnableCropper', e.target.checked)} 
                                                style={checkboxInputStyle} 
                                            />
                                            <div>
                                                Enable Image Cropper
                                                <div style={{ opacity: 0.5, fontSize: '0.9em' }}>
                                                    Allows you to crop the image before sending to Anki
                                                </div>
                                            </div>
                                        </label>
                                    </div>

                                    {!localSettings.enableYomitan && (
                                        <div style={{ marginBottom: '15px', padding: '10px', backgroundColor: 'rgba(255,255,255,0.05)', borderRadius: '6px', border: '1px solid rgba(255,255,255,0.1)' }}>
                                            <label style={{ ...checkboxLabelStyle, marginBottom: '0' }}>
                                                <input
                                                    type="checkbox"
                                                    checked={localSettings.skipAnkiUpdateConfirm ?? false}
                                                    onChange={(e) => handleChange('skipAnkiUpdateConfirm', e.target.checked)}
                                                    style={checkboxInputStyle}
                                                />
                                                <div>
                                                    Skip Update Anki Card confirmation
                                                    <div style={{ opacity: 0.5, fontSize: '0.9em' }}>
                                                        Updates the last card immediately when you use the right-click action.
                                                    </div>
                                                </div>
                                            </label>
                                        </div>
                                    )}

                                    {localSettings.enableYomitan && (
                                        <div style={{ marginBottom: '15px', padding: '10px', backgroundColor: 'rgba(255,255,255,0.05)', borderRadius: '6px', border: '1px solid rgba(255,255,255,0.1)' }}>
                                            <label style={{ ...checkboxLabelStyle }}>
                                                <input 
                                                    type="checkbox" 
                                                    checked={localSettings.ankiCheckDuplicates ?? true} 
                                                    onChange={(e) => handleChange('ankiCheckDuplicates', e.target.checked)} 
                                                    style={checkboxInputStyle} 
                                                />
                                                <div>
                                                    Check for Duplicates
                                                    <div style={{ opacity: 0.5, fontSize: '0.9em' }}>
                                                        Checks if the word already exists in Anki
                                                    </div>
                                                </div>
                                            </label>

                                            {localSettings.ankiCheckDuplicates && (
                                                <div style={{ marginTop: '10px', display: 'flex', flexDirection: 'column', gap: '8px' }}>
                                                    <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                                                        <span style={{ fontSize: '0.9em', color: '#ccc' }}>Duplicate Action</span>
                                                        <select
                                                            value={localSettings.ankiDuplicateAction || 'prevent'}
                                                            onChange={e => handleChange('ankiDuplicateAction', e.target.value)}
                                                            style={{ padding: '4px', borderRadius: '4px', border: '1px solid #444', background: '#222', color: 'white', minWidth: '120px' }}
                                                        >
                                                            <option value="prevent">Prevent</option>
                                                            <option value="overwrite">Overwrite</option>
                                                            <option value="add">Add Anyway</option>
                                                        </select>
                                                    </div>

                                                    <button
                                                        onClick={() => setShowAdvancedAnki(!showAdvancedAnki)}
                                                        style={{ background: 'none', border: 'none', color: '#7cc8ff', cursor: 'pointer', fontSize: '0.8em', padding: 0, textAlign: 'left', width: 'fit-content' }}
                                                    >
                                                        {showAdvancedAnki ? 'Less...' : 'More...'}
                                                    </button>

                                                    {showAdvancedAnki && (
                                                        <div style={{ display: 'flex', flexDirection: 'column', gap: '8px', paddingLeft: '10px', borderLeft: '1px solid rgba(255,255,255,0.1)' }}>
                                                            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                                                                <span style={{ fontSize: '0.85em', color: '#aaa' }}>Check Scope</span>
                                                                <select
                                                                    value={localSettings.ankiDuplicateScope || 'deck'}
                                                                    onChange={e => handleChange('ankiDuplicateScope', e.target.value)}
                                                                    style={{ padding: '4px', borderRadius: '4px', border: '1px solid #444', background: '#222', color: 'white', minWidth: '120px' }}
                                                                >
                                                                    <option value="deck">Current Deck</option>
                                                                    <option value="deck-root">Deck Root</option>
                                                                    <option value="collection">Collection</option>
                                                                </select>
                                                            </div>
                                                            <label style={{ ...checkboxLabelStyle, marginBottom: 0 }}>
                                                                <input
                                                                    type="checkbox"
                                                                    checked={localSettings.ankiCheckDuplicatesAllModels ?? false}
                                                                    onChange={e => handleChange('ankiCheckDuplicatesAllModels', e.target.checked)}
                                                                    style={checkboxInputStyle}
                                                                />
                                                                <div style={{ fontSize: '0.85em', color: '#aaa' }}>
                                                                    Check across all models
                                                                </div>
                                                            </label>
                                                        </div>
                                                    )}
                                                </div>
                                            )}
                                        </div>
                                    )}

                                    {/* Deck & Model Selection */}
                                    {ankiStatus === 'connected' && (
                                        <>
                                            <div className="grid" style={{marginTop: '20px', borderTop: '1px solid rgba(255,255,255,0.1)', paddingTop: '15px'}}>
                                                <label htmlFor="ankiDeck">Target Deck</label>
                                                <select 
                                                    id="ankiDeck"
                                                    value={localSettings.ankiDeck || ''}
                                                    onChange={e => handleChange('ankiDeck', e.target.value)}
                                                >
                                                    <option value="">Select a Deck...</option>
                                                    {ankiDecks.map(d => <option key={d} value={d}>{d}</option>)}
                                                </select>
                                                <div style={{ gridColumn: '1 / -1', fontSize: '0.85em', color: '#aaa' }}>
                                                    Deck where new cards will be added.
                                                </div>

                                                <label htmlFor="ankiModel">Card Type</label>
                                                <select 
                                                    id="ankiModel"
                                                    value={localSettings.ankiModel || ''}
                                                    onChange={e => {
                                                        const newVal = e.target.value;
                                                        setLocalSettings(prev => {
                                                            const next = {
                                                                ...prev,
                                                                ankiModel: newVal,
                                                                ankiFieldMap: {},
                                                            };
                                                            persistSettings(next);
                                                            return next;
                                                        });
                                                    }}
                                                >
                                                    <option value="">Select Card Type...</option>
                                                    {ankiModels.map(m => <option key={m} value={m}>{m}</option>)}
                                                </select>
                                                <div style={{ gridColumn: '1 / -1', fontSize: '0.85em', color: '#aaa' }}>
                                                    Note type used when creating cards.
                                                </div>
                                            </div>

                                            {/* Field Mapping Section */}
                                            {localSettings.ankiModel && currentModelFields.length > 0 && (
                                                <div style={{ marginTop: '20px' }}>
                                                    <h4 style={{marginBottom: '10px', color: '#ddd'}}>Field Mapping</h4>
                                                    <div style={{ fontSize: '0.85em', color: '#aaa', marginBottom: '10px' }}>
                                                        Map OCR and dictionary content to your Anki fields.
                                                    </div>
                                                    
                                                    {/* If built-in dictionary is enabled, show full table mapping */}
                                                    {localSettings.enableYomitan ? (
                                                        <div style={{overflowX: 'auto'}}>
                                                            <table style={{width: '100%', borderCollapse: 'collapse', fontSize: '0.9em'}}>
                                                                <thead>
                                                                    <tr style={{borderBottom: '1px solid rgba(255,255,255,0.2)'}}>
                                                                        <th style={{textAlign: 'left', padding: '8px', color: '#aaa'}}>Anki Field</th>
                                                                        <th style={{textAlign: 'left', padding: '8px', color: '#aaa'}}>Content</th>
                                                                        {localSettings.ankiDuplicateAction === 'overwrite' && (
                                                                            <th style={{textAlign: 'left', padding: '8px', color: '#aaa'}}>Update Mode</th>
                                                                        )}
                                                                    </tr>
                                                                </thead>
                                                                <tbody>
                                                                    {currentModelFields.map(field => (
                                                                        <tr key={field} style={{borderBottom: '1px solid rgba(255,255,255,0.1)'}}>
                                                                            <td style={{padding: '8px'}}>{field}</td>
                                                                            <td style={{padding: '8px'}}>
                                                                                <select
                                                                                    style={{width: '100%', padding: '4px', borderRadius: '4px', border: '1px solid #444', background: '#222', color: 'white'}}
                                                                                    value={(localSettings.ankiFieldMap as any)?.[field] || 'None'}
                                                                                    onChange={e => handleFieldMapChange(field, e.target.value)}
                                                                                >
                                                                                    {mappingOptions.map((opt) => (
                                                                                        <option key={opt.value} value={opt.value}>
                                                                                            {opt.label}
                                                                                        </option>
                                                                                    ))}
                                                                                </select>
                                                                            </td>
                                                                            {localSettings.ankiDuplicateAction === 'overwrite' && (
                                                                                <td style={{padding: '8px'}}>
                                                                                    <select
                                                                                        style={{width: '100%', padding: '4px', borderRadius: '4px', border: '1px solid #444', background: '#222', color: 'white'}}
                                                                                        value={localSettings.ankiFieldUpdateModes?.[field] || 'overwrite'}
                                                                                        onChange={e => handleUpdateModeChange(field, e.target.value)}
                                                                                    >
                                                                                        {updateModes.map(m => (
                                                                                            <option key={m.value} value={m.value}>{m.label}</option>
                                                                                        ))}
                                                                                    </select>
                                                                                </td>
                                                                            )}
                                                                        </tr>
                                                                    ))}
                                                                </tbody>
                                                            </table>
                                                        </div>
                                                    ) : (
                                                        // If built-in dictionary is disabled, show simple dropdowns for Sentence/Image
                                                        <div className="grid">
                                                            <label>Sentence Field</label>
                                                            <select
                                                                value={getFieldForContent('Sentence')}
                                                                onChange={(e) => handleContentToFieldChange('Sentence', e.target.value)}
                                                            >
                                                                <option value="">(None)</option>
                                                                {currentModelFields.map(f => <option key={f} value={f}>{f}</option>)}
                                                            </select>
                                                            <div style={{ gridColumn: '1 / -1', fontSize: '0.85em', color: '#aaa' }}>
                                                                Field where the selected sentence will be stored.
                                                            </div>

                                                            <label>Image Field</label>
                                                            <select
                                                                value={getFieldForContent('Image')}
                                                                onChange={(e) => handleContentToFieldChange('Image', e.target.value)}
                                                            >
                                                                <option value="">(None)</option>
                                                                {currentModelFields.map(f => <option key={f} value={f}>{f}</option>)}
                                                            </select>
                                                            <div style={{ gridColumn: '1 / -1', fontSize: '0.85em', color: '#aaa' }}>
                                                                Field where the screenshot image will be stored.
                                                            </div>

                                                            <label>Sentence Audio Field</label>
                                                            <select
                                                                value={getFieldForContent('Sentence Audio')}
                                                                onChange={(e) => handleContentToFieldChange('Sentence Audio', e.target.value)}
                                                            >
                                                                <option value="">(None)</option>
                                                                {currentModelFields.map(f => <option key={f} value={f}>{f}</option>)}
                                                            </select>
                                                            <div style={{ gridColumn: '1 / -1', fontSize: '0.85em', color: '#aaa' }}>
                                                                Field where the sentence audio will be stored.
                                                            </div>
                                                        </div>
                                                    )}

                                                        {/* FREQUENCY MODE DROPDOWN - ADD THIS */}
                                                        {localSettings.enableYomitan &&
                                                            Object.values(localSettings.ankiFieldMap || {}).includes('Frequency') && (
                                                                <div style={{
                                                                    marginTop: '20px',
                                                                    paddingTop: '15px',
                                                                    borderTop: '1px solid rgba(255,255,255,0.1)'
                                                                }}>
                                                                    <h4 style={{ marginTop: 0, marginBottom: '10px', color: '#ddd' }}>
                                                                        Frequency Export Mode
                                                                    </h4>
                                                                    <div className="grid">
                                                                        <label htmlFor="ankiFreqMode">Frequency Value</label>
                                                                        <select
                                                                            id="ankiFreqMode"
                                                                            value={localSettings.ankiFreqMode || 'lowest'}
                                                                            onChange={(e) => handleChange('ankiFreqMode', e.target.value)}
                                                                            style={{
                                                                                padding: '6px',
                                                                                borderRadius: '4px',
                                                                                border: '1px solid #444',
                                                                                background: '#222',
                                                                                color: 'white'
                                                                            }}
                                                                        >
                                                                            <option value="lowest">Lowest Frequency</option>
                                                                            <option value="harmonic">Harmonic Mean</option>
                                                                            {availableFreqDicts.map(dict => (
                                                                                <option key={dict} value={dict}>{dict}</option>
                                                                            ))}
                                                                        </select>
                                                                        <div style={{ gridColumn: '1 / -1', fontSize: '0.85em', color: '#aaa' }}>
                                                                            Choose which frequency value to export to Anki.
                                                                        </div>
                                                                    </div>
                                                                </div>
                                                            )}
                                                        <div style={{
                                                            marginTop: '20px',
                                                            paddingTop: '15px',
                                                            borderTop: '1px solid rgba(255,255,255,0.1)',
                                                        }}>
                                                            <button
                                                                type="button"
                                                                onClick={previewSyntheticAnkiTest}
                                                                style={{
                                                                    padding: '9px 12px',
                                                                    cursor: 'pointer',
                                                                    backgroundColor: 'rgba(46, 204, 113, 0.15)',
                                                                    border: '1px solid rgba(46, 204, 113, 0.6)',
                                                                    color: 'white',
                                                                    borderRadius: '4px',
                                                                }}
                                                            >
                                                                Preview synthetic test note
                                                            </button>
                                                            <div
                                                                style={{ marginTop: '8px', fontSize: '0.85em', color: '#aaa' }}
                                                            >
                                                                Reviews the selected deck, model, mapping, tag, and
                                                                generated test media before creating one note.
                                                            </div>
                                                        </div>
                                                    </div>
                                                )}
                                            </>
                                        )}
                                    </div>
                                </div>
                            </div>
                        </>
                    )}

                    <h3>General Settings</h3>
                    <div style={sectionBoxStyle}>
                        <div className="checkboxes">
                            <label style={checkboxLabelStyle}>
                                <input type="checkbox" checked={localSettings.mobileMode} onChange={(e) => handleChange('mobileMode', e.target.checked)} style={checkboxInputStyle} />
                                <div>
                                    Mobile Mode
                                    <div style={{ opacity: 0.6, fontSize: '0.85em' }}>
                                        Optimizes layout and gestures for smaller screens.
                                    </div>
                                </div>
                            </label>
                            <label style={checkboxLabelStyle}>
                                <input type="checkbox" checked={localSettings.debugMode} onChange={(e) => handleChange('debugMode', e.target.checked)} style={checkboxInputStyle} />
                                <div>
                                    Debug Mode
                                    <div style={{ opacity: 0.6, fontSize: '0.85em' }}>
                                        Shows extra diagnostics and debug overlays.
                                    </div>
                                </div>
                            </label>
                            <label style={checkboxLabelStyle}>
                                <input type="checkbox" checked={localSettings.disableStatusIcon} onChange={(e) => handleChange('disableStatusIcon', e.target.checked)} style={checkboxInputStyle} />
                                <div>
                                    Disable Status Icon
                                    <div style={{ opacity: 0.6, fontSize: '0.85em' }}>
                                        Hides the floating status indicator in readers.
                                    </div>
                                </div>
                            </label>
                        </div>
                    </div>
                    <h3>Manga Settings</h3>
                    <div style={sectionBoxStyle}>
                        <h4 style={{ marginTop: 0 }}>General</h4>
                        <div className="checkboxes">
                            <label style={checkboxLabelStyle}>
                                <input type="checkbox" checked={localSettings.enableOverlay} onChange={(e) => handleChange('enableOverlay', e.target.checked)} style={checkboxInputStyle} />
                                <div>
                                    Enable Text Overlay
                                    <div style={{ opacity: 0.6, fontSize: '0.85em' }}>
                                        Shows OCR text overlays while reading manga.
                                    </div>
                                </div>
                            </label>
                            <label style={checkboxLabelStyle}>
                                <input type="checkbox" checked={localSettings.soloHoverMode} onChange={(e) => handleChange('soloHoverMode', e.target.checked)} style={checkboxInputStyle} />
                                <div>
                                    Solo Hover
                                    <div style={{ opacity: 0.6, fontSize: '0.85em' }}>
                                        Only show the active hover box instead of all boxes.
                                    </div>
                                </div>
                            </label>
                            <label style={checkboxLabelStyle}>
                                <input type="checkbox" checked={localSettings.enableDoubleClickEdit} onChange={(e) => handleChange('enableDoubleClickEdit', e.target.checked)} style={checkboxInputStyle} />
                                <div>
                                    Enable Double-Click Edit
                                    <div style={{ opacity: 0.6, fontSize: '0.85em' }}>
                                        Allows double-click to edit OCR text boxes.
                                    </div>
                                </div>
                            </label>
                            <label style={checkboxLabelStyle}>
                                <input type="checkbox" checked={localSettings.enableDoubleTapZoom} onChange={(e) => handleChange('enableDoubleTapZoom', e.target.checked)} style={checkboxInputStyle} />
                                <div>
                                    Enable Double-Tap Zoom
                                    <div style={{ opacity: 0.6, fontSize: '0.85em' }}>
                                        Allows double-tap to zoom in the manga reader.
                                    </div>
                                </div>
                            </label>
                        </div>

                        <h4>Visuals</h4>
                        <div className="grid">
                            <label htmlFor="colorTheme">Theme</label>
                            <select id="colorTheme" value={localSettings.colorTheme} onChange={(e) => handleChange('colorTheme', e.target.value)}>
                                {Object.keys(COLOR_THEMES).map((k) => <option key={k} value={k}>{k}</option>)}
                            </select>
                            <div style={{ gridColumn: '1 / -1', fontSize: '0.85em', color: '#aaa' }}>
                                Controls overlay colors and highlight styling.
                            </div>
                        </div>

                        <h4>Fine Tuning</h4>
                        <div className="grid">
                            <label htmlFor="dimmedOpacity">Opacity</label>
                            <input id="dimmedOpacity" type="number" step="0.1" max="1" min="0" value={localSettings.dimmedOpacity} onChange={(e) => handleChange('dimmedOpacity', parseFloat(e.target.value))} />
                            <div style={{ gridColumn: '1 / -1', fontSize: '0.85em', color: '#aaa' }}>
                                Background dim amount for non-focused text.
                            </div>
                            <label htmlFor="focusScale">Scale</label>
                            <input id="focusScale" type="number" step="0.1" value={localSettings.focusScaleMultiplier} onChange={(e) => handleChange('focusScaleMultiplier', parseFloat(e.target.value))} />
                            <div style={{ gridColumn: '1 / -1', fontSize: '0.85em', color: '#aaa' }}>
                                Zoom multiplier for focused text.
                            </div>
                            <label htmlFor="fontMultH">H. Font Mult</label>
                            <input id="fontMultH" type="number" step="0.1" value={localSettings.fontMultiplierHorizontal} onChange={(e) => handleChange('fontMultiplierHorizontal', parseFloat(e.target.value))} />
                            <div style={{ gridColumn: '1 / -1', fontSize: '0.85em', color: '#aaa' }}>
                                Font size multiplier for horizontal text.
                            </div>
                            <label htmlFor="fontMultV">V. Font Mult</label>
                            <input id="fontMultV" type="number" step="0.1" value={localSettings.fontMultiplierVertical} onChange={(e) => handleChange('fontMultiplierVertical', parseFloat(e.target.value))} />
                            <div style={{ gridColumn: '1 / -1', fontSize: '0.85em', color: '#aaa' }}>
                                Font size multiplier for vertical text.
                            </div>
                            <label htmlFor="boxAdjust">Box Adjust (px)</label>
                            <input id="boxAdjust" type="number" step="1" value={localSettings.boundingBoxAdjustment} onChange={(e) => handleChange('boundingBoxAdjustment', parseInt(e.target.value, 10))} />
                            <div style={{ gridColumn: '1 / -1', fontSize: '0.85em', color: '#aaa' }}>
                                Expands or shrinks OCR bounding boxes.
                            </div>
                        </div>

                        <h4>Interaction</h4>
                        <div className="grid">
                            <label htmlFor="interactMode">Mode</label>
                            <select id="interactMode" value={localSettings.interactionMode} onChange={(e) => handleChange('interactionMode', e.target.value)}>
                                <option value="hover">Hover</option><option value="click">Click</option>
                            </select>
                            <div style={{ gridColumn: '1 / -1', fontSize: '0.85em', color: '#aaa' }}>
                                Choose how text boxes activate in the reader.
                            </div>
                            <label htmlFor="delKey">Delete Key</label>
                            <input id="delKey" value={localSettings.deleteModifierKey} onChange={(e) => handleChange('deleteModifierKey', e.target.value)} placeholder="Alt, Control, Shift..." />
                            <div style={{ gridColumn: '1 / -1', fontSize: '0.85em', color: '#aaa' }}>
                                Modifier key used to delete OCR boxes.
                            </div>
                            <label htmlFor="mergeKey">Merge Key</label>
                            <input id="mergeKey" value={localSettings.mergeModifierKey} onChange={(e) => handleChange('mergeModifierKey', e.target.value)} placeholder="Alt, Control, Shift..." />
                            <div style={{ gridColumn: '1 / -1', fontSize: '0.85em', color: '#aaa' }}>
                                Modifier key used to merge OCR boxes.
                            </div>
                        </div>
                    </div>

                    <h3>Maintenance</h3>
                    <div style={sectionBoxStyle}>
                        <div style={{ display: 'flex', flexDirection: 'column', gap: '12px' }}>
                            <div style={{ display: 'flex', flexDirection: 'column', gap: '6px', width: '100%' }}>
                                <button
                                    type="button"
                                    onClick={resetYomitanDB}
                                    style={{
                                        textAlign: 'left',
                                        padding: '10px 14px',
                                        borderRadius: '8px',
                                        border: '1px solid #c0392b',
                                        background: 'rgba(192, 57, 43, 0.12)',
                                        color: '#f4d3cf',
                                        cursor: 'pointer',
                                        fontWeight: 600,
                                        width: 'fit-content',
                                    }}
                                >
                                    Reinstall Dictionary Database
                                </button>
                                <div style={{ fontSize: '0.85em', color: '#aaa' }}>
                                    Deletes all dictionaries and reinstalls the selected language.
                                </div>
                            </div>
                            <div style={{ display: 'flex', flexDirection: 'column', gap: '6px', width: '100%' }}>
                                <button
                                    type="button"
                                    onClick={purgeCache}
                                    style={{
                                        textAlign: 'left',
                                        padding: '10px 14px',
                                        borderRadius: '8px',
                                        border: '1px solid #c0392b',
                                        background: 'rgba(192, 57, 43, 0.12)',
                                        color: '#f4d3cf',
                                        cursor: 'pointer',
                                        fontWeight: 600,
                                        width: 'fit-content',
                                    }}
                                >
                                    Clear OCR Cache
                                </button>
                                <div style={{ fontSize: '0.85em', color: '#aaa' }}>
                                    Removes cached OCR results stored on the server.
                                </div>
                            </div>
                        </div>
                    </div>
                </div>
                <div className="ocr-modal-footer">
                    <button type="button" className="warning" onClick={resetToDefaults} style={{ marginRight: 'auto', background: '#e67e22', borderColor: '#d35400' }}>Defaults</button>
                    <button type="button" className="primary" onClick={onClose}>Close</button>
                </div>
            </div>
        </div>
    );
};
