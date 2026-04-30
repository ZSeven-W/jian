#!/usr/bin/env sh
# install.sh — Linux curl|sh installer for the `jian` CLI.
#
# Usage:
#   curl -sSf https://get.jian.dev/install.sh | sh
#   # or with a pinned version + custom prefix:
#   curl -sSf https://get.jian.dev/install.sh | env JIAN_VERSION=0.0.1 PREFIX=$HOME/.local sh
#
# Behaviour:
#   - Detects host arch (x86_64 / aarch64) and downloads the matching
#     GitHub Release tarball.
#   - Extracts the `jian` binary into `$PREFIX/bin` (default: /usr/local
#     when run as root, $HOME/.local otherwise).
#   - macOS / Windows are out of scope — Homebrew / winget are the
#     supported channels there. Exits with 1 if you try.
#
# No build dependencies. Requires `curl`, `tar`, and `mktemp` — all
# standard on glibc and musl base images.
#
# Exit codes:
#   0   success
#   1   unsupported host (macOS, Windows, BSD, etc. — use Homebrew /
#       winget instead)
#   2   network / extraction failure
#   3   missing required tool (curl, tar)

set -eu

OWNER="ZSeven-W"
REPO="jian"
VERSION="${JIAN_VERSION:-latest}"

err() {
    printf 'jian install: error: %s\n' "$1" >&2
}

need_cmd() {
    if ! command -v "$1" >/dev/null 2>&1; then
        err "missing required tool '$1' — install it via your package manager and re-run"
        exit 3
    fi
}

detect_target() {
    os="$(uname -s)"
    if [ "$os" != "Linux" ]; then
        err "this installer supports Linux only; on $os use Homebrew (macOS) or winget (Windows)"
        exit 1
    fi
    arch="$(uname -m)"
    case "$arch" in
        x86_64 | amd64)   echo "x86_64-unknown-linux-gnu" ;;
        aarch64 | arm64)  echo "aarch64-unknown-linux-gnu" ;;
        *)
            err "unsupported architecture '$arch' — Linux glibc binaries ship for x86_64 and aarch64 only"
            exit 1
            ;;
    esac
}

# Resolve `latest` → an actual tag via the GitHub API. Anonymous reads
# are rate-limited but always sufficient for an install path.
resolve_version() {
    if [ "$VERSION" = "latest" ]; then
        api="https://api.github.com/repos/$OWNER/$REPO/releases/latest"
        if ! payload="$(curl -sSf "$api")"; then
            err "could not query latest release from $api"
            exit 2
        fi
        # Extract `"tag_name": "vX.Y.Z"` without depending on jq.
        VERSION="$(printf '%s' "$payload" | sed -n 's/.*"tag_name": *"v\([^"]*\)".*/\1/p' | head -1)"
        if [ -z "$VERSION" ]; then
            err "could not parse tag_name from GitHub API response"
            exit 2
        fi
    fi
}

choose_prefix() {
    if [ -n "${PREFIX:-}" ]; then
        echo "$PREFIX"
    elif [ "$(id -u)" = "0" ]; then
        echo "/usr/local"
    else
        echo "$HOME/.local"
    fi
}

main() {
    need_cmd curl
    need_cmd tar
    need_cmd mktemp

    target="$(detect_target)"
    resolve_version
    prefix="$(choose_prefix)"

    archive="jian-${VERSION}-${target}.tar.gz"
    url="https://github.com/$OWNER/$REPO/releases/download/v${VERSION}/${archive}"

    printf 'jian install: downloading %s\n' "$url"
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT INT HUP TERM

    if ! curl -sSfL -o "$tmp/$archive" "$url"; then
        err "download failed (network issue or release artefact missing)"
        exit 2
    fi
    if ! tar -xzf "$tmp/$archive" -C "$tmp"; then
        err "tar extraction failed"
        exit 2
    fi
    if [ ! -x "$tmp/jian" ]; then
        err "tarball did not contain a 'jian' binary at the top level"
        exit 2
    fi

    mkdir -p "$prefix/bin"
    install -m 0755 "$tmp/jian" "$prefix/bin/jian"

    printf 'jian install: installed %s/bin/jian\n' "$prefix"
    case ":$PATH:" in
        *":$prefix/bin:"*) ;;
        *) printf 'jian install: NOTE — add %s/bin to your PATH (e.g. export PATH="%s/bin:$PATH" in ~/.profile)\n' "$prefix" "$prefix" ;;
    esac
    "$prefix/bin/jian" --version
}

main
