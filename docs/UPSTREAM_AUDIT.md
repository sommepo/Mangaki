# Upstream and dependency audit

Audit date: 2026-09-09. This document distinguishes observed facts from pending device verification.

## Version resolution

- Public upstream: `https://github.com/KolbyML/Manatan`, default branch `master`.
- Pinned starting commit: `4862ea479233c5713111fe73bb9df277968d08f2` (`v6.1.8`, 2026-09-08T08:06:55Z).
- Latest official Android artefact named by that tag: `Manatan-Android-v6.1.8.apk`.
- Published and independently calculated SHA-256: `73f1ed0499d2030ea8824caa2e53848174c52174009997a7562aab1038a91577`.
- Official APK manifest: package `com.mangatan.app`, version name `6.1.8`, version code `17170696`, min SDK 26, target/compile SDK 36, ARM64 only.
- Installed stock version: `6.0.101`, observed read-only through package metadata on the authorised device. It differs from the audited official `6.1.8` APK; stock data was not opened or modified.

## Public source versus shipping APK

The tag and default branch align, but the public Android target does **not** reproduce the shipping application frontend:

- Public `bin/manatan_android` is a Rust `NativeActivity`/egui bootstrap with Java `MangatanActivity` and `WebviewActivity`, embedding the React WebUI.
- The official APK contains Flutter assets, `libflutter.so`, `libapp.so`, and launch activity `com.mangatan.app.StartupActivity`, alongside `assets/webui.tar` and `libmanatan_android.so`.
- No Flutter project/source or `StartupActivity` implementation exists in this checkout.
- Both public Android and desktop targets depend on `manatan-server-public` pinned at `c0f1c9696bf8bdacfa3663a5b1d863d0a77fc428`.
- `Manatan-Server-Public` declares MPL-2.0 for its wrapper but explicitly downloads target static archives built from a private repository. The backend implementation is therefore opaque and not independently maintainable from the published source.

Conclusion: the release tag is an aligned integration snapshot, not corresponding source for the complete shipping APK. This fork uses the smallest editable baseline—the public Rust/Java shell and WebUI—while replacing account-bound or unavailable manga-specific interfaces as work proceeds. Shipping-Flutter-only behavior is a parity boundary until equivalent editable functionality is implemented and tested.

## Ownership/dependency map

| Area | Classification | Observed implementation / consequence |
| --- | --- | --- |
| Manga reader | Editable source + opaque API | React reader under `WebUI/src/features/reader`; catalogue/page operations cross the opaque server API. |
| Source catalogue/local storage | Opaque binary with editable client | Suwayomi/extension calls are client-visible, but persistence and core routes reside in private static backend. Local Java runtime/JAR is separately downloadable. |
| Komga | Fork-owned adapter + embedded compatibility backend | The fork now owns its authenticated libraries/books/pages/progress adapter; the embedded backend remains available for inherited routes. |
| SMB / WebDAV | Fork-owned adapters | Maintained Rust clients now supply authenticated scan/read behavior without replacing unrelated backend functions. |
| OCR | Editable service + bundled Android recogniser | `crates/ocr-server` and the queue are editable; Android bundles ML Kit Japanese. Network Lens is separate and explicit opt-in. |
| Dictionaries | Editable service/client | `crates/yomitan-server` plus WebUI manager/lookup. Rich-format and archive-safety behavior require tests. |
| Word audio | Editable service/client with external endpoints | `crates/audio-server` and lookup/audio client are editable; availability depends on selected sources. |
| Anki bridge | Editable Java/client | Java content-provider bridge and AnkiConnect-compatible client are public. Existing listener binding to all interfaces is unsafe and will be narrowed/isolated. |
| Statistics | Unavailable for required manga metrics | No complete editable implementation matching the brief has been established. |
| Accounts/membership/sync/SRS | Mixed editable UI and opaque services | Not required by the manga fork and must be removed from startup/navigation/runtime coupling. Mined history must remain local. |

## Licensing notes

- Manatan root declares MIT; many inherited Suwayomi WebUI files carry MPL-2.0 headers. Preserve per-file notices.
- `Manatan-Server-Public` wrapper declares MPL-2.0. The downloaded private archive has no source in the public repository; distribution/rebuild status remains a release risk.
- Hoshi Reader Android is GPL-3.0. It is used only as behavior/design reference in this work; no code has been copied.
- Model/native redistribution must be verified component by component before any public distribution. This task creates no public release.

## Architecture decision

Proceed on the public WebUI/native Android architecture because it preserves the largest editable manga/mining surface without an unrelated rewrite. Retain the published `Manatan-Server-Public` static library as the embedded/local backend dependency used by existing builds and forks. Empirical loopback checks confirm it supplies health, settings/defaults, global metadata, Yomitan dictionary, manga-library and chapter-state routes. The fork-owned APIs narrowly add source scanning, offline jobs, privacy and statistics required by the manga-only design. The archive remains opaque and must not be represented as source-reproducible, but opacity alone is not a reason to reimplement it.

## On-device OCR resolution

The Android wrapper now uses Gradle to merge the signed native APK with bundled `com.google.mlkit:text-recognition-japanese:16.0.1`. The Java bridge performs on-device recognition and returns text blocks with geometry to the durable fork queue. The installed stock APK's model files were not extracted or redistributed. Integration/self-tests and queue recovery pass; representative Japanese manga accuracy and overlay-selection QA remain before final acceptance.

Public Manatan routes pages through the network `chrome_lens_ocr` client and then through its geometry-aware Rust `auto_merge` stage. The fork's first offline implementation returned directly from ML Kit and therefore skipped that merger; 0.2.3 corrects this. Chimahon's current GPL-3.0 local OCR module is an on-device Google Lens integration with native model packs, not MeikiOCR. Its source remains reference-only and the redistribution rights for the Lens binaries/models have not been established. MeikiOCR itself is Apache-2.0 and ONNX-based, but is optimized for video-game text and documents vertical recognition as beta; it is not currently the best justified default for manga.

## Reproducibility findings and fork baseline

- `WebUI/package.json` declared `dashjs` without a matching Yarn v1 lock entry. The lockfile was regenerated with pinned Node 22.12.0/Yarn 1.22.22; the WebUI production build passes. Upstream lint currently reports 11,591 inherited findings, so changed-file checks and the production build remain the meaningful gate until that debt is reduced.
- The workspace claims Rust 1.88, but locked `chrome_lens_ocr` 0.3 requires Rust 1.90. The Android build is therefore pinned to Rust 1.90 on this host.
- The documented `KolbyML/cargo-apk2` Git source is unavailable publicly. Maintained crates.io `cargo-apk2` 1.3.1 is used; newer releases require a Rust toolchain not available on this host.
- The upstream custom Suwayomi v1.0.23 Android JAR URL returns 404. The retained downloaded static backend/JAR inputs remain disclosed opaque compatibility artefacts and are not represented as source-reproducible.
- A baseline APK was built from unchanged application identity and measured as package `com.mangatan.app`, version `0.1.0`, min/target 23/29. It was not installed because it could replace stock.
- The isolated release APK builds and runs as `dev.local.mangareader`, min/target 26/36, ARM64, on loopback ports 14568 and 18765. It uses private/package-scoped data paths and a separate local signing certificate; device package/UID/signature coexistence with stock is verified.
