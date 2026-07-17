#!/usr/bin/env bash
# M4 Task-1 spike: hand-assembled APK (no Gradle on this host).
# aapt2 link -> javac against android.jar -> d8 -> zip in .so -> align + sign.
set -euo pipefail
cd "$(dirname "$0")"

SDK="$HOME/Library/Android/sdk"
BT="$SDK/build-tools/36.0.0"
PLATFORM="$SDK/platforms/android-34/android.jar"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$SDK/ndk/27.1.12297006}"

OUT=build
rm -rf "$OUT" && mkdir -p "$OUT/classes" "$OUT/apk/lib/arm64-v8a"

echo "== rust cdylib (arm64) =="
(cd spike-rs && cargo ndk -t arm64-v8a build --release)
cp spike-rs/target/aarch64-linux-android/release/libspike.so "$OUT/apk/lib/arm64-v8a/"

echo "== aapt2 link (manifest, no resources) =="
"$BT/aapt2" link -o "$OUT/base.apk" --manifest app/AndroidManifest.xml -I "$PLATFORM"

echo "== javac + d8 =="
javac --release 17 -cp "$PLATFORM" -d "$OUT/classes" app/java/dev/jian/spike/MainActivity.java
"$BT/d8" --lib "$PLATFORM" --release --output "$OUT/apk" $(find "$OUT/classes" -name '*.class')

echo "== assemble =="
cp "$OUT/base.apk" "$OUT/unsigned.apk"
(cd "$OUT/apk" && zip -q -r ../unsigned.apk classes.dex lib)

echo "== align + sign (debug keystore) =="
KS="$HOME/.android/debug.keystore"
if [ ! -f "$KS" ]; then
  mkdir -p "$HOME/.android"
  keytool -genkeypair -keystore "$KS" -storepass android -keypass android \
    -alias androiddebugkey -dname CN=Android-Debug -keyalg RSA -keysize 2048 -validity 10000
fi
"$BT/zipalign" -f -p 4 "$OUT/unsigned.apk" "$OUT/aligned.apk"
"$BT/apksigner" sign --ks "$KS" --ks-pass pass:android --key-pass pass:android \
  --out "$OUT/spike.apk" "$OUT/aligned.apk"

echo "OK: $OUT/spike.apk"
