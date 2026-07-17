#!/usr/bin/env bash
# Pinned §6.8 Android build gates (M4). `.github/workflows/ci.yml` is a
# protected concurrent-session file — this script is the interim gate until
# the coordinated CI edit lands.
set -euo pipefail
cd "$(dirname "$0")/.."
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$HOME/Library/Android/sdk/ndk/27.1.12297006}"
cargo ndk -t arm64-v8a check -p jian-skia -p jian-engine-ffi -p jian-jni --features jian-skia/gl,jian-skia/textlayout,jian-engine-ffi/gl,jian-engine-ffi/textlayout,jian-jni/gl,jian-jni/textlayout
cargo ndk -t x86_64  check -p jian-skia -p jian-engine-ffi -p jian-jni --features jian-skia/gl,jian-skia/textlayout,jian-engine-ffi/gl,jian-engine-ffi/textlayout,jian-jni/gl,jian-jni/textlayout
# `check` never links: one ABI must BUILD the cdylib so the EGL/GLESv2/android
# link directives, symbol resolution, and shared-library production are
# actually validated.
cargo ndk -t arm64-v8a build -p jian-jni --features jian-jni/gl,jian-jni/textlayout
echo "check-android: OK"
