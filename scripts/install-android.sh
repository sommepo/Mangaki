#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$SCRIPT_DIR/android-env.sh"

APK=${1:-"$REPO_ROOT/android-app/app/build/outputs/apk/release/app-release.apk"}
PACKAGE=$("$ANDROID_HOME/build-tools/36.0.0/aapt" dump badging "$APK" | sed -n "s/^package: name='\([^']*\)'.*/\1/p")
if [ "$PACKAGE" != "dev.local.mangareader" ]; then
    echo "Refusing to install APK with application ID '$PACKAGE'." >&2
    exit 1
fi

SERIALS=$(adb devices | awk 'NR > 1 && $2 == "device" {print $1}')
COUNT=$(printf '%s\n' "$SERIALS" | awk 'NF {count++} END {print count+0}')
if [ "$COUNT" -ne 1 ]; then
    echo "Expected exactly one authorised adb device; found $COUNT." >&2
    adb devices -l >&2
    exit 1
fi
SERIAL=$(printf '%s\n' "$SERIALS" | awk 'NF {print; exit}')

adb -s "$SERIAL" install -r "$APK"
# Package replacement can return before an OEM launcher observes the new process.
sleep 2
adb -s "$SERIAL" shell monkey -p dev.local.mangareader -c android.intent.category.LAUNCHER 1
