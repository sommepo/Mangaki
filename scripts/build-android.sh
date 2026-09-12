#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$SCRIPT_DIR/android-env.sh"

SIGNING_ENV="$REPO_ROOT/.secrets/signing.env"
if [ ! -r "$SIGNING_ENV" ]; then
    echo "Missing $SIGNING_ENV; run scripts/create-signing-key.sh first." >&2
    exit 1
fi
. "$SIGNING_ENV"

cd "$REPO_ROOT/WebUI"
yarn install --frozen-lockfile
yarn build

mkdir -p "$REPO_ROOT/bin/manatan_android/assets"
tar -cf "$REPO_ROOT/bin/manatan_android/assets/webui.tar" -C "$REPO_ROOT/WebUI/build" .

cd "$REPO_ROOT/bin/manatan_android"
cargo +1.90.0 apk2 build --release --target aarch64-linux-android

"$SCRIPT_DIR/bootstrap-gradle.sh"
cd "$REPO_ROOT/android-app"
"$REPO_ROOT/.toolchains/gradle-8.13/bin/gradle" --no-daemon :app:assembleRelease

APK="$REPO_ROOT/android-app/app/build/outputs/apk/release/app-release.apk"
"$ANDROID_HOME/build-tools/36.0.0/aapt" dump badging "$APK" | sed -n '1,4p'
sha256sum "$APK"
