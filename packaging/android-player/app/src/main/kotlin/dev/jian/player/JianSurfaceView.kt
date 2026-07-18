package dev.jian.player

import android.content.Context
import android.graphics.Matrix
import android.text.InputType
import android.util.Log
import android.view.Choreographer
import android.view.MotionEvent
import android.view.Surface
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.inputmethod.CursorAnchorInfo
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.ExtractedText
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager

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

    private val imm: InputMethodManager
        get() = context.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager

    // ---- VIEW-owned IME state (survives restartInput; §6.4) ---------------
    /** The owned composing snapshot, cleared/updated only after an Ok native. */
    var platformComposingText: String? = null
    /** GET_EXTRACTED_TEXT_MONITOR request token, or null. */
    var extractedTextToken: Int? = null
    /** CURSOR_UPDATE_MONITOR flag. */
    var cursorMonitor = false

    private var editable = false
    private var inputKind = 0
    private var returnKeyHint = 0

    init {
        holder.addCallback(this)
        isFocusable = true
        isFocusableInTouchMode = true
    }

    override fun onCheckIsTextEditor(): Boolean = editable

    override fun onCreateInputConnection(outAttrs: EditorInfo): InputConnection? {
        if (!editable || engine == 0L) return null
        outAttrs.inputType = when (inputKind) {
            1 -> InputType.TYPE_CLASS_NUMBER
            2 -> InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
            else -> InputType.TYPE_CLASS_TEXT
        }
        // JianReturnKeyHint: 1 Done, 2 Go, 3 Next, 4 Search, 5 Send.
        outAttrs.imeOptions = when (returnKeyHint) {
            1 -> EditorInfo.IME_ACTION_DONE
            2 -> EditorInfo.IME_ACTION_GO
            3 -> EditorInfo.IME_ACTION_NEXT
            4 -> EditorInfo.IME_ACTION_SEARCH
            5 -> EditorInfo.IME_ACTION_SEND
            else -> EditorInfo.IME_ACTION_UNSPECIFIED
        }
        val s = JianTextState()
        JianNative.nativeTextGetState(engine, s)
        outAttrs.initialSelStart = s.selectionStart
        outAttrs.initialSelEnd = s.selectionEnd
        return JianInputConnection(this)
    }

    /** Focus/editable transition from the engine (called on the main thread). */
    fun applyFocus(focused: Boolean, kind: Int, returnKey: Int) {
        editable = focused
        inputKind = kind
        returnKeyHint = returnKey
        if (focused) {
            requestFocus()
            imm.restartInput(this)
            imm.showSoftInput(this, 0)
        } else {
            platformComposingText = null // never commit a stale snapshot elsewhere
            imm.restartInput(this)
            imm.hideSoftInputFromWindow(windowToken, 0)
        }
    }

    fun restartInput() = imm.restartInput(this)

    fun updateSelectionFromEngine() {
        if (engine == 0L) return
        val s = JianTextState()
        if (JianNative.nativeTextGetState(engine, s) != 0) return
        imm.updateSelection(
            this,
            s.selectionStart,
            s.selectionEnd,
            if (s.hasComposing) s.composingStart else -1,
            if (s.hasComposing) s.composingEnd else -1,
        )
    }

    fun hideKeyboard() = imm.hideSoftInputFromWindow(windowToken, 0)

    /** Sends a CursorAnchorInfo now (logical → view px, on-screen matrix). */
    fun pushCursorAnchor() {
        if (engine == 0L) return
        val rect = FloatArray(4)
        if (JianNative.nativeTextCaretRect(engine, rect) != 0) return
        val loc = IntArray(2)
        getLocationOnScreen(loc)
        val matrix = Matrix().apply { setTranslate(loc[0].toFloat(), loc[1].toFloat()) }
        val x = rect[0] * density
        val top = rect[1] * density
        val bottom = (rect[1] + rect[3]) * density
        val info = CursorAnchorInfo.Builder()
            .setMatrix(matrix)
            .setInsertionMarkerLocation(x, top, bottom, bottom, CursorAnchorInfo.FLAG_HAS_VISIBLE_REGION)
            .build()
        imm.updateCursorAnchorInfo(this, info)
    }

    fun pushCursorAnchorIfMonitoring() {
        if (cursorMonitor) pushCursorAnchor()
    }

    fun pushExtractedTextIfMonitoring() {
        val token = extractedTextToken ?: return
        if (engine == 0L) return
        val full = JianNative.nativeTextGetRange(engine, 0, Int.MAX_VALUE) ?: ""
        val s = JianTextState()
        JianNative.nativeTextGetState(engine, s)
        val et = ExtractedText().apply {
            text = full
            startOffset = 0
            selectionStart = s.selectionStart
            selectionEnd = s.selectionEnd
            partialStartOffset = -1
            partialEndOffset = -1
        }
        imm.updateExtractedText(this, token, et)
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
