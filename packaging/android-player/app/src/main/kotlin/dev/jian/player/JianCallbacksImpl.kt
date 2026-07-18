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
        Log.d(TAG, "imeControl op=$op req=$requestId (Phase B)")
    }

    override fun onInputFocusChanged(focused: Boolean, inputKind: Int, returnKeyHint: Int) {
        Log.d(TAG, "inputFocusChanged focused=$focused kind=$inputKind (Phase B)")
    }

    override fun onTextStateChanged() {
        Log.d(TAG, "textStateChanged (Phase B)")
    }

    override fun onCapabilityRequest(requestId: Long, kind: Int, payloadJson: String, bodyBytes: ByteArray?) {
        Log.d(TAG, "capabilityRequest id=$requestId kind=$kind payload=$payloadJson (Phase B)")
    }

    override fun onCapabilityCancelled(requestId: Long) {
        Log.d(TAG, "capabilityCancelled id=$requestId (Phase B)")
    }
}
