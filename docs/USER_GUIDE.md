# Mangaki first-round test guide

This build is ready for focused feature testing, especially the reader. It is not yet full brief acceptance.

Version 0.2.30 uses Manatan's standard reader for continuous scrolling, page mounting, resume, tap zones, zoom/pan and reader controls; a thin adapter supplies local fork volumes and persists their progress. It fixes the OCR observer regression in 0.2.29 that mistakenly treated those standard reader pages as library covers, leaving every page without lookup hit targets. It also retains the black-screen recovery for a WebView whose embedded local server host disappeared. The provider choices, OCR cache, borderless OCR replacement text, Mangaki identity, and two-column library/volume cards remain fork features. The fork page scrubber follows those same reader controls: tap an empty/menu area to reveal it and its current/total page badge, then tap the empty area again to hide it.

Dictionary setup, AnkiConnect and manga reader essentials now live directly inside unified Settings. Legacy settings buttons route there instead of opening a separate Manatan modal. Dictionary controls are intentionally limited to popup theme and term/frequency ZIP installation/management. Anki controls cover the connection, deck, note type and field mappings. Background activity shows only current or actionable jobs, not completed history.

Use the Select icon on Library or Volumes (or long-press a cover), then tap covers to add them to the selection. The bottom bar contains the available bulk actions and can scroll sideways when they do not all fit. In Stats, tap Manage to select sessions or whole manga groups, then remove them from the bottom bar. Ordinary browsing does not show selection checkboxes.

## Reader test

Open **Library**, choose a series, then tap a volume card. In **Settings → Manga reader** choose:

- **Vertical** for a continuous, gapless page stream. Scroll normally; the page counter, resume position and scrollbar follow the most-visible page.
- **Right to left** or **Left to right** for page-at-a-time reading and swipe navigation.
- **Fit whole page** or **Fit width**. Two-page mode applies on wide screens.
- **Tap zones → Disabled** to turn off the left/right page-turn zones. This is Manatan's persisted reader setting; the other stock layouts remain available.

Double-tap does not zoom. Pinch between 1× and 5×; the touched point should stay under your fingers. At zoom, drag in both axes. Rotate/fold the phone and confirm the current page remains sensible. In standard mode, tap recognised text once to activate its OCR block, then tap the desired word in that block to open the lookup. Activating a different block replaces the first. One tap outside the lookup closes it. Pinching clears the active OCR box and popup when zoom begins, then OCR geometry refreshes when zoom ends.

The bottom reader control is a page scrollbar with a counter and arrows; there is no page-preview strip. Tap blank page space to reveal the controls. The top-left Back button returns to that series' Volumes screen; Android's edge-swipe Back returns directly to Library. Library's less-used filters are collapsed behind the small filter icon beside Search. Series and volume lists show reading progress bars.

The volume screen uses smaller cover cards. Use a card's action to mark one volume read/unread, or select several cards for bulk read-state changes. The small document icon alongside each volume's caption is subdued when OCR is not prepared, checked when OCR is processed, spins while work is queued/running, and carries a warning/error badge for partial, stale, or failed OCR. Pressing the icon exposes its exact accessible status. Please report the volume/page, orientation, reading direction, and whether OCR overlays were enabled for any reader problem. In this build, specifically try repeated pinch-out gestures deep in a long volume after a lookup: the page should remain nearby, the lookup and active OCR box should disappear as soon as the gesture starts, and OCR should work again when the gesture ends.

For resume testing, move to a recognisable page, leave the volume for Library, and reopen it. Repeat after backgrounding and reopening Mangaki. The saved page should render directly without a black screen or jump to page 1. A page number already overwritten by version 0.2.16 or 0.2.17 must be set again manually because the exact prior position is no longer present in the saved volume state.

## Sources and offline jobs

- **On this phone** uses Android's folder picker and a persisted read grant. Scans never alter originals.
- In **Library**, pull down (or use the refresh icon) to pick up new files copied into an on-device folder, such as by Calibre. This is additive: it imports new volumes only and leaves existing entries unchanged.
- **Remote folder** supports SMB 2/3 and WebDAV. For WebDAV/Komga, HTTP requires an explicit trusted-LAN opt-in; normal TLS validation remains on otherwise.
- **Komga** tests credentials and asks you to select libraries before save.
- Use **Preview** before applying a scan when you need title corrections or same-title merging.
- From a series, select volumes for **Make available offline**, **Prepare OCR**, or both. **Settings → Background activity** supports pause, resume, cancel, and retry. Android shows a foreground notification for long work. Mangaki runs one background OCR/download worker at a time and briefly yields between OCR pages so live reader lookups can take priority. A paused job keeps completed pages and resumes from the remaining page list.

The adapters passed controlled protocol fixtures, but your real server is the acceptance test for its authentication, paths, TLS and naming conventions.

## OCR, lookup, and mining

In **Settings → Manga reader → OCR provider**, choose one of:

- **On-device**: bundled ML Kit, private and offline, but less accurate on manga/furigana.
- **Self-hosted Manga OCR**: enter the server's Tailscale URL, such as `http://<server-tailnet-ip>:7332`, and tap **Test OCR server**. This uses owoocr's segmented Manga OCR for live and bulk OCR. Tailscale must be connected; no silent fallback changes the provider if it is unavailable.
- **Google Lens**: the inherited high-quality network path, only after explicit selection; it sends the current page to Google.

Prepared results and geometry persist by provider. After changing provider, select the affected volume and use **Reprocess OCR** to replace its older cache. Manga OCR is materially better suited to Japanese manga than the bundled general model, but it is not guaranteed to duplicate Google Lens exactly. The server adapter filters only a small kana-only region strongly nested inside a larger vertical text block, avoiding the broad small-text rules that previously made valid bubbles untappable.

The supplied server service is intentionally bounded: one inference runs at a time, no more than three requests are admitted, and CPU/memory/task limits prevent a burst from growing without bound. Extra requests receive a temporary busy response and can be retried instead of making the whole host unresponsive. If the server host has just rebooted, leave bulk OCR paused until the health test passes; resuming preserves already completed pages.

OCR is limited to reader-page images; library covers are not submitted. Tap recognised text on a manga page to open the geometrically merged text block in the selection/lookup popup rather than a single row. Select a series or volume and choose **Reprocess OCR** after an OCR/model/provider update or to replace older results; unlike Prepare OCR, this deletes the selected cached results and performs fresh recognition.

The compact lookup popup avoids the selected word rather than trying to avoid its complete OCR block. It stays hidden while a lookup is pending, shows dictionary and frequency metadata without large badges, and keeps long results inside a short scrollable card. As in base Manatan, its transparent backdrop gives dismissal a single owner: tap outside once to dismiss before selecting another block. Pinching, using a zoom control, resetting fit, or scrolling immediately closes the lookup and clears the one active OCR block. A delayed dictionary result or gesture-release tap must not bring it back, and whitespace-only OCR results are not drawn. OCR remains available at the new zoom level after the gesture. Enable **Experimental: tap original artwork without showing OCR boxes** under OCR settings to keep the page artwork visible and use invisible OCR hit regions; it performs direct lookup, the selected word receives a blue overlay, and no replacement-text selection rectangle is painted. Its placement can still be slightly approximate because the current cache has only block geometry, not per-glyph boxes; a stronger OCR model alone does not solve that mapping limitation.

Closing or backgrounding the fork and tapping its launcher icon should return directly to the existing reader/library WebView. Android process reclamation is also supported. If reopening ever leaves a blank native surface for more than a second, report whether the app was closed with Home, Back, Recents swipe-away, or by the OS.

Dictionary/lookup and Anki settings are under the unified Settings screen. The AnkiDroid bridge passed one synthetic tagged-note and duplicate test in the configured test deck. Until the complete reader mining journey is accepted, keep using a disposable test deck—not a production deck—for card creation feedback.

## Categories, privacy, stats, and backup

Library selection provides category and series/volume hidden actions. Hidden content is excluded by default. Check Library, Background activity, Stats, search and covers when testing that policy.

Backup JSON excludes credentials and intentionally requires source re-authorisation after restore.

The Stats screen presents reading sessions grouped by manga, with reading time and pages only. A session starts after 15 minutes away. Select sessions individually, by manga, or all at once to remove them in bulk. Background OCR/offline job controls are under **Settings → Background activity**. Settings sections now start collapsed, and the fork defaults to a dark pure-black theme.

For remote sources, **Available offline** means every page was copied into the fork's private storage so the source server is not required while reading. Local-folder manga already resides on the device and is never duplicated by this action.
