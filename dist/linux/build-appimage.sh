#!/usr/bin/env bash
# Build a Linux AppImage from the release binary at
# `target/<TARGET>/release/jian`. Requires `linuxdeploy`
# (https://github.com/linuxdeploy/linuxdeploy/releases) on PATH.
#
# Usage:
#   ./dist/linux/build-appimage.sh [TARGET]
#
# TARGET defaults to `x86_64-unknown-linux-gnu`. Outputs
# `target/<TARGET>/release/Jian-<VERSION>-x86_64.AppImage`.
#
# The release CI workflow (.github/workflows/release.yml) currently
# packages `.tar.gz` archives only; AppImage is opt-in for users
# who want the self-contained format. The script also installs
# `dist/linux/jian.desktop` + `jian.mime.xml` into the AppDir so
# file double-click and `jian://` URL handling work after
# `appimaged` registers the bundle.

set -euo pipefail

TARGET="${1:-x86_64-unknown-linux-gnu}"
ROOT="$(cd "$(dirname "$0")"/../.. && pwd)"
BIN="$ROOT/target/$TARGET/release/jian"
VERSION="$(awk -F\" '/^version[[:space:]]*=/ {print $2; exit}' "$ROOT/Cargo.toml")"

if [ ! -x "$BIN" ]; then
    echo "build-appimage: missing release binary at $BIN" >&2
    echo "  run \`cargo build --release -p jian --target $TARGET\` first" >&2
    exit 1
fi

if ! command -v linuxdeploy >/dev/null 2>&1; then
    echo "build-appimage: linuxdeploy not on PATH; install from" >&2
    echo "  https://github.com/linuxdeploy/linuxdeploy/releases" >&2
    exit 1
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

APPDIR="$WORK/AppDir"
mkdir -p "$APPDIR/usr/bin"
mkdir -p "$APPDIR/usr/share/mime/packages"
mkdir -p "$APPDIR/usr/share/applications"

cp "$BIN" "$APPDIR/usr/bin/jian"
cp "$ROOT/dist/linux/jian.desktop" "$APPDIR/usr/share/applications/"
cp "$ROOT/dist/linux/jian.mime.xml" "$APPDIR/usr/share/mime/packages/"
# Icon: the release pipeline drops a 256x256 PNG at `dist/icon/jian.png`
# before invoking this script. We require a real PNG file rather than
# attempting to synthesise one — `linuxdeploy --icon-file` validates
# the bytes and rejects malformed input, and silently writing a 1x1
# placeholder masks a configuration mistake the release script
# should surface.
ICON_SRC="$ROOT/dist/icon/jian.png"
if [ ! -f "$ICON_SRC" ]; then
    echo "build-appimage: missing icon at $ICON_SRC" >&2
    echo "  drop a 256x256 PNG there before re-running, or wire the" >&2
    echo "  release pipeline to generate one." >&2
    exit 1
fi
cp "$ICON_SRC" "$APPDIR/usr/share/applications/jian.png"

cd "$WORK"
ARCH="$(uname -m)"
linuxdeploy \
    --appdir "$APPDIR" \
    --desktop-file "$APPDIR/usr/share/applications/jian.desktop" \
    --icon-file "$APPDIR/usr/share/applications/jian.png" \
    --output appimage

OUT_NAME="Jian-${VERSION}-${ARCH}.AppImage"
mv ./*.AppImage "$ROOT/target/$TARGET/release/$OUT_NAME"
echo "build-appimage: wrote target/$TARGET/release/$OUT_NAME"
