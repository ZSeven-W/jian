package dev.jian.player

import android.view.Surface

/**
 * The C-ABI engine surface, marshalled by `crates/jian-jni` (Task 5). Every
 * `external` method dispatches onto the engine's dedicated thread; a
 * closed/unknown handle returns [STATUS_CLOSING]. Loaded from `libjian_jni.so`
 * (packaged into jniLibs by `cargo ndk`).
 */
object JianNative {
    const val STATUS_CLOSING = -1

    init {
        System.loadLibrary("jian_jni")
    }

    external fun nativeCreate(
        doc: ByteArray,
        w: Float,
        h: Float,
        dpr: Float,
        storageDir: String,
        assetBase: String?, // null = NO asset base
        receiver: JianCallbacks,
    ): Long // 0 = failure

    external fun nativeLastError(engine: Long): String
    external fun nativeAttachSurface(engine: Long, surface: Surface): Int
    external fun nativeSuspend(engine: Long): Int // blocking barrier
    external fun nativeResume(engine: Long, surface: Surface?): Int
    external fun nativeResize(engine: Long, w: Float, h: Float, dpr: Float): Int
    external fun nativeSetSafeArea(engine: Long, t: Float, r: Float, b: Float, l: Float): Int
    external fun nativeSetKeyboard(engine: Long, h: Float): Int
    external fun nativeFrame(engine: Long, tMs: Long): Int // blocking barrier; the TRUE frame status
    external fun nativePointer(engine: Long, id: Int, phase: Int, x: Float, y: Float, tMs: Long): Int
    external fun nativeTextInsert(engine: Long, text: String): Int
    external fun nativeTextReplaceRange(engine: Long, start: Int, end: Int, text: String): Int
    external fun nativeTextSetSelection(engine: Long, start: Int, end: Int): Int
    external fun nativeImeSetComposingRegion(engine: Long, start: Int, end: Int): Int
    external fun nativeImeSetComposingText(engine: Long, text: String, selStart: Int, selEnd: Int): Int
    external fun nativeImeCommit(engine: Long, text: String, newCursorPosition: Int, requestId: Long): Int
    external fun nativeImeCancel(engine: Long, requestId: Long): Int
    external fun nativeTextBatchBegin(engine: Long): Int
    external fun nativeTextBatchEnd(engine: Long): Int
    external fun nativeTextGetState(engine: Long, out: JianTextState): Int
    external fun nativeTextGetRange(engine: Long, start: Int, end: Int): String?
    external fun nativeTextCaretRect(engine: Long, out: FloatArray): Int
    external fun nativeCapabilityResult(
        engine: Long,
        requestId: Long,
        kind: Int,
        ok: Boolean,
        httpStatus: Int,
        headersJson: String?,
        bytes: ByteArray?,
        boolValue: Boolean,
        error: String?,
    ): Int
    external fun nativeRegisterFont(engine: Long, bytes: ByteArray): Int
    external fun nativeDestroy(engine: Long)

    // Debug builds only (`debug-hooks` feature): arm the Task 3 fault seams.
    external fun nativeDebugFailNextAttach(engine: Long): Int
    external fun nativeDebugLoseContext(engine: Long): Int
}

/** Owned surrounding-text snapshot filled by [JianNative.nativeTextGetState]. */
class JianTextState {
    @JvmField var text = ""
    @JvmField var windowStart = 0
    @JvmField var selectionStart = 0
    @JvmField var selectionEnd = 0
    @JvmField var hasComposing = false
    @JvmField var composingStart = 0
    @JvmField var composingEnd = 0
}

/** Engine-thread upcalls (forward-only). Every method runs ON the engine thread. */
interface JianCallbacks {
    fun onNeedsRedraw(fromFrame: Boolean, hasNextWake: Boolean, nextWakeMs: Long)
    fun onRuntimeError(kind: Int, message: String, source: String?)
    fun onImeControl(op: Int, requestId: Long)
    fun onInputFocusChanged(focused: Boolean, inputKind: Int, returnKeyHint: Int)
    fun onTextStateChanged()
    fun onCapabilityRequest(requestId: Long, kind: Int, payloadJson: String, bodyBytes: ByteArray?)
    fun onCapabilityCancelled(requestId: Long)
}
