# Integrated acceptance checklist

Status date: 2026-09-12. Checked items are implemented and have the evidence described in `ACCEPTANCE_REPORT.md`; they do not imply that every parity row is finished.

- [x] Read the implementation brief completely and resolve the public-source/shipping-release mismatch.
- [x] Retain and empirically map the published `Manatan-Server-Public` static library as an embedded/local compatibility dependency.
- [x] Inspect stock Manatan package metadata read-only; never open, clear, replace, uninstall, downgrade, or access its data.
- [x] Isolate package, UID, storage, loopback ports, signing identity, OAuth/update paths, and launcher identity.
- [x] Remove account/paid/update/anime/novel/sync surfaces and provide Library, Stats, and Settings with Library as default and background activity inside Settings.
- [x] Provide one focused settings surface for sources, reader, dictionaries, mining, OCR, privacy, statistics, and backup/about.
- [x] Implement SAF folder and CBZ/ZIP scanning, stable identity, natural ordering, explicit unreadable records, scan preview, title overrides, and same-title merge.
- [x] Implement SMB 2/3, WebDAV, and dedicated Komga source adapters, authentication, pagination, page reads, stable rescans, and outage-safe catalogue retention.
- [x] Implement app-managed offline downloads and page-level OCR queues with truthful rollups, pause/resume/cancel/retry, foreground progress, and process recovery.
- [x] Bundle the Japanese ML Kit recogniser for free on-device OCR and persist text plus geometry per page.
- [x] Implement continuous vertical reading with lazy page loading, most-visible-page progress, resume, scrollbar jumps, and no page gap.
- [x] Reuse the Manatan focal-point zoom/pan path for bounded pinch, toolbar zoom, and smooth transform updates; double-tap zoom is intentionally disabled.
- [x] Preserve RTL/LTR paging, swipe navigation, fit modes, bookmarks, two-page wide mode, next/previous volume navigation, and immersive reader UI.
- [x] Implement categories, multiple membership, ordering, series/volume hidden marking, default-hidden content, and safe-cover selection.
- [x] Implement local history/statistics, day/week/month/all ranges, per-series aggregates, privacy filtering, inactivity accounting, and CSV export.
- [x] Simplify user-facing Stats to removable reading sessions grouped by manga, without lookup counters; recover legacy unassigned history from stable volume IDs/fingerprints and tag future base-reader events with their manga.
- [x] Move background Activity into Settings and reduce bottom navigation to Library, Stats, and Settings.
- [x] Add series/volume progress bars, explicit offline wording, a persistent reader Back button, and leak-free WebView relaunch/renderer recovery.
- [x] Keep offline ML Kit bulk OCR private while allowing explicitly consented reader taps to use the higher-quality inherited Lens path with an isolated cache.
- [x] Return a whole ML Kit text block for tapping/lookup instead of one OCR row, with cache-version isolation.
- [x] Run offline OCR regions through Manatan's existing geometric merger and make Reprocess OCR force fresh inference rather than count cache hits.
- [x] Keep dictionary popups clear of the selected word, synchronously disengage active OCR UI during every zoom/reset path, reject delayed post-zoom lookup/selection events, restore OCR afterward, and provide an experimental boxless lookup mode.
- [x] Present volumes as compact cover cards with progress and individual/bulk read-state actions; remove the duplicate Back control and natural-order subtitle.
- [x] Brand the isolated fork as Mangaki with the supplied launcher artwork and show a compact, truthful per-volume OCR-state icon.
- [x] Add individual, per-manga and all-visible selection for bulk Stats session removal.
- [x] Collapse all unified-settings sections by default and use dark pure-black as the fork default theme.
- [x] Implement versioned fork backup/restore without credentials and with source re-authorisation.
- [x] Exercise AnkiDroid enumeration/mapping/duplicate safety using one tagged synthetic note in the explicitly selected test deck.
- [x] Build and install a signed ARM64 acceptance APK alongside stock, preserving fork state across update.
- [x] Lock the user-confirmed base-Manatan tap lookup path as the regression baseline; reader polish must not replace its hit testing or zoom transforms.
- [x] Restore adjacent-volume chapter data and add a fork-only page scrubber without altering Manatan's viewer/pager implementation; show it with a current/total page badge only while Manatan's reader controls are revealed.

Deferred work is tracked in [`ROADMAP.md`](ROADMAP.md), including an optional user-configured OCR server. Deferred items are not counted as accepted features.
- [x] Run protocol fixtures for SMB/WebDAV/Komga, durable queue interruption/retry tests, focused lint/build/core tests, and reader device smoke tests.
- [ ] Complete the real-page mining journey regression: OCR selection, correction, dictionary lookup, crop, audio, custom template, send failure, and duplicate behavior together.
- [ ] Extend the passed real-page OCR tap check into an accuracy/transformed-overlay matrix across representative Japanese pages, including pinch while an overlay is visible.
- [ ] Run the remote adapters against the user's actual SMB/WebDAV/Komga services; fixture protocol coverage is complete, deployment-specific coverage is not.
- [ ] Resolve eight inherited `manatan-yomitan-server` deinflector-vector failures and run large/cancel-safe dictionary imports.
- [ ] Run long-volume reader soak/memory testing and hands-on pinch smoothness comparison against stock Manatan.
- [ ] Run comprehensive hidden-content leak checks, London DST boundary checks, and restore/re-authorisation UI tests.
- [ ] Implement an independently designed lock-screen/current-cover feature after its exact Hoshi behavior and hidden-cover privacy policy are confirmed; GPL Hoshi code/assets cannot be copied.
- [ ] Prototype and benchmark an Android-native conversion of `kha-white/manga-ocr`; the official Apache-2.0 project requires Python, PyTorch/Transformers, and an approximately 400 MB model, so it is not a drop-in APK dependency.
