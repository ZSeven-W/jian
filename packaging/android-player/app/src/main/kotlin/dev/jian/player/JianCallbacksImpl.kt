package dev.jian.player

import android.os.SystemClock
import android.util.Log

private const val TAG = "JianPlayer"

/**
 * Engine → shell upcalls. All methods run ON the engine thread; anything
 * touching the view / Choreographer is posted to the main thread by the view.
 *
 * Phase A wires the frame pump ([onNeedsRedraw]) fully; IME and capability
 * callbacks are logged (Phase B fills in the InputConnection and the HTTP /
 * confirm / open-url capability implementations).
 */
class JianCallbacksImpl(private val view: JianSurfaceView) : JianCallbacks {

    override fun onNeedsRedraw(fromFrame: Boolean, hasNextWake: Boolean, nextWakeMs: Long) {
        if (!fromFrame) {
            // A mutation outside a frame woke the engine — draw promptly.
            view.requestFrame()
            return
        }
        // End-of-frame directive: schedule the next animation wake, or idle.
        if (hasNextWake) {
            val delay = nextWakeMs - SystemClock.uptimeMillis()
            view.scheduleFrame(delay)
        }
    }

    override fun onRuntimeError(kind: Int, message: String, source: String?) {
        Log.e(TAG, "runtime error kind=$kind: $message${source?.let { " ($it)" } ?: ""}")
    }

    override fun onImeControl(op: Int, requestId: Long) {
        // Mirror iOS answerImeControl: exactly one completion per request id,
        // never routed through the connection's request-id-0 methods.
        view.post {
            val engine = view.engine
            if (engine == 0L) return@post
            when (op) {
                1 -> JianNative.nativeImeCancel(engine, requestId) // Cancel
                else -> { // Commit (0) / Dismiss (2)
                    val text = view.platformComposingText ?: engineComposingText(engine) ?: ""
                    JianNative.nativeImeCommit(engine, text, 1, requestId)
                    view.platformComposingText = null
                    if (op == 2) view.hideKeyboard() // Dismiss
                }
            }
            view.restartInput()
            view.requestFrame()
        }
    }

    override fun onInputFocusChanged(focused: Boolean, inputKind: Int, returnKeyHint: Int) {
        view.post { view.applyFocus(focused, inputKind, returnKeyHint) }
    }

    override fun onTextStateChanged() {
        view.post {
            view.updateSelectionFromEngine()
            view.pushCursorAnchorIfMonitoring()
            view.pushExtractedTextIfMonitoring()
        }
    }

    override fun onCapabilityRequest(requestId: Long, kind: Int, payloadJson: String, bodyBytes: ByteArray?) {
        Log.d(TAG, "capabilityRequest id=$requestId kind=$kind payload=$payloadJson (Phase B — capabilities)")
    }

    override fun onCapabilityCancelled(requestId: Long) {
        Log.d(TAG, "capabilityCancelled id=$requestId (Phase B — capabilities)")
    }

    private fun engineComposingText(engine: Long): String? {
        val s = JianTextState()
        if (JianNative.nativeTextGetState(engine, s) != 0 || !s.hasComposing) return null
        return JianNative.nativeTextGetRange(engine, s.composingStart, s.composingEnd)
    }
}
