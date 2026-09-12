# Android manga-only Manatan fork — Codex implementation brief

Prepared 8 September 2026. This is an implementation assignment, not a request for another plan or a demonstration. Deliver one integrated Android acceptance build with the requested functionality together. Use internal milestones and continuous testing, but do not require me to test successive incomplete feature slices.

## 1. Product objective and authority

Create an Android-only manga reader derived from Manatan. Preserve its existing manga dictionary lookup and sentence-mining capabilities, but remove the all-in-one immersion-app product surface. The result should feel like a focused, polished manga reader, with Hoshi Reader Android informing the simplicity of dictionary management, Anki configuration, and statistics.

Upstream and reference:

```text
https://github.com/KolbyML/Manatan
https://github.com/HuangAntimony/Hoshi-Reader-Android
```

Support manga on the phone, manga in a remote folder, and manga served by Komga. Make adding these sources an understandable onboarding flow rather than a sequence involving server settings, extensions, source IDs, and separate browse screens.

The app must have no subscriptions, paid unlocks, advertising, Manatan login, Manatan SRS, Discord promotion, or dependency on a Manatan membership. Credentials for my own Komga/NAS and optional AnkiConnect endpoint are legitimate source credentials, not app accounts.

My phone will be connected by USB. Use ADB to inspect the installed standard Manatan, compare behaviour, and install/test this fork alongside it. Verify the actual device and OS rather than assuming them. Account for Samsung Fold-style narrow and expanded layouts. The development host may be Windows with WSL or a Linux server; determine which machine actually has ADB/device access before choosing the build/deploy workflow.

Use the temporary display name `Manga Reader (Fork)` and a distinct, configurable application ID such as `dev.local.mangareader`. Keep branding configuration central. Do not create or publish a public release, upload private data, or push to upstream as part of this task. Work locally on a feature branch; retain upstream history and attribution. A local Git fork/derivative is sufficient unless a separate remote fork is already configured.

## 2. First resolve the public-source/release mismatch

Do not assume the public repository contains everything used by the latest APK.

Research for this brief found that the public Rust workspace depends on `KolbyML/Manatan-Server-Public`. That repository's README explicitly describes a wrapper around precompiled static libraries built from private backend source. Public Manatan Android instructions describe a Rust/WebUI/Java-wrapper build, whereas recent release notes refer to Flutter. These observations are an audit lead, not proof that every current component is unavailable. Recheck the repository state when starting.

Before substantial UI work, inspect the default branch, relevant tags/branches, submodules, dependency manifests, release workflows, generated assets, native libraries, model downloads, and their licensing. Record the exact upstream commit, official APK version and checksum, and source versions used. Determine whether the public source reproduces the intended baseline, rather than assuming a GitHub source archive corresponds to the shipping app.

Create `docs/UPSTREAM_AUDIT.md` containing a small ownership/dependency map. Classify the reader, source catalogue, local storage, Komga adapter, OCR, dictionaries, audio, Anki bridge, statistics, accounts, and SRS as editable source, externally licensed component, opaque binary, or unavailable implementation. Identify any shared services that the manga pipeline genuinely needs.

Choose the least disruptive technically sound implementation:

* Reuse and refactor the available reader, OCR, dictionary, and mining code wherever practical.
* When an unavailable backend prevents the requirements, implement or replace that narrow manga-specific interface using available, appropriately licensed components. Do not retain an account-bound private backend simply because it makes the first build easier.
* Do not rewrite the entire app in Kotlin/Compose or Flutter merely for appearance. Conversely, do not pretend an inaccessible current frontend is editable: document and implement the smallest necessary replacement if the audit establishes that requirement.
* Do not silently switch the project to a Hoshi fork, bypass paid services, spoof membership, or distribute binaries/models whose redistribution terms are unverified. Prefer a rebuildable, independently maintainable free implementation.

A missing source component must not be concealed by disabling its UI. Continue implementation of independent work while resolving the dependency. Document precise remaining blockers rather than declaring everything complete, manufacturing parity, or repeatedly asking me to approve technical choices. If a fundamental dependency cannot be replaced in the available run, report the boundary and preserve a resumable working tree; do not label the result feature-complete.

Treat upstream Android build documentation as a starting point, not authoritative current toolchain configuration. Derive and pin versions from the chosen working source/build workflow. Keep build tools scoped to this project where practical. Do not change unrelated projects or system-wide settings unnecessarily.

## 3. Establish the manga/mining baseline before deleting features

Create `docs/MANGA_PARITY.md`. For each manga-related capability, record its observable behaviour in stock Manatan, relevant implementation location when available, fork equivalent, and verification result. Compare on a small, authorised test library and a dedicated Anki test deck.

Inspect the complete journey: open manga; obtain OCR; select text; open and navigate the dictionary; choose a definition; edit sentence context; select/crop an image; choose word audio where supported; create a note; inspect the resulting fields and media in Anki. Include settings that affect these steps, not only the visible reader.

Preserve all observed manga-relevant options: dictionary ordering and formatting, recursive lookup, readings/furigana, frequency/pitch information, supported lookup languages, selection controls, context editing, image capture, audio sources, Anki field/template semantics, tags, duplicate checks, supported manga export paths, and existing relevant keyboard/gesture controls. Distinguish confirmed capabilities from assumptions. Do not narrow support to a hardcoded vocabulary-card template.

Remove anime/video/novel/audio-player features, but do not mistake pronunciation audio used in dictionary lookups/cards for an unwanted media player. Likewise, remove the SRS review engine and its UI without deleting mined-word history or other learning metadata required by existing manga mining. Decouple those records from SRS/account services if necessary.

Latest public release and the installed stock version may differ. Record both. Never overwrite or downgrade my installed stock app merely to create a matching baseline; use a separate emulator/test environment for another official version when practical. Be explicit about which version was actually observed.

## 4. User experience and navigation

Use four clear destinations: **Library**, **Activity**, **Stats**, and **Settings**. Library is the default. Activity contains OCR and offline-download jobs, not a social feed. Avoid a generic all-media dashboard or an extension marketplace as the first-run experience.

Use Hoshi as a behavioural and visual reference, not a request to copy its identity. Inspect its current implementation of dictionary management, Anki mapping, and statistics. Use restrained typography, consistent spacing, readable controls, clear empty/error states, and system light/dark appearance. Keep the UI coherent across the app; do not graft a second settings framework onto the first.

Library should provide a searchable cover grid, continue reading, source and reading-state filters, sensible sorting, and category chips. A series opens its volumes/chapters in natural reading order. Support long-press selection and clear bulk actions. Remember scroll position, sort, and filters. Avoid turning every optional filter into another permanent top-level tab.

One **Settings** page is the only settings destination. Group settings into Sources & storage, Reader, Dictionaries & lookup, Anki & mining, OCR, Library & privacy, Statistics, and Backup/About. Show current values and connection status. Ordinary preferences are editable on this page; complex operations such as field mapping or dictionary import may use a focused sheet/dialog, not a separate “Manatan settings” universe. Reader quick controls and Settings must edit the same underlying preferences.

Normal library/settings screens should behave like ordinary Android screens with correct system insets. Immersive mode belongs in the reader. Back should first dismiss selection/popup, then close the reader, without losing position. Avoid tiny desktop-style hover controls. Support accessibility labels, usable touch targets, larger text, TalkBack where appropriate, and reduced motion.

## 5. Sources and onboarding: “Where is your manga?”

On first launch show a short choice: **On this phone**, **Remote folder**, **Komga**. Explain each in one sentence. The same flow is available later through Add source. Allow multiple sources. Let people start reading before configuring dictionaries or Anki; explain mining setup only when needed.

### On this phone

Use Android's document/folder picker and persisted document permissions. Do not require typing an internal filesystem path or copying everything into a special Manatan directory. Support adding a folder and importing selected archives. Prefer referencing originals; copying into managed storage must be an explicit option.

The minimum is CBZ/ZIP and folders containing page images, including JPEG, PNG, and WebP. Preserve additional manga formats already supported by the chosen baseline when their implementation/licensing is available; inventory CBR/RAR, PDF, and other image formats rather than falsely advertising them. Novel/EPUB reading is outside scope.

Handle common structures such as:

```text
Manga/
  Series A/
    Vol 01.cbz
    Vol 02.cbz
  Series B/
    Vol 01/
      001.jpg
      002.jpg
```

Support Japanese/Unicode names, mixed case extensions, single-volume folders, and numeric sorting so volume 10 follows volume 9. Offer a scan preview with detected series/volumes and a simple grouping correction when ambiguous. Do not rename, move, unpack beside, or delete original files automatically.

### Remote folder

Make this a real folder connector, not a URL text box that cannot enumerate content. Implement **SMB 2/3** and **WebDAV** for the first integrated release. These are deliberate implementation requirements, not assumptions that current Manatan already supports both. SMB 1, FTP, SFTP, and arbitrary HTTP directory scraping are not required.

Use a simple connection form: server/address, credentials when needed, test connection, choose share/root/subfolder, preview, and scan. Show the protocol only where it is necessary. Read archives/pages on demand with bounded local caching; make whole-volume offline download explicit. A temporary complete archive download may be needed for formats/transports without efficient random access: surface its progress rather than promising instant streaming.

Use maintained, appropriately licensed protocol libraries; do not implement SMB or WebDAV from scratch. Test wrong credentials, disconnected networks, Unicode paths, and reconnects.

### Komga

Provide a dedicated Komga connection flow: base URL, authentication supported by the server/API, Test connection, select libraries, then import/refresh the catalogue. Do not ask the user to select the server's filesystem folder with Android's picker or configure a generic extension by ID.

Reuse a suitable editable adapter or implement against Komga's documented API. Handle pagination, series/book IDs, thumbnails, pages, metadata, progress, base-path deployments, and LAN/Tailscale addresses. Do not assume a hosted server is publicly reachable. Refresh the client catalogue rather than triggering an administrative server-wide filesystem scan.

Read server progress and persist local progress promptly. Support queued progress writeback where permitted, with a documented conflict policy that does not silently overwrite newer local progress. A read-only account should still read manga. Do not modify server categories or metadata just because I organise the local library.

### Shared source behaviour

Use stable identities: source-instance UUID, stable series/book identity, and page/content identity. Do not key everything by title or transient download URL. Rescans must be idempotent and preserve reading progress, categories, sensitivity flags, and valid OCR. Recognise changes with server IDs, metadata, and appropriate fingerprints; offer relinking for ambiguous local moves rather than destroying history.

A server outage or lost folder permission is not an empty library. Preserve entries, explain the problem, and provide reconnect/re-authorise. Incremental scans should update covers/new volumes without duplicating the library. Removing a source removes the connection/library association only after confirmation; it must not delete NAS/original files.

Store credentials using Android-backed secure storage. Keep secrets out of logs, URLs, Git, screenshots, and exports. Verify TLS; do not use a global trust-all switch. Make insecure local HTTP an explicit per-source choice. Access only configured servers/folders, not arbitrary LAN discovery or credential harvesting.

## 6. Manga reader quality

Keep and polish the existing functional reader rather than substituting a basic image gallery. Provide RTL reading by default for Japanese manga, LTR and vertical modes, fit controls, pinch zoom/pan, double-tap zoom, a page scrubber/thumbnails, bookmarks, resume, and next/previous volume navigation. Retain other useful established reader settings where they remain relevant.

Support larger-screen/two-page presentation where suitable, while keeping the narrow phone layout uncluttered. Preserve page, zoom where reasonable, reading direction, and active mining context across rotation/resizing/fold transitions. Do not lose the current page when an image finishes decoding or an OCR result arrives.

Prioritise the visible page over background prefetch/OCR. Bound image decoding and bitmap memory, cancel obsolete prefetches, and avoid loading a whole volume into RAM. Do not perform remote scans, archive processing, dictionary imports, or OCR on the UI thread. Measure cold start, library scrolling, page turns, and lookup latency on the device; report regressions against the observed baseline rather than inventing performance numbers.

Text hitboxes and crop coordinates must stay correct with zoom, pan, fitting, cropping, rotation, page splitting, and two-page layouts. Include these transformations in automated tests.

## 7. Bulk OCR and honest readiness indicators

From Library, select multiple series; from a series, select multiple volumes/chapters. Offer **Prepare OCR**. Show the selected volume/page count when known, existing completion, and planned work before starting. Selecting a series includes its currently selected/discovered volumes, not an undocumented perpetual subscription to every future volume.

Use one durable task system, not separate ad-hoc loops in UI components. Persist per-page jobs and results so queued/running work can recover after interruption. Deduplicate overlapping selections and repeated taps; finish already-valid work without repeating OCR unnecessarily. Keep volume/series aggregation separate from page processing.

Provide pause/resume, cancellation, retry failed/missing pages, and explicit reprocess. Display progress at page and volume level, useful errors, and per-item recovery. Use bounded concurrency with sensible battery/thermal behaviour. A cancelled job must not corrupt earlier results or leave permanently “running” records.

Provide a free usable OCR path. Prefer a working on-device engine with suitable Japanese manga accuracy after a clearly explained model download. Audit engine/model availability, licence, and actual device compatibility. Do not substitute an arbitrary text recogniser without comparing vertical text, furigana, punctuation, and bubble geometry against the baseline. Do not train a new model as part of this task.

Preserve existing free OCR providers when usable. A user-configured self-hosted provider may be an optional alternative, but must not become a mandatory second product or conceal failure of the phone-only path. No paid API key, Manatan account, or vendor-hosted quota may be required for the core workflow. Never upload pages to an external OCR service silently; identify destination and require explicit opt-in.

Store text, spatial geometry, orientation/reading order, language, engine/model version, page dimensions, and page fingerprint with results. Save atomically. Keep completed OCR in durable app storage, not an evictable temporary cache while displaying it as permanently available. Successful blank/no-text pages are processed pages; network failures are not.

Volume states: **Not prepared**, **Queued**, **Processing**, **Partial**, **OCR ready**, **Failed**, and **Out of date**. `OCR ready` means all pages for the current content version have validated, locally usable results. Imported/downloaded OCR must pass the same validation. Preserve baseline import/export formats where available and document compatibility; do not invent a cloud OCR download service.

Show compact OCR badges on thumbnails and detailed counts on tap. A series can show `OCR 3/8 volumes`; do not mark the whole series complete because its cover or one chapter was processed. Clear/invalidate the badge when data is deleted, missing, corrupted, or stale.

Keep **image availability** separate: `Downloaded` means required manga images are stored locally; `OCR ready` means text/geometry are locally available. A book can have either, both, or neither. Offer an explicit **Download + prepare OCR** action for offline use. Verify the combined state in airplane mode with installed dictionaries and local AnkiDroid.

Use Android-appropriate background execution with user-visible progress/notification and necessary permissions. Check the current OS/target-SDK constraints. Do not promise unlimited uninterrupted processing or work continuing after Android force-stop. Persist checkpoints, stop cleanly when required, and recover after allowed rescheduling or reopening the app. Test ordinary process death separately from force-stop/relaunch.

## 8. Dictionaries, lookup, and Anki

### Dictionary management

One comprehensible manager should import one or multiple Yomitan-format ZIP dictionaries, display import progress/errors, list installed dictionaries, enable/disable/reorder them, and remove or replace them. Distinguish term/kanji dictionaries, frequency lists, and pitch data clearly. Retain richer supported dictionary content, including structured definitions and media; do not flatten everything into plain text for convenience.

Imports must be cancel-safe and leave prior working dictionaries intact after failure. Handle large dictionaries off the UI thread. Validate archives, bound decompression, prevent path traversal, and safely render untrusted dictionary markup. Keep dictionary setup accessible from the same unified settings page.

Preserve popup behaviour useful for manga, including recursive lookups and choosing the relevant entry/definition. Keep dictionary assets available offline. Retain free word-audio options where available; external audio availability must not block text-only mining.

### Anki setup and field mapping

Use **AnkiDroid on this phone** as the simple default. Preserve **AnkiConnect** as an alternative for my existing workflows. Use supported APIs, package visibility rules, permissions, and media-sharing mechanisms; do not write directly into Anki databases or guess private storage paths.

The flow is: choose backend, verify connection/permission, choose deck, choose note type, map its actual fields, preview a sample card, save. Fetch deck/model/field names from the selected backend. Give each actual Anki field a row with a readable source selector and a live preview, plus advanced template editing where current mining supports it. Never require users to memorise placeholder syntax for normal mapping.

Support the baseline's manga-relevant field values and formatting, including expression, reading/furigana, chosen definition(s), sentence/context, pronunciation audio, image/page crop, source/series, volume/chapter, page number, source reference, and tags as applicable. Do not invent sentence audio for a silent manga or promise cloud translation. Keep existing free optional enrichment only when genuinely supported.

Preserve custom mappings/templates; note-type changes must not silently destroy them. Warn about missing required content, unknown placeholders, unavailable audio, and changed fields. Honour selected definitions, ordering, and duplicate policy. Media links must work in the created card, not reference temporary app files that immediately disappear.

Allow sentence/text correction and image-region adjustment before mining. Persist pending note data and media safely during interruptions. Record successful note IDs/results; retries must not silently create duplicate cards. Where a backend cannot guarantee idempotency, detect/report ambiguous outcomes and require a safe retry decision instead of claiming success.

A test-card action should preview first and use an explicitly selected test destination. Automated testing uses a dedicated test deck and identifiable test tags. Never bulk-delete, remap, overwrite, or synchronise my production Anki collection as a test.

## 9. Categories and NSFW visibility

Make categories simple named shelves with multiple membership. Create/rename/reorder/remove them in one small manager, and assign/unassign from the library or bulk-selection sheet. Removing a category does not remove manga. Keep system reading-state filters separate from user categories. Preserve membership through rescans and restarts.

Implement a dedicated local **NSFW/sensitive** flag, independent of category names. Provide one-tap Mark NSFW / Remove NSFW actions in item menus and bulk selection. Provide one obvious **Show NSFW manga** setting, off by default for a new install. Do not require automated content classification.

A sensitive series hides all its volumes. A separately flagged volume stays sensitive even when the series flag is later removed. With hidden individual volumes, ensure the parent-series thumbnail does not use a sensitive volume's cover; use a safe alternative or neutral placeholder.

When visibility is off, consistently exclude sensitive entries from library/search/recent/continue-reading and itemised statistics. Apply the same policy to category results/counts and source catalogue presentation. Redact sensitive titles/covers in task notifications and Activity instead of leaking them through progress messages. Preserve data and queued work; hiding is not deletion. Changing the setting must refresh the current UI immediately and persist across relaunch.

Keep explicit local flags through metadata refresh. Imported source labels must not silently override a user's choice. This is a visibility feature, not an encryption/PIN-protected vault; do not claim otherwise. Test with ordinary fixture images carrying sensitivity metadata.

## 10. Local statistics, with defined measurement rules

Use Hoshi's clear statistics presentation as a reference. Provide daily/weekly/monthly/all-time views for active reading time, pages read, completed volumes, dictionary lookups, and successful cards mined, with per-series breakdowns and a restrained history chart. Optional streaks should remain secondary, not a gamified dashboard.

Count active reader time with a documented inactivity rule. Dictionary/mining interactions opened from an active reading session can count; background jobs, a locked screen, leaving the reader, and overnight idle time must not. Separate unique pages encountered from rereading/page-turn counts, and avoid counting preloaded pages as read. Count a mined note once after confirmed creation, not when its button is tapped or a request fails.

If showing characters or characters/hour, label them as OCR-derived estimates from pages actually encountered, with a documented normalisation rule. Do not count the whole OCR queue as reading. Missing OCR is unknown coverage, not zero characters; avoid silently revising historical totals without preserving provenance. Manga OCR, furigana, and sound effects make this metric less exact than EPUB text.

Keep statistics local and independent of SRS/accounts. Use the device timezone and test Europe/London midnight/DST boundaries. Allow a simple CSV/JSON export. No fabricated sample statistics in the acceptance build.

## 11. Trimming, storage ownership, backup, and security

Remove unwanted navigation, routes, settings, startup work, downloads, permissions, and build artefacts—not only the visible tabs. Remove novel/EPUB, anime/video, YouTube/streaming, podcasts/audiobooks, external screen-capture immersion tools, Manatan cloud login/membership, SRS review, community promotion, and non-Android release pipelines from this product. Do not remove dependencies still required for manga dictionaries/audio/mining without replacing their function.

Do not add new media extensions or a storefront. Retain additional existing self-hosted manga adapters only when they are already functional, free, and do not complicate the required source UX; do not use them to postpone SMB/WebDAV/Komga. Avoid a broad architectural “clean-up” unrelated to the requirements.

Ensure no unwanted vendor account/subscription requests happen on startup or while reading/mining. If local HTTP services are retained, bind locally, restrict access/origins appropriately, and namespace their ports/instances and storage so the fork cannot connect to or overwrite stock Manatan's backend. Avoid port-conflict “fixes” that stop another app's services.

Give the fork distinct application ID, authorities, intents/deep links, service identities, notification channels where relevant, storage/database/cache paths, WebView profile/origin where relevant, update channel, and signing identity. Audit JNI names and hardcoded package references carefully. Remove upstream self-updating behaviour that could replace the fork with stock.

Provide export/restore of app settings, library references, categories, sensitivity flags, progress, stats, and optionally OCR. Keep credentials excluded by default, clearly explain that source access permissions may need re-authorisation, and validate restore compatibility. Preserve existing relevant backup capabilities when possible. Do not bundle my manga or dictionaries into project releases.

Hoshi's repository identifies GPL-3.0; treat it as design inspiration unless deliberately reusing code under compatible terms. Check per-component/per-file licences in Manatan and every dependency rather than assuming a root licence badge covers private/precompiled components. Retain necessary attribution/licence notices in About and source distribution even though community/promotion links are removed.

## 12. Internal implementation sequence — one acceptance release

Work through these engineering milestones without asking me to approve each phase:

1. Audit source/build availability, pin versions, inspect stock, establish the parity checklist, and build a minimal unchanged baseline where feasible.
2. Establish the isolated fork identity and test/build infrastructure; define stable library/source identities, persistence, and the single settings ownership model.
3. Implement onboarding, local/SMB/WebDAV/Komga providers, idempotent scanning, offline storage, library/category/sensitivity behaviour, and core reader regressions.
4. Implement durable bulk OCR, validated readiness states, geometry correctness, and the full dictionary/mining/Anki configuration workflow.
5. Finish coherent UI, local statistics, backups, lifecycle/foldable behaviour, and remove inactive out-of-scope runtime/build dependencies.
6. Run the full automated/integration/device regression suite, fix failures, compare against the baseline, and package the integrated APK.

Create a concise project `AGENTS.md` linking to this brief and recording proven build/test commands, ownership boundaries, and device safety rules. Maintain an implementation checklist and checkpoint commits. Keep the tree resumable across context limits. Do not stop after writing these documents or a scaffold; implementation and verification are the assignment.

## 13. ADB, test data, and acceptance evidence

Use local execution on the machine that can reach the device. Establish `adb devices -l`, select the intended authorised device explicitly for every device command, and record model/OS/ABI without committing private identifiers. If using WSL for builds and Windows for USB, make the split explicit. Do not expose an ADB server to the LAN for convenience.

Inspect stock with supported package information, UI navigation, screenshots, and app-specific logs. Do not clear its data, uninstall it, downgrade it, root the phone, bypass Android app isolation, change production accounts, or use broad destructive shell commands. Build first, inspect the APK's actual package ID, and install/update only the fork. Stock and fork should both remain independently launchable.

Use synthetic/authorised Japanese test pages and small multi-volume archives. Create isolated SMB/WebDAV/Komga test services when useful, with constrained network exposure. Verify real protocol/API interaction, not only mocks. Never create public fixtures containing copyrighted manga, dictionary databases, NAS credentials, or screenshots of private libraries.

Mocks support deterministic error tests, but do not establish real compatibility. Separately label fixture-server tests and tests against my actual server. If my server address/credentials or an Android permission cannot be obtained through an authorised available setup, continue the other work and record exactly what is blocked. Do not claim my Komga/NAS was tested using only a mock. ADB forwarding/reverse rules must be scoped to this device/task and removed individually when done; never remove unrelated forwarding rules.

Test at least:

| Area | Required proof |
| --- | --- |
| Isolation | Both apps launch independently; no package, provider, port, database, update, or storage cross-talk. |
| Sources | Local + SMB + WebDAV + Komga add/test/select/scan/read flows; authentication errors; pagination; Unicode; natural ordering. |
| Rescans | New volume appears; changed cover refreshes; repeat scan makes no duplicates; outage does not erase library; tags/progress survive. |
| Reader | Navigation/resume/zoom/RTL/vertical; correct page after rotate/resize; no broken OCR hitboxes or crop geometry. |
| Offline | Downloaded images + prepared OCR + imported dictionary allow reading, lookup, and AnkiDroid mining without internet. |
| OCR | Multi-series and multi-volume selection; dedupe; partial failure/retry; blank pages; stale/corrupt data; interruption recovery; truthful rollups. |
| Dictionaries | Small and large imports; multiple selection; enabled/order changes; rich definitions/media; interrupted or invalid import does not corrupt working data. |
| Anki | Actual backend field enumeration; custom mapping/template; selected definition/sentence; visible crop; supported audio; duplicate and failed-send behaviour. Inspect created notes/media. |
| Privacy/categories | Bulk assignment/removal; NSFW hide/unhide across all affected surfaces; persist through scan/relaunch; no sensitive cover/title leaks. |
| Statistics | Background/OCR/preload do not count as reading; correct time/unique-page/card accounting; midnight/DST; missing OCR coverage is not fabricated. |
| Lifecycle | Background/foreground; ordinary process death; force-stop then reopen; lost folder permission; network loss; low storage; cancellation. |
| Scope | No removed media/account/SRS/payment navigation or active startup dependency; no placeholders for required features. |
| Restore/build | Backup restore round-trip; migrations; clean build; update the fork without losing fork data. |

Use suitable unit, integration, and Android/UI tests for the actual architecture; do not select frameworks just because they are fashionable. Add stable test selectors where needed. Capture relevant logcat, screenshots and test output. Test resource usage with a realistically sized library and record the fixture scale and device.

Automate portrait/landscape and resizing. Physical hinge movement, system permission approval, USB authorisation, or unlocking may require my intervention: identify these narrowly, continue other tests, and never claim a physical fold test occurred because an emulator was resized. Do not reboot my phone without separate explicit approval.

Use a dedicated Anki test deck and tags for all writes. Do not alter existing production notes. Do not run the OCR job over my entire real library merely to demonstrate the feature. Large-scale tests should use controlled fixtures unless I explicitly start the real queue.

## 14. Definition of done and final handoff

The goal is one installed, integrated Android build containing all requirements above—not an MVP, mocked dashboard, UI-only reskin, or request for me to assemble unfinished parts. Run internal implementation/test/fix cycles before handoff.

Deliver an ARM64 APK appropriate for the attached phone; a release-mode test build is preferable for performance validation. Sign with a separate local development/release key, keep the key outside Git, and document its location and backup requirement privately. Include APK checksum, application ID, version, build variant, and source commit. Supply an isolated debuggable build only when needed for diagnostics, clearly labelled.

Leave these artefacts in the repository/deliverables directory:

* Full source changes and dependency lockfiles; repeatable build/install/test scripts for the actual host setup.
* The source/dependency audit, manga parity matrix, implementation checklist, and relevant licence notices.
* An acceptance report with PASS / FAIL / BLOCKED for each requirement, exact tests/devices/backends exercised, screenshots, and concise known limitations.
* A short user guide covering add source, import dictionaries, configure Anki, bulk OCR, badge meanings, categories, NSFW visibility, offline reading, and backup.

Report the installed APK and how I launch it, a compact summary of implemented changes, and remaining limitations. Explicitly disclose deviations from stock mining behaviour or dependencies replaced. A compiling APK is not evidence that mining or OCR works; neither are successful mocks evidence of device/server integration.

Do not label a required path complete until its functional test passes. If an external blocker remains, give the usable work and precise outstanding boundary without pretending the requested fully featured release has been achieved. No paid service or subscription workaround is an acceptable substitute for the free core workflow.

---

## Research reference index

These sources were inspected while preparing this brief. Revalidate them against the implementation checkout; no application was built or device-tested during preparation of this document. Paths below are investigation leads, not a guarantee of current release parity.

```text
Manatan workspace and public backend dependency:
https://github.com/KolbyML/Manatan/blob/master/Cargo.toml

Private-backend wrapper description:
https://github.com/KolbyML/Manatan-Server-Public/blob/master/README.md

Public Android build instructions (older Rust/WebUI workflow):
https://github.com/KolbyML/Manatan/blob/master/docs/build/android.md

Shipping feature/release notes, including Komga and Flutter references:
https://github.com/KolbyML/Manatan/blob/master/changelog.txt

Public manga/OCR integration leads:
WebUI/src/Manatan/OCRManager.tsx
WebUI/src/Manatan/context/OCRContext.tsx
WebUI/src/Manatan/utils/api.ts
crates/ocr-server/src/lib.rs
crates/ocr-server/src/state.rs
crates/ocr-server/src/logic.rs
bin/manatan_android/java/com/mangatan/app/AnkiBridge.java
bin/manatan_android/java/com/mangatan/app/WebviewActivity.java

Hoshi reference implementation leads:
https://github.com/HuangAntimony/Hoshi-Reader-Android
app/src/main/java/moe/antimony/hoshi/features/anki/AnkiConnectView.kt
app/src/main/java/moe/antimony/hoshi/features/anki/AnkiViewModel.kt
app/src/main/java/moe/antimony/hoshi/features/anki/AnkiModels.kt
app/src/main/java/moe/antimony/hoshi/features/statistics/StatisticsView.kt
app/src/main/java/moe/antimony/hoshi/dictionary/DictionaryManager.kt
app/src/main/java/moe/antimony/hoshi/features/dictionary/DictionaryUpdateService.kt

Komga API:
https://komga.org/docs/openapi/komga-api/

Android folder/document access:
https://developer.android.com/training/data-storage/shared/documents-files

Android long-running work constraints:
https://developer.android.com/develop/background-work/background-tasks/persistent/how-to/long-running
https://developer.android.com/develop/background-work/services/fgs/timeout

AnkiDroid API:
https://github.com/ankidroid/Anki-Android/wiki/AnkiDroid-API

Local Codex execution:
https://developers.openai.com/codex/cli
```
