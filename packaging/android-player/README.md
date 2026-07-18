# JianPlayer (Android)

A thin Android host for the jian engine, consuming `crates/jian-jni`'s
`dev.jian.player.JianNative` C-ABI surface. The engine renders through EGL/GLES
onto a `SurfaceView`; the shell owns lifecycle, insets, touch, IME, and
capabilities.

## Build + install

Requires the Android SDK + NDK and `cargo-ndk` (`cargo install cargo-ndk`).
Uses the Gradle wrapper (Gradle 8.14.3); point Gradle at a JDK 17+ (Android
Studio's bundled JBR works):

```bash
export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"

# ACCEPTANCE / debug (keeps the debug-hooks fault seams alive) — build the
# cdylib into jniLibs, then install immediately:
cargo ndk -t arm64-v8a -t x86_64 -o packaging/android-player/app/src/main/jniLibs \
  build -p jian-jni --features gl,textlayout,debug-hooks
cd packaging/android-player && ./gradlew installDebug && cd -

# SHIPPING (mutually exclusive — overwrites jniLibs; never before an acceptance install):
cargo ndk -t arm64-v8a -t x86_64 -o packaging/android-player/app/src/main/jniLibs \
  build -p jian-jni --features gl,textlayout --release
cd packaging/android-player && ./gradlew assembleRelease && cd -
```

## Run

```bash
adb shell am start -n dev.jian.player/.MainActivity              # default doc (m1_acceptance)
adb shell am start -n dev.jian.player/.MainActivity --es doc m4_media
adb shell am start -n dev.jian.player/.MainActivity --ez noAssetBase true   # absent asset base
adb logcat -s JianPlayer:V JianJni:V AndroidRuntime:E libEGL:W
```

## Status

- **Phase A (rendering pipeline): implemented** — `JianNative` contract,
  `JianSurfaceView` (create/attach/resume/suspend/resize, Choreographer frame
  pump, touch), `JianCallbacksImpl` (frame pump), `MainActivity` (edge-to-edge,
  real inset path, asset extraction, `nativeDestroy` on destroy), font
  registration.
- **Phase B (pending):** `JianInputConnection` (full IME), `JianCapabilities`
  (HTTP/confirm/open-url), `JianDebugReceiver` (broadcast fault/edge hooks),
  the `m4_media`/`corrupt.op` fixtures, and the launcher icon.
