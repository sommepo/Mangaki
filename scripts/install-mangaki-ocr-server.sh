#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
venv="$repo_root/.toolchains/owocr-venv"
python_bin="${PYTHON_BIN:-python3}"
bind="${1:-127.0.0.1}"

"$python_bin" -c 'import ipaddress,sys; a=ipaddress.ip_address(sys.argv[1]); ok=a.is_loopback or (a.version == 4 and a in ipaddress.ip_network("100.64.0.0/10")); raise SystemExit(0 if ok else "Bind address must be loopback or Tailscale 100.64.0.0/10")' "$bind"

mkdir -p "$repo_root/.toolchains"
if [[ ! -x "$venv/bin/python" ]]; then
    "$python_bin" -m venv "$venv"
fi
if ! "$venv/bin/python" -m pip --version >/dev/null 2>&1; then
    echo "The Python venv module did not install pip. Install python3-venv, then rerun." >&2
    exit 1
fi

"$venv/bin/python" -m pip install torch==2.14.0+cpu torchvision==0.29.0+cpu \
    --index-url https://download.pytorch.org/whl/cpu
"$venv/bin/python" -m pip install --no-deps owocr==1.26.8
"$venv/bin/python" -m pip install -r "$repo_root/services/mangaki-ocr/requirements.lock"
detector="$repo_root/.toolchains/comic_text_detector"
detector_patch="$repo_root/services/mangaki-ocr/comic-text-detector-line-threshold.patch"
if [[ ! -d "$detector/.git" ]]; then
    git clone https://github.com/kha-white/comic-text-detector.git "$detector"
fi
git -C "$detector" fetch --depth 1 origin 92ea867bd85a13105ebcf6bb7f7314784ca89902
# Remove our known patch before re-checking the pinned revision. Refuse to
# overwrite any unrelated local toolchain edits.
if git -C "$detector" apply --reverse --check "$detector_patch" >/dev/null 2>&1; then
    git -C "$detector" apply --reverse "$detector_patch"
fi
git -C "$detector" checkout --detach 92ea867bd85a13105ebcf6bb7f7314784ca89902
if ! git -C "$detector" diff --quiet; then
    echo "comic-text-detector has unexpected local edits; refusing to overwrite them" >&2
    exit 1
fi
git -C "$detector" apply --check "$detector_patch"
git -C "$detector" apply "$detector_patch"
PYTHONPATH="$repo_root/.toolchains" "$venv/bin/python" -c 'from owocr.ocr import MangaOcrSegmented; engine=MangaOcrSegmented({"force_cpu": True}); raise SystemExit(0 if engine.available else "OCR model failed to initialise")'

user_home="$(getent passwd "$(id -u)" | cut -d: -f6)"
unit_dir="${XDG_CONFIG_HOME:-$user_home/.config}/systemd/user"
unit_path="$unit_dir/mangaki-ocr.service"
mkdir -p "$unit_dir"
sed \
    -e "s|@@REPO_ROOT@@|$repo_root|g" \
    -e "s|@@PYTHON@@|$venv/bin/python|g" \
    -e "s|@@BIND@@|$bind|g" \
    "$repo_root/services/mangaki-ocr/mangaki-ocr.service.in" > "$unit_path.tmp"
mv "$unit_path.tmp" "$unit_path"
runtime_dir="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
session_bus="${DBUS_SESSION_BUS_ADDRESS:-unix:path=$runtime_dir/bus}"
env XDG_RUNTIME_DIR="$runtime_dir" DBUS_SESSION_BUS_ADDRESS="$session_bus" systemctl --user daemon-reload
env XDG_RUNTIME_DIR="$runtime_dir" DBUS_SESSION_BUS_ADDRESS="$session_bus" systemctl --user enable --now mangaki-ocr.service

echo "Runtime installed at $venv"
echo "Service enabled on http://$bind:7332"
echo "Use a Tailscale ACL to restrict port 7332 to the intended devices."
