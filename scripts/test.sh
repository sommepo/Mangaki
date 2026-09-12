#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$SCRIPT_DIR/android-env.sh"

# Some lean CI/dev hosts ship SQLite's runtime library without the unversioned
# linker name normally supplied by libsqlite3-dev. Keep the compatibility link
# project-local so the test command does not require a system-wide package.
HOST_LIB_DIR="$REPO_ROOT/.toolchains/host-libs"
mkdir -p "$HOST_LIB_DIR"
if [ ! -e "$HOST_LIB_DIR/libsqlite3.so" ]; then
    for SQLITE_RUNTIME in \
        /usr/lib/x86_64-linux-gnu/libsqlite3.so.0 \
        /lib/x86_64-linux-gnu/libsqlite3.so.0
    do
        if [ -e "$SQLITE_RUNTIME" ]; then
            ln -s "$SQLITE_RUNTIME" "$HOST_LIB_DIR/libsqlite3.so"
            break
        fi
    done
fi
if [ -e "$HOST_LIB_DIR/libsqlite3.so" ]; then
    export LIBRARY_PATH="$HOST_LIB_DIR${LIBRARY_PATH:+:$LIBRARY_PATH}"
fi

cd "$REPO_ROOT/WebUI"
yarn eslint \
    src/App.tsx \
    src/Manatan/OCRManager.tsx \
    src/Manatan/components/ImageOverlay.tsx \
    src/Manatan/hooks/useMangaObserver.ts \
    src/features/settings/screens/Settings.tsx \
    src/features/fork/Activity.tsx \
    src/features/fork/BackgroundJobBridge.tsx \
    src/features/fork/ForkApi.ts \
    src/features/fork/ForkReader.tsx \
    src/features/fork/SeriesDetail.tsx \
    src/features/fork/SourceOnboarding.tsx \
    src/features/fork/Stats.tsx \
    src/features/fork/useForkState.ts \
    src/features/library/screens/Library.tsx
yarn build

cd "$REPO_ROOT"
cargo +1.90.0 test --workspace -- --nocapture
