#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$SCRIPT_DIR/android-env.sh"

mkdir -p "$REPO_ROOT/.secrets"
KEYSTORE="$REPO_ROOT/.secrets/manga-reader-fork.keystore"
PASSWORD="manga-reader-local-acceptance-key"

if [ ! -e "$KEYSTORE" ]; then
    keytool -genkeypair -noprompt -keystore "$KEYSTORE" -storepass "$PASSWORD" \
        -keypass "$PASSWORD" -alias manga-reader-fork -keyalg RSA -keysize 3072 \
        -validity 10000 -dname "CN=Manga Reader Local Acceptance, OU=Local Development, O=Local, C=GB"
fi

umask 077
{
    echo "export CARGO_APK_DEV_KEYSTORE='$KEYSTORE'"
    echo "export CARGO_APK_DEV_KEYSTORE_PASSWORD='$PASSWORD'"
    echo "export CARGO_APK_RELEASE_KEYSTORE='$KEYSTORE'"
    echo "export CARGO_APK_RELEASE_KEYSTORE_PASSWORD='$PASSWORD'"
} > "$REPO_ROOT/.secrets/signing.env"

echo "Created local fork signing material at $KEYSTORE"
echo "Back up this file to retain update compatibility; do not commit it."
