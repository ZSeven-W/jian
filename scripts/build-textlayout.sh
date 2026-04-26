#!/usr/bin/env bash
# Build (or test) jian-skia with the `textlayout` feature, prepending a
# Python 3.10 / 3.11 / 3.12 `python3` onto PATH so depot_tools' `ninja`
# wrapper can run.
#
# Why: skia-bindings 0.78's depot_tools imports the stdlib `pipes`
# module, which Python 3.13 removed. macOS / Homebrew's default
# `python3` is 3.13+; we need 3.10–3.12 in scope just for this build.
#
# Discovery order (first hit wins):
#   1. Homebrew kegs under /opt/homebrew (Apple Silicon) and
#      /usr/local (Intel) for python@3.{11,12,10}.
#   2. Generic `python3.{11,12,10}` already on PATH (Linux, Nixpkgs,
#      pyenv-installed shims, etc.). The script wraps that binary in
#      a temp `python3` symlink directory so the depot_tools wrapper
#      finds it as `python3`.
#
# Forward all arguments verbatim — typical use:
#   scripts/build-textlayout.sh build -p jian-skia
#   scripts/build-textlayout.sh test  -p jian-skia
#   scripts/build-textlayout.sh test  --workspace
#
# The script always appends `--features textlayout` to the cargo
# invocation. Don't pass it twice in your own arguments — cargo
# deduplicates but the helper is intentionally feature-enforcing,
# not a transparent pass-through.
#
# Maintenance: delete this script once skia-bindings ships a
# depot_tools snapshot that drops the `pipes` import.
set -euo pipefail

PY_BIN=""

# Tier 1: Homebrew kegs (covers macOS Apple Silicon + Intel).
for keg in \
    "/opt/homebrew/opt/python@3.11/libexec/bin" \
    "/opt/homebrew/opt/python@3.12/libexec/bin" \
    "/opt/homebrew/opt/python@3.10/libexec/bin" \
    "/usr/local/opt/python@3.11/libexec/bin" \
    "/usr/local/opt/python@3.12/libexec/bin" \
    "/usr/local/opt/python@3.10/libexec/bin"
do
    if [ -x "$keg/python3" ]; then
        PY_BIN="$keg"
        break
    fi
done

# Tier 2: PATH lookup for `python3.X` (Linux distros, pyenv, etc.).
# Materialise a temp directory with a `python3` → python3.X symlink
# so the depot_tools `ninja` wrapper's `exec python3 …` resolves.
if [ -z "$PY_BIN" ]; then
    for ver in 3.11 3.12 3.10; do
        if cand="$(command -v "python$ver" || true)" && [ -n "$cand" ]; then
            tmp="$(mktemp -d)"
            ln -s "$cand" "$tmp/python3"
            PY_BIN="$tmp"
            # Best-effort cleanup; not critical if cargo crashes.
            trap 'rm -rf "$PY_BIN"' EXIT
            break
        fi
    done
fi

if [ -z "$PY_BIN" ]; then
    cat >&2 <<'EOF'
build-textlayout.sh: no Python 3.10 / 3.11 / 3.12 'python3' in scope.
  macOS:  brew install python@3.11
  Ubuntu: sudo apt install python3.11
  Other:  install via pyenv / package manager and ensure
          `python3.11` (or 3.10 / 3.12) is on PATH.
EOF
    exit 2
fi

echo "build-textlayout.sh: using $PY_BIN/python3 ($("$PY_BIN/python3" --version 2>&1))"

# Insert `--features textlayout` *right after* the subcommand
# (e.g. `build` / `test` / `clippy`) so it lands before any
# subcommand-specific `--` separator. Trailing-append worked for
# build / test but tripped clippy because `--features` after `--`
# routes to clippy-driver instead of cargo.
PATH="$PY_BIN:$PATH" exec cargo "$1" --features textlayout "${@:2}"
