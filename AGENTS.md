# Manga Reader fork working agreement

The product authority is [`Manga_Reader_Codex_Implementation_Brief.md`](Manga_Reader_Codex_Implementation_Brief.md). Do not weaken or silently omit a requirement.

## Proven commands

- Upstream identity: `git describe --tags --exact-match upstream/master`
- Web checks: `cd WebUI && yarn lint && yarn build`
- Rust checks: `cargo test --workspace -- --nocapture`
- Web bundle: `cd WebUI && yarn build`
- Release APK: `scripts/build-android.sh`
- APK identity: `.toolchains/android-sdk/build-tools/36.0.0/aapt dump badging <apk>`
- Device discovery: `.toolchains/android-sdk/platform-tools/adb devices -l`
- Guarded install: `scripts/install-android.sh [apk]`

Toolchains and signing material stay in `.toolchains/` and `.secrets/` and must not be committed. The signing key is generated with `scripts/create-signing-key.sh` and must be backed up privately to preserve fork update compatibility.

## Ownership boundaries

- Editable: this repository's Rust crates, Java Android wrapper, and React WebUI.
- Opaque: the static `manatan-server` archive downloaded by `Manatan-Server-Public`; it may be used only as a temporary compatibility dependency and must not be represented as source-reproducible.
- Reference only: Hoshi Reader Android GPL-3.0 code. Do not copy it into this MIT/MPL derivative without an explicit licensing decision.
- User data and credentials are never fixtures, build inputs, logs, screenshots, or committed files.

## Device safety

- Resolve one authorized device serial, then pass `-s <serial>` to every device command.
- Stock `com.mangatan.app` is read-only: never clear, uninstall, downgrade, replace, run-as, or mutate its files/settings.
- Install only `dev.local.mangareader`; verify the APK application ID before install.
- Never touch production Anki notes. Functional writes use an explicitly selected test deck and identifiable test tag.
- Do not use `adb forward --remove-all`, reboot, root, or broad package/file deletion.
