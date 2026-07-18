#!/usr/bin/env bash
# M4 Android acceptance (Task 7). Drives the JianPlayer debug build on a
# connected device/emulator and asserts the §6.7 lifecycle, the fault seams,
# and the deterministic IME harnesses from logcat.
#
#   bash scripts/m4-acceptance.sh            # build + install + run everything
#   SKIP_BUILD=1 bash scripts/m4-acceptance.sh
#
# Requires: adb on PATH or ANDROID_HOME set, a booted device/emulator,
# cargo-ndk + JAVA_HOME (a JDK 17+, e.g. Android Studio's JBR) when building.
set -uo pipefail
cd "$(dirname "$0")/.."

ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
ADB="${ADB:-$ANDROID_HOME/platform-tools/adb}"
PKG=dev.jian.player
PLAYER=packaging/android-player
ENDPOINT_PORT=8477

pass=0; fail=0
ok()   { echo "  PASS  $*"; pass=$((pass+1)); }
bad()  { echo "  FAIL  $*"; fail=$((fail+1)); }
step() { echo; echo "== $* =="; }
logs() { "$ADB" logcat -d -s JianPlayer:V JianJni:V AndroidRuntime:E 2>/dev/null; }
clear_logs() { "$ADB" logcat -c 2>/dev/null; }
launch() { # launch <doc>
  "$ADB" shell am force-stop "$PKG" >/dev/null 2>&1
  clear_logs
  "$ADB" shell am start -n "$PKG/.MainActivity" --es doc "$1" >/dev/null 2>&1
  wait_log "engine created" 40 || true
}
bcast() { "$ADB" shell am broadcast -a "dev.jian.player.$1" "${@:2}" >/dev/null 2>&1; sleep 2; }
alive() { [ -n "$("$ADB" shell pidof "$PKG" 2>/dev/null | tr -d '\r')" ]; }
# wait_log <extended-regex> [seconds]: poll logcat instead of sleeping a fixed
# amount. A loaded host makes engine startup and the fetch round-trip take
# several seconds longer than any blind sleep worth writing, and a too-short
# sleep turns into a phantom failure of an assertion that would have passed.
wait_log() {
  local pattern="$1" limit="${2:-30}" waited=0
  while [ "$waited" -lt "$limit" ]; do
    logs | grep -qE "$pattern" && return 0
    sleep 1; waited=$((waited+1))
  done
  return 1
}

step "Preflight"
"$ADB" get-state >/dev/null 2>&1 || { echo "no device/emulator"; exit 1; }
echo "device: $("$ADB" shell getprop ro.product.model 2>/dev/null | tr -d '\r')"

if [ -z "${SKIP_BUILD:-}" ]; then
  step "Build (debug: keeps the debug-hooks fault seams alive)"
  export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$ANDROID_HOME/ndk/27.1.12297006}"
  cargo ndk -t arm64-v8a -t x86_64 -o "$PLAYER/app/src/main/jniLibs" \
    build -p jian-jni --features gl,textlayout,debug-hooks >/dev/null 2>&1 \
    && ok "cdylib built" || { bad "cdylib build"; exit 1; }
  (cd "$PLAYER" && ./gradlew assembleDebug --no-daemon >/dev/null 2>&1) \
    && ok "APK assembled" || { bad "gradle assembleDebug"; exit 1; }
fi
"$ADB" install -r "$PLAYER/app/build/outputs/apk/debug/app-debug.apk" >/dev/null 2>&1 \
  && ok "installed" || bad "install"

step "Test endpoint (adb reverse :$ENDPOINT_PORT)"
pkill -f m4-test-endpoint.py 2>/dev/null
nohup python3 scripts/m4-test-endpoint.py >/tmp/m4-endpoint.log 2>&1 &
sleep 2
"$ADB" reverse "tcp:$ENDPOINT_PORT" "tcp:$ENDPOINT_PORT" >/dev/null 2>&1 \
  && ok "reverse forwarded" || bad "adb reverse"

step "A. First frame — responsive variant + viewport width (m1_acceptance)"
launch m1_acceptance
logs | grep -q "engine created" && ok "engine created" || bad "engine created"
logs | grep -q "JianJni : window acquired" && ok "window acquired on the engine thread" || bad "window acquired"
alive && ok "process alive" || bad "process died"

step "B. §6.7 lifecycle — suspend releases the window, resume re-acquires, engine survives"
pid_before=$("$ADB" shell pidof "$PKG" | tr -d '\r')
clear_logs
"$ADB" shell input keyevent KEYCODE_HOME >/dev/null 2>&1
wait_log "window released" 30 || true
logs | grep -q "window released" && ok "surfaceDestroyed → suspend released the window" || bad "no window release on suspend"
clear_logs
"$ADB" shell am start -n "$PKG/.MainActivity" >/dev/null 2>&1
wait_log "window acquired" 40 || true
logs | grep -q "window acquired" && ok "resume re-acquired the window" || bad "no re-acquire on resume"
pid_after=$("$ADB" shell pidof "$PKG" | tr -d '\r')
[ "$pid_before" = "$pid_after" ] && ok "engine survived (same pid $pid_after)" || bad "process restarted ($pid_before → $pid_after)"

step "C. Fault seams (debug-hooks)"
clear_logs; bcast THROW_UPCALL
alive && ok "THROW_UPCALL: upcall threw, trampoline cleared, process survived" || bad "THROW_UPCALL killed the process"
# LOSE_CONTEXT first: the recovery budget is one attempt per surface
# generation, and FAIL_NEXT_ATTACH leaves the engine surface-less (its next
# frame consumes the budget), so running it first would mask the recovery.
clear_logs; bcast LOSE_CONTEXT; sleep 2
logs | grep -q "GpuError → suspend/resume recovery" && ok "LOSE_CONTEXT: GpuError → suspend/resume recovery" || bad "no GpuError recovery"
alive && ok "process alive after context loss" || bad "process died after context loss"
# Re-launch for a fresh surface generation, then the destructive seam.
"$ADB" shell am start -n "$PKG/.MainActivity" >/dev/null 2>&1
wait_log "window acquired" 40 || true
clear_logs; bcast FAIL_NEXT_ATTACH
acq=$(logs | grep -c "window acquired"); rel=$(logs | grep -c "window released")
[ "$acq" -ge 1 ] && [ "$rel" -ge 1 ] && ok "FAIL_NEXT_ATTACH: post-acquisition failure released the window (acq=$acq rel=$rel)" \
  || bad "FAIL_NEXT_ATTACH pairing (acq=$acq rel=$rel)"
alive && ok "process alive after failed attach" || bad "process died after failed attach"

step "D. m4_media — asset base, capability gate, image fetch, timeout guard"
launch m4_media
wait_log "result [0-9]+ kind=4 .* status=0" 40 || true
logs | grep -q "media/absent.png\`: No such file" && ok "absent image resolved against the extracted asset base" || bad "absent-image resolution"
logs | grep -q "network capability denied" && bad "capability gate denied (is app.capabilities missing?)" || ok "declared network capability opened the gate"
logs | grep -qE "capability request [0-9]+ kind=4" && ok "image-fetch capability requests marshalled" || bad "no image-fetch requests"
logs | grep -qE "result [0-9]+ kind=4 ok=true .* status=0" && ok "fetch result delivered and accepted (status=0)" || bad "fetch result not accepted"
logs | grep -q "no authored timeout, using 30000ms guard" && ok "un-authored timeout → 30s guard (cross-host rule)" || bad "30s guard not applied"
# Accepted bytes are not a painted image: the engine used to consume the
# resolver completion one pump AFTER the frame the host had been asked for, so
# a delivered image stayed a placeholder until unrelated input woke a frame.
# `img-remote-ok` is 64x48 at (180,20) logical, dpr 2.625, served solid
# #1E66C8 by the test endpoint.
"$ADB" exec-out screencap -p > /tmp/m4-frame.png 2>/dev/null
remote_rgb=$(python3 scripts/m4-pixel.py /tmp/m4-frame.png 555 160 2>/dev/null)
[ "$remote_rgb" = "30,102,200" ] \
  && ok "fetched remote image PAINTED without further input (RGB $remote_rgb)" \
  || bad "remote image not on screen (RGB ${remote_rgb:-unreadable}, want 30,102,200)"

step "E. Deterministic IME harnesses (assert via nativeTextGetState)"
"$ADB" shell input tap 200 260 >/dev/null 2>&1; sleep 2   # focus the long field
clear_logs
bcast IME_QUERY_TEST                    # first: needs the untouched long field
bcast IME_DELETE_TEST
bcast IME_DELETE_TEST --ez codepoints true
bcast IME_FINISH_TEST
bcast IME_FINISH_TEST --ez outside true
bcast BATCH_RESTART_TEST
bcast IME_KEY_TEST                      # backspace as a KEY EVENT (real-IME path)
sleep 2
while read -r line; do
  case "$line" in
    *"IME_TEST"*PASS*) ok "${line#*IME_TEST }";;
    *"IME_TEST"*FAIL*) bad "${line#*IME_TEST }";;
    *"IME_TEST"*SKIP*) bad "${line#*IME_TEST }";;
  esac
done < <(logs | grep "IME_TEST")

step "F. LOAD_DOC full recreate + teardown"
clear_logs; bcast LOAD_DOC --es name m1_acceptance; sleep 2
logs | grep -q "engine created" && ok "LOAD_DOC recreated the engine (destroy → create → attach)" || bad "LOAD_DOC recreate"
alive && ok "process alive after recreate" || bad "process died on recreate"

echo
echo "================ M4 acceptance: $pass passed, $fail failed ================"
pkill -f m4-test-endpoint.py 2>/dev/null
[ "$fail" -eq 0 ]
