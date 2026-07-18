package dev.jian.player

import android.content.Context
import android.util.Log
import android.view.Choreographer
import android.view.MotionEvent
import android.view.Surface
import android.view.SurfaceHolder
import android.view.SurfaceView

private const val TAG = "JianPlayer"

/** Pointer phases mirroring the C ABI `JianPointerPhase`. */
private const val PHASE_DOWN = 0
private const val PHASE_MOVE = 1
private const val PHASE_UP = 2
private const val PHASE_CANCEL = 3

/**
 * Hosts the engine's rendering surface and drives the frame pump. The engine
 * is created ONCE on the first `surfaceCreated`; the shell owns the
 * Surface→ANativeWindow pairing only indirectly — the native layer acquires
 * and releases the window on the engine thread (§6.7).
 */
class JianSurfaceView(context: Context) : SurfaceView(context), SurfaceHolder.Callback {

    var engine: Long = 0L
        private set

    private var density: Float = resources.displayMetrics.density
    private var attachedOnce = false
    private var docBytes: ByteArray = ByteArray(0)
    private var assetBase: String? = null
    private var fontBytes: ByteArray? = null

    private val choreographer = Choreographer.getInstance()
    private var frameScheduled = false

    /** Latest insets (logical px), replayed after create/attach and resize. */
    private var safeArea = floatArrayOf(0f, 0f, 0f, 0f) // t, r, b, l
    private var keyboardHeight = 0f

    private val callbacks = JianCallbacksImpl(this)

    init {
        holder.addCallback(this)
    }

    fun configure(doc: ByteArray, assetBaseDir: String?, font: ByteArray?) {
        docBytes = doc
        assetBase = assetBaseDir
        fontBytes = font
    }

    // ---- SurfaceHolder.Callback ------------------------------------------

    override fun surfaceCreated(holder: SurfaceHolder) {
        val wLogical = width / density
        val hLogical = height / density
        if (engine == 0L) {
            engine = JianNative.nativeCreate(
                docBytes,
                wLogical,
                hLogical,
                density,
                context.filesDir.absolutePath,
                assetBase,
                callbacks,
            )
            if (engine == 0L) {
                Log.e(TAG, "nativeCreate failed: ${JianNative.nativeLastError(0)}")
                return
            }
            Log.i(TAG, "engine created (${wLogical}x$hLogical dpr=$density)")
            fontBytes?.let { JianNative.nativeRegisterFont(engine, it) }
        }
        attachOrResume(holder.surface)
        replayInsets()
        requestFrame()
    }

    override fun surfaceChanged(holder: SurfaceHolder, format: Int, wPx: Int, hPx: Int) {
        if (engine == 0L) return
        JianNative.nativeResize(engine, wPx / density, hPx / density, density)
        replayInsets()
        requestFrame()
    }

    override fun surfaceDestroyed(holder: SurfaceHolder) {
        if (engine == 0L) return
        // Blocking suspend BEFORE returning — the platform reclaims the
        // Surface after this returns (§6.7).
        JianNative.nativeSuspend(engine)
    }

    /**
     * Until the FIRST successful attach, GPU mode was never selected and
     * `nativeResume` is invalid (§6.2): retry `nativeAttachSurface` on each
     * surfaceCreated until it succeeds, then use `nativeResume` thereafter.
     */
    private fun attachOrResume(surface: Surface) {
        if (!attachedOnce) {
            val status = JianNative.nativeAttachSurface(engine, surface)
            if (status == 0) {
                attachedOnce = true
            } else {
                Log.w(TAG, "attach failed status=$status: ${JianNative.nativeLastError(engine)}")
            }
        } else {
            JianNative.nativeResume(engine, surface)
        }
    }

    // ---- Insets (set by MainActivity's OnApplyWindowInsetsListener) -------

    fun updateSafeArea(t: Float, r: Float, b: Float, l: Float) {
        safeArea = floatArrayOf(t, r, b, l)
        if (engine != 0L) JianNative.nativeSetSafeArea(engine, t, r, b, l)
    }

    fun updateKeyboard(h: Float) {
        keyboardHeight = h
        if (engine != 0L) JianNative.nativeSetKeyboard(engine, h)
    }

    private fun replayInsets() {
        if (engine == 0L) return
        JianNative.nativeSetSafeArea(engine, safeArea[0], safeArea[1], safeArea[2], safeArea[3])
        JianNative.nativeSetKeyboard(engine, keyboardHeight)
    }

    // ---- Frame pump (driven by onNeedsRedraw) ----------------------------

    /** Requests a single Choreographer frame; idempotent while one is queued. */
    fun requestFrame() {
        post {
            if (frameScheduled || engine == 0L) return@post
            frameScheduled = true
            choreographer.postFrameCallback(frameCallback)
        }
    }

    private val frameCallback = Choreographer.FrameCallback { frameTimeNanos ->
        frameScheduled = false
        if (engine == 0L) return@FrameCallback
        JianNative.nativeFrame(engine, frameTimeNanos / 1_000_000)
    }

    /** Schedules a frame `delayMs` from now (the engine's next animation wake). */
    fun scheduleFrame(delayMs: Long) {
        if (delayMs <= 0) {
            requestFrame()
        } else {
            postDelayed({ requestFrame() }, delayMs)
        }
    }

    // ---- Touch (§6.3 pin: the changed pointer only for down/up) ----------

    override fun onTouchEvent(event: MotionEvent): Boolean {
        if (engine == 0L) return false
        val tMs = event.eventTime
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN, MotionEvent.ACTION_POINTER_DOWN -> {
                val i = event.actionIndex
                sendPointer(event, i, PHASE_DOWN, tMs)
            }
            MotionEvent.ACTION_MOVE -> {
                for (i in 0 until event.pointerCount) sendPointer(event, i, PHASE_MOVE, tMs)
            }
            MotionEvent.ACTION_UP, MotionEvent.ACTION_POINTER_UP -> {
                val i = event.actionIndex
                sendPointer(event, i, PHASE_UP, tMs)
            }
            MotionEvent.ACTION_CANCEL -> {
                val i = event.actionIndex
                sendPointer(event, i, PHASE_CANCEL, tMs)
            }
            else -> return false
        }
        return true
    }

    private fun sendPointer(event: MotionEvent, index: Int, phase: Int, tMs: Long) {
        val id = event.getPointerId(index)
        JianNative.nativePointer(
            engine,
            id,
            phase,
            event.getX(index) / density,
            event.getY(index) / density,
            tMs,
        )
    }

    fun destroy() {
        if (engine != 0L) {
            JianNative.nativeDestroy(engine)
            engine = 0L
        }
    }
}
