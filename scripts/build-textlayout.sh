#!/usr/bin/env bash
# Build (or test) jian-skia with the `textlayout` feature, prepending
# Python 3.11 onto PATH so depot_tools' `ninja` wrapper can run.
#
# Why: skia-bindings 0.78's depot_tools imports the stdlib `pipes`
# module, which Python 3.13 removed. macOS / Homebrew's default
# `python3` is 3.13+; we need 3.10–3.12 in scope just for this build.
#
# Forward all arguments verbatim — typical use:
#   scripts/build-textlayout.sh build -p jian-skia
#   scripts/build-textlayout.sh test  -p jian-skia
#   scripts/build-textlayout.sh test  --workspace
set -euo pipefail

PY311_BIN=""
for candidate in \
    "/opt/homebrew/opt/python@3.11/libexec/bin" \
    "/opt/homebrew/opt/python@3.10/libexec/bin" \
    "/opt/homebrew/opt/python@3.12/libexec/bin" \
    "/usr/local/opt/python@3.11/libexec/bin" \
    "/usr/local/opt/python@3.10/libexec/bin"
do
    if [ -x "$candidate/python3" ]; then
        PY311_BIN="$candidate"
        break
    fi
done

if [ -z "$PY311_BIN" ]; then
    echo >&2 "build-textlayout.sh: no Python 3.10/3.11/3.12 'python3' shim found in /opt/homebrew or /usr/local."
    echo >&2 "  Install with: brew install python@3.11"
    exit 2
fi

echo "build-textlayout.sh: using $PY311_BIN/python3 ($($PY311_BIN/python3 --version))"

PATH="$PY311_BIN:$PATH" exec cargo "$@" --features textlayout
