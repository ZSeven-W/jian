#!/usr/bin/env bash
# Generate a macOS `.icns` icon set from a single source PNG (Plan 8
# §T10 / C4).
#
# `cargo bundle` reads `[package.metadata.bundle].icon` from
# `crates/jian-cli/Cargo.toml`, which points at
# `packaging/icon/AppIcon.icns`. The release pipeline drops a 1024×1024
# RGBA PNG named `packaging/icon/AppIcon-source.png`; this script
# produces the multi-resolution `.icns` Apple's launcher expects.
#
# Apple's reference table:
#   icon_16x16.png       16×16
#   icon_16x16@2x.png    32×32
#   icon_32x32.png       32×32
#   icon_32x32@2x.png    64×64
#   icon_128x128.png     128×128
#   icon_128x128@2x.png  256×256
#   icon_256x256.png     256×256
#   icon_256x256@2x.png  512×512
#   icon_512x512.png     512×512
#   icon_512x512@2x.png  1024×1024
#
# Requires `sips` (macOS only) and `iconutil` (also macOS only). On a
# non-macOS dev machine, skip — CI runs this on the macos-aarch64
# matrix. Linux / Windows packagers don't need this script: they
# consume `packaging/icon/AppIcon-source.png` directly via cargo-bundle
# (Linux .png embedding) or `cargo wix` (which converts to .ico itself).

set -euo pipefail

ROOT="$(cd "$(dirname "$0")"/../.. && pwd)"
SRC="${1:-$ROOT/packaging/icon/AppIcon-source.png}"
OUT="$ROOT/packaging/icon/AppIcon.icns"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "generate-icns: requires macOS (sips + iconutil)." >&2
    echo "  CI runs this on the macos-aarch64 release matrix." >&2
    exit 1
fi

if [[ ! -f "$SRC" ]]; then
    echo "generate-icns: missing source PNG at $SRC" >&2
    echo "  drop a 1024×1024 RGBA PNG there and re-run." >&2
    exit 1
fi

ICONSET="$WORK/AppIcon.iconset"
mkdir -p "$ICONSET"

# Apple's documented table. `sips -z H W` resizes to height × width.
sizes=(
    "16:icon_16x16.png"
    "32:icon_16x16@2x.png"
    "32:icon_32x32.png"
    "64:icon_32x32@2x.png"
    "128:icon_128x128.png"
    "256:icon_128x128@2x.png"
    "256:icon_256x256.png"
    "512:icon_256x256@2x.png"
    "512:icon_512x512.png"
    "1024:icon_512x512@2x.png"
)
for entry in "${sizes[@]}"; do
    px="${entry%%:*}"
    name="${entry#*:}"
    sips -z "$px" "$px" "$SRC" --out "$ICONSET/$name" >/dev/null
done

iconutil -c icns "$ICONSET" -o "$OUT"
echo "generate-icns: wrote $OUT ($(wc -c < "$OUT") bytes)"
