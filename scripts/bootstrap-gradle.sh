#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$SCRIPT_DIR/android-env.sh"

VERSION=8.13
EXPECTED=20f1b1176237254a6fc204d8434196fa11a4cfb387567519c61556e8710aed78
ARCHIVE="$REPO_ROOT/.toolchains/gradle-$VERSION-bin.zip"
INSTALL="$REPO_ROOT/.toolchains/gradle-$VERSION"

if [ -x "$INSTALL/bin/gradle" ]; then
    exit 0
fi

mkdir -p "$REPO_ROOT/.toolchains"
if [ ! -f "$ARCHIVE" ]; then
    curl -fL "https://services.gradle.org/distributions/gradle-$VERSION-bin.zip" -o "$ARCHIVE"
fi
printf '%s  %s\n' "$EXPECTED" "$ARCHIVE" | sha256sum -c -
unzip -q "$ARCHIVE" -d "$REPO_ROOT/.toolchains"
