#!/usr/bin/env bash
# Pinned §6.8 Android build gates (M4). `.github/workflows/ci.yml` is a
# protected concurrent-session file — this script is the interim gate until
# the coordinated CI edit lands.
set -euo pipefail
cd "$(dirname "$0")/.."
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$HOME/Library/Android/sdk/ndk/27.1.12297006}"
cargo ndk -t arm64-v8a check -p jian-skia -p jian-engine-ffi -p jian-jni --features jian-skia/gl,jian-skia/textlayout,jian-engine-ffi/gl,jian-engine-ffi/textlayout,jian-jni/gl,jian-jni/textlayout
cargo ndk -t x86_64  check -p jian-skia -p jian-engine-ffi -p jian-jni --features jian-skia/gl,jian-skia/textlayout,jian-engine-ffi/gl,jian-engine-ffi/textlayout,jian-jni/gl,jian-jni/textlayout
# `check` never links: one ABI must BUILD a cdylib that actually references
# the EGL path so the EGL/GLESv2/android link directives, symbol resolution,
# and shared-library production are validated. jian-engine-ffi is that
# cdylib — its Android lifecycle arms own an EglSurface, so the exported C
# ABI retains the EGL code and libjian_engine_ffi.so links libEGL/libGLESv2.
cargo ndk -t arm64-v8a build -p jian-engine-ffi --features jian-engine-ffi/gl,jian-engine-ffi/textlayout
SO="target/aarch64-linux-android/debug/libjian_engine_ffi.so"
if ! "${ANDROID_NDK_HOME}"/toolchains/llvm/prebuilt/*/bin/llvm-readelf -d "$SO" 2>/dev/null | grep -q 'libEGL.so'; then
  echo "check-android: FAIL — $SO does not link libEGL.so (EGL path not exercised)" >&2
  exit 1
fi
echo "check-android: OK"
