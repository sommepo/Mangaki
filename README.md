# Mangaki

Mangaki is an Android manga reader for Japanese immersion, derived from the public [Manatan](https://github.com/KolbyML/Manatan) source. It keeps Manatan's reader, zoom, OCR-overlay, dictionary lookup, and mining interaction while providing a manga-focused library and a compact, high-contrast interface.

This is an independent community fork. It is not an official Manatan release and is not affiliated with the Manatan maintainers.

## Current features

- Local Android folders and CBZ/ZIP archives through the Storage Access Framework
- SMB, WebDAV, and Komga sources
- Two-column manga and volume libraries with progress and read state
- Continuous vertical and left-to-right/right-to-left readers
- Manatan-compatible pinch zoom, pan, tap zones, page scrubber, and resume position
- On-device Japanese OCR plus an optional self-hosted OCR endpoint
- Tap-to-lookup dictionaries with term and frequency dictionary import
- AnkiDroid and AnkiConnect mining with configurable field mappings
- Bulk OCR/offline operations, categories, hidden-content controls, and background jobs
- Reading sessions grouped by manga with individual and bulk removal
- Credential-free settings backup and restore

See [the user guide](docs/USER_GUIDE.md), [implementation checklist](docs/IMPLEMENTATION_CHECKLIST.md), [acceptance report](docs/ACCEPTANCE_REPORT.md), and [remaining roadmap](docs/ROADMAP.md) for the exact tested boundaries.

## Android build

The supported local release command is:

```sh
scripts/create-signing-key.sh # first build only; back up .secrets privately
scripts/build-android.sh
```

The signed APK is written to:

```text
android-app/app/build/outputs/apk/release/app-release.apk
```

Local Android SDK/NDK/Rust/Gradle tooling can be bootstrapped with the scripts in `scripts/`. Generated toolchains, signing keys, WebUI bundles, device evidence, and APKs are intentionally ignored by Git.

The Android application ID is `dev.local.mangareader`, allowing it to be installed beside stock Manatan. `scripts/install-android.sh` refuses to install an APK with any other application ID.

Proven validation commands are documented in [AGENTS.md](AGENTS.md).

## Optional OCR service

The self-hosted service is intended for a private HTTPS or Tailscale connection and is not required for the offline ML Kit mode. Installation instructions are in [services/mangaki-ocr/README.md](services/mangaki-ocr/README.md).

Reader pages are sent to that endpoint only when the user explicitly selects the self-hosted provider. The included service does not intentionally persist submitted images or recognised text.

## Source and binary boundary

Mangaki retains the published `Manatan-Server-Public` Android static library as an embedded, local compatibility dependency. Its wrapper is public, but the downloaded static archive is built from private backend source and is therefore opaque and not source-reproducible from this repository. It is not an external cloud service.

The fork adds or replaces individual manga-only interfaces where needed; it does not claim to reimplement the complete backend. See [the upstream audit](docs/UPSTREAM_AUDIT.md) for the component and ownership map.

No signing keys, credentials, personal manga, private dictionaries, production Anki data, or prebuilt APK are committed to this repository.

## Licensing and attribution

The repository retains Manatan's upstream history and copyright notices. Root components are distributed under the [MIT License](LICENSE); the WebUI and covered modifications retain the [Mozilla Public License 2.0](WebUI/LICENSE). Third-party dependencies remain under their respective licenses.

Hoshi Reader was used only as a visual reference. Its GPL-3.0 source was not copied into this derivative.
