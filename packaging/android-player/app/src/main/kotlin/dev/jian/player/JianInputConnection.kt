package dev.jian.player

import android.icu.text.BreakIterator
import android.view.KeyEvent
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.ExtractedText
import android.view.inputmethod.ExtractedTextRequest
import android.view.inputmethod.InputConnection.CURSOR_UPDATE_IMMEDIATE
import android.view.inputmethod.InputConnection.CURSOR_UPDATE_MONITOR
import android.view.inputmethod.InputConnection.GET_EXTRACTED_TEXT_MONITOR

private const val WIDE = Int.MAX_VALUE // the ABI clamps; the returned length IS the doc length

/**
 * Bridges the platform IME to the engine's text surface (spec §6.4, Task 6
 * Step 5). The composing snapshot and cursor-monitor flag are VIEW-owned
 * (survive `restartInput`); this connection reads/writes them. The engine —
 * not `BaseInputConnection`'s fake editable — is the source of truth: every
 * query goes through `nativeTextGetState`/`nativeTextGetRange`, and every
 * successful mutation schedules an immediate frame (the mutation-wake rule).
 */
class JianInputConnection(
    private val view: JianSurfaceView,
) : BaseInputConnection(view, true) {

    private val engine: Long get() = view.engine
    private var batchDepth = 0
    private var valid = true

    private fun state(): JianTextState {
        val s = JianTextState()
        JianNative.nativeTextGetState(engine, s)
        return s
    }

    /** Full document text (the JianTextState window is bounded; this is not). */
    private fun fullText(): String = JianNative.nativeTextGetRange(engine, 0, WIDE) ?: ""

    private fun ok(status: Int): Boolean = status == 0

    private fun wake() = view.requestFrame()

    // ---- Commit / composition -------------------------------------------

    override fun commitText(text: CharSequence, newCursorPosition: Int): Boolean {
        if (!valid || engine == 0L) return false
        // N passes through unconverted (jian.h).
        if (ok(JianNative.nativeImeCommit(engine, text.toString(), newCursorPosition, 0))) {
            view.platformComposingText = null
            wake()
        }
        return true
    }

    override fun setComposingText(text: CharSequence, newCursorPosition: Int): Boolean {
        if (!valid || engine == 0L) return false
        val s = state()
        val full = fullText()
        val oldLen = full.length
        val selStart = s.selectionStart
        val replacedLen = if (s.hasComposing) s.composingEnd - s.composingStart else 0
        val newText = text.toString()
        val finalLen = (oldLen.toLong() - replacedLen + newText.length).coerceAtLeast(0)

        // Absolute target caret in WIDE arithmetic (a far-negative N must not
        // wrap the unsigned ABI types).
        val n = newCursorPosition.toLong()
        val absolute = if (n > 0) {
            selStart.toLong() + replacedLen + n - 1
        } else {
            selStart.toLong() + n
        }.coerceIn(0L, finalLen)

        // The composing call's own relative cursor (selStart==selEnd).
        val composingBase = selStart.toLong()
        val rel = (absolute - composingBase).coerceIn(0L, newText.length.toLong()).toInt()

        JianNative.nativeTextBatchBegin(engine)
        val status = JianNative.nativeImeSetComposingText(engine, newText, rel, rel)
        // When the clamped absolute caret is outside the new composing range,
        // set it explicitly (the jian.h escape hatch).
        val composingLo = composingBase
        val composingHi = composingBase + newText.length
        if (absolute < composingLo || absolute > composingHi) {
            JianNative.nativeTextSetSelection(engine, absolute.toInt(), absolute.toInt())
        }
        JianNative.nativeTextBatchEnd(engine)

        if (ok(status)) {
            view.platformComposingText = newText
            wake()
        }
        return true
    }

    override fun setComposingRegion(start: Int, end: Int): Boolean {
        if (!valid || engine == 0L) return false
        val len = fullText().length
        // Negative endpoints clamp to 0; swap reversed; clamp to length.
        var lo = start.coerceAtLeast(0).coerceAtMost(len)
        var hi = end.coerceAtLeast(0).coerceAtMost(len)
        if (lo > hi) {
            val t = lo; lo = hi; hi = t
        }
        if (lo == hi) {
            // Normalizes to empty → the finishComposing path (Android).
            return finishComposingText()
        }
        val s = state()
        JianNative.nativeTextBatchBegin(engine)
        val status = JianNative.nativeImeSetComposingRegion(engine, lo, hi)
        JianNative.nativeTextSetSelection(engine, s.selectionStart, s.selectionEnd)
        JianNative.nativeTextBatchEnd(engine)
        if (ok(status)) {
            // Assign the snapshot only after Ok (never plant stale text).
            view.platformComposingText = JianNative.nativeTextGetRange(engine, lo, hi)
            wake()
        }
        return true
    }

    override fun finishComposingText(): Boolean {
        if (!valid || engine == 0L) return true
        val s = state()
        if (!s.hasComposing) return true
        val snapshot = view.platformComposingText
            ?: JianNative.nativeTextGetRange(engine, s.composingStart, s.composingEnd)
            ?: ""
        val savedStart = s.selectionStart
        val savedEnd = s.selectionEnd
        JianNative.nativeTextBatchBegin(engine)
        // A bare N=1 commit moves the caret to the composing end; restore the
        // saved selection (the snapshot replaces the span 1:1, so offsets past
        // it are unchanged).
        JianNative.nativeImeCommit(engine, snapshot, 1, 0)
        JianNative.nativeTextSetSelection(engine, savedStart, savedEnd)
        JianNative.nativeTextBatchEnd(engine)
        view.platformComposingText = null
        wake()
        return true
    }

    // ---- Deletion -------------------------------------------------------

    override fun deleteSurroundingText(beforeLength: Int, afterLength: Int): Boolean {
        if (!valid || engine == 0L) return false
        deleteSurrounding(beforeLength.toLong().coerceAtLeast(0), afterLength.toLong().coerceAtLeast(0))
        return true
    }

    /**
     * Editing keys arrive as KEY EVENTS from real IMEs — Sogou sends
     * KEYCODE_DEL for backspace rather than calling `deleteSurroundingText`,
     * and Gboard does the same on an empty composition. `BaseInputConnection`
     * would route them to the view's key dispatcher, which this view does not
     * implement, so without this override Backspace and Enter are silently
     * inert under a real IME even though every direct editing call works.
     */
    override fun sendKeyEvent(event: KeyEvent): Boolean {
        if (!valid || engine == 0L) return false
        return applyKey(event)
    }

    /**
     * Applies one key event, reporting whether it actually produced an edit.
     *
     * The honest return value matters: [JianSurfaceView] routes VIEW-level key
     * events here too (a physical keyboard never goes through an
     * `InputConnection`), and it may only consume what this handled. Claiming
     * every key would swallow Back and the volume keys.
     */
    internal fun applyKey(event: KeyEvent): Boolean {
        if (!valid || engine == 0L) return false
        // Text the platform cannot express as key codes arrives whole here.
        if (event.action == KeyEvent.ACTION_MULTIPLE && event.keyCode == KeyEvent.KEYCODE_UNKNOWN) {
            val characters = event.characters
            if (characters.isNullOrEmpty()) return false
            commitText(characters, 1)
            return true
        }
        val handled = when (event.keyCode) {
            KeyEvent.KEYCODE_DEL, KeyEvent.KEYCODE_FORWARD_DEL, KeyEvent.KEYCODE_ENTER -> true
            else -> event.unicodeChar != 0
        }
        if (!handled) return false
        // Consume the UP half of a key we own so nothing re-dispatches it.
        if (event.action != KeyEvent.ACTION_DOWN) return true
        when (event.keyCode) {
            KeyEvent.KEYCODE_DEL -> deleteOneUnit(before = true)
            KeyEvent.KEYCODE_FORWARD_DEL -> deleteOneUnit(before = false)
            KeyEvent.KEYCODE_ENTER -> commitText("\n", 1)
            else -> commitText(String(Character.toChars(event.unicodeChar)), 1)
        }
        return true
    }

    /**
     * A non-collapsed selection is dropped whole (the platform's backspace
     * contract); otherwise one GRAPHEME CLUSTER goes.
     *
     * Cluster, not code point: a flag or a ZWJ family emoji is several code
     * points that render as one character, and deleting them one at a time
     * leaves visible debris. This matches the iOS host, which uses
     * `rangeOfComposedCharacterSequence` — keeping backspace identical across
     * the two platforms. The length is handed to `deleteSurroundingText` in
     * UTF-16 units so the composing-region and selection transforms there
     * still apply.
     */
    private fun deleteOneUnit(before: Boolean) {
        val s = state()
        if (s.selectionEnd > s.selectionStart) {
            if (ok(JianNative.nativeTextReplaceRange(engine, s.selectionStart, s.selectionEnd, ""))) {
                wake()
            }
            return
        }
        val text = fullText()
        val caret = s.selectionStart.coerceIn(0, text.length)
        val breaks = BreakIterator.getCharacterInstance()
        breaks.setText(text)
        val boundary = if (before) {
            if (caret <= 0) return
            breaks.preceding(caret)
        } else {
            if (caret >= text.length) return
            breaks.following(caret)
        }
        if (boundary == BreakIterator.DONE) return
        val length = if (before) caret - boundary else boundary - caret
        if (length <= 0) return
        if (before) deleteSurroundingText(length, 0) else deleteSurroundingText(0, length)
    }

    override fun deleteSurroundingTextInCodePoints(beforeLength: Int, afterLength: Int): Boolean {
        if (!valid || engine == 0L) return false
        val s = state()
        val full = fullText()
        // Convert code points → UTF-16 units against a wide range around the
        // selection.
        val before = codePointsToUnitsBefore(full, s.selectionStart, beforeLength.coerceAtLeast(0))
        val after = codePointsToUnitsAfter(full, s.selectionEnd, afterLength.coerceAtLeast(0))
        deleteSurrounding(before.toLong(), after.toLong())
        return true
    }

    /**
     * Deletes `[selEnd, selEnd+after)` then `[selStart-before, selStart)`,
     * transforms the pre-edit composing interval AND selection through BOTH
     * deletions, re-establishes the composition, and restores the selection.
     */
    private fun deleteSurrounding(before: Long, after: Long) {
        val s = state()
        val len = fullText().length.toLong()
        val selStart = s.selectionStart.toLong()
        val selEnd = s.selectionEnd.toLong()

        val aLo = selEnd.coerceIn(0, len)
        val aHi = (selEnd + after).coerceIn(0, len)
        val bLo = (selStart - before).coerceIn(0, len)
        val bHi = selStart.coerceIn(0, len)

        val hadComposing = s.hasComposing
        var compLo = s.composingStart.toLong()
        var compHi = s.composingEnd.toLong()
        var newSelStart = selStart
        var newSelEnd = selEnd

        JianNative.nativeTextBatchBegin(engine)
        // Delete A first (higher offsets), then B.
        if (aHi > aLo) {
            JianNative.nativeTextReplaceRange(engine, aLo.toInt(), aHi.toInt(), "")
            val d = aHi - aLo
            compLo = shiftDown(compLo, aLo, aHi); compHi = shiftDown(compHi, aLo, aHi)
            newSelStart = shiftDown(newSelStart, aLo, aHi); newSelEnd = shiftDown(newSelEnd, aLo, aHi)
            @Suppress("UNUSED_VALUE") d.let {}
        }
        if (bHi > bLo) {
            JianNative.nativeTextReplaceRange(engine, bLo.toInt(), bHi.toInt(), "")
            compLo = shiftDown(compLo, bLo, bHi); compHi = shiftDown(compHi, bLo, bHi)
            newSelStart = shiftDown(newSelStart, bLo, bHi); newSelEnd = shiftDown(newSelEnd, bLo, bHi)
        }

        if (hadComposing) {
            if (compHi > compLo) {
                JianNative.nativeImeSetComposingRegion(engine, compLo.toInt(), compHi.toInt())
            } else {
                // Transformation left the composition empty: clear the view
                // snapshot; do NOT enter the finish path (would re-insert it).
                view.platformComposingText = null
            }
        }
        // Restore the transformed selection (Android deletes AROUND it).
        JianNative.nativeTextSetSelection(engine, newSelStart.toInt(), newSelEnd.toInt())
        JianNative.nativeTextBatchEnd(engine)
        wake()
    }

    /** A position shifts down by a deleted range's length; inside → its start. */
    private fun shiftDown(pos: Long, lo: Long, hi: Long): Long = when {
        pos <= lo -> pos
        pos >= hi -> pos - (hi - lo)
        else -> lo
    }

    private fun codePointsToUnitsBefore(full: String, sel: Int, cps: Int): Int {
        var i = sel.coerceIn(0, full.length)
        var remaining = cps
        while (remaining > 0 && i > 0) {
            val cp = full.codePointBefore(i)
            i -= Character.charCount(cp)
            remaining--
        }
        return sel.coerceIn(0, full.length) - i
    }

    private fun codePointsToUnitsAfter(full: String, sel: Int, cps: Int): Int {
        var i = sel.coerceIn(0, full.length)
        var remaining = cps
        while (remaining > 0 && i < full.length) {
            val cp = full.codePointAt(i)
            i += Character.charCount(cp)
            remaining--
        }
        return i - sel.coerceIn(0, full.length)
    }

    // ---- Queries (engine is the source of truth) ------------------------

    override fun getTextBeforeCursor(length: Int, flags: Int): CharSequence {
        if (engine == 0L) return ""
        val s = state()
        val lo = (s.selectionStart - length).coerceAtLeast(0)
        return JianNative.nativeTextGetRange(engine, lo, s.selectionStart) ?: ""
    }

    override fun getTextAfterCursor(length: Int, flags: Int): CharSequence {
        if (engine == 0L) return ""
        val s = state()
        val hi = s.selectionEnd + length
        return JianNative.nativeTextGetRange(engine, s.selectionEnd, hi) ?: ""
    }

    override fun getSelectedText(flags: Int): CharSequence? {
        if (engine == 0L) return null
        val s = state()
        if (s.selectionStart == s.selectionEnd) return null
        return JianNative.nativeTextGetRange(engine, s.selectionStart, s.selectionEnd)
    }

    override fun getExtractedText(request: ExtractedTextRequest?, flags: Int): ExtractedText {
        val s = state()
        val full = fullText()
        val et = ExtractedText()
        et.text = full
        et.startOffset = 0
        et.selectionStart = s.selectionStart
        et.selectionEnd = s.selectionEnd
        et.partialStartOffset = -1
        et.partialEndOffset = -1
        if (request != null && (flags and GET_EXTRACTED_TEXT_MONITOR) != 0) {
            view.extractedTextToken = request.token
        }
        return et
    }

    override fun setSelection(start: Int, end: Int): Boolean {
        if (!valid || engine == 0L) return false
        val len = fullText().length
        // Negative or un-normalizable → false, no native call (Android ignores
        // invalid selections; an unchecked Int→u32 cast would wrap).
        if (start < 0 || end < 0 || start > len || end > len) return false
        val lo = minOf(start, end)
        val hi = maxOf(start, end)
        JianNative.nativeTextSetSelection(engine, lo, hi)
        wake()
        return true
    }

    // ---- Batch edits (per-connection depth counter) ---------------------

    override fun beginBatchEdit(): Boolean {
        if (!valid || engine == 0L) return false
        batchDepth++
        JianNative.nativeTextBatchBegin(engine)
        return true
    }

    override fun endBatchEdit(): Boolean {
        if (!valid || engine == 0L || batchDepth == 0) return false
        batchDepth--
        JianNative.nativeTextBatchEnd(engine)
        return batchDepth > 0
    }

    // ---- Cursor updates -------------------------------------------------

    override fun requestCursorUpdates(cursorUpdateMode: Int): Boolean {
        if (!valid || engine == 0L) return false
        val modeBits = CURSOR_UPDATE_IMMEDIATE or CURSOR_UPDATE_MONITOR
        // Filter bits (API 33) may ride inside cursorUpdateMode on this
        // one-arg overload; accept them (v1 always builds the full info —
        // filters are a permission to send less, not an obligation).
        val filterBits = 0x2 or 0x4 or 0x8 or 0x10 // EDITOR_BOUNDS/CHARACTER/INSERTION/VISIBLE_LINE
        if (cursorUpdateMode and (modeBits or filterBits).inv() != 0) return false // unknown bits
        when {
            cursorUpdateMode and CURSOR_UPDATE_MONITOR != 0 -> view.cursorMonitor = true
            cursorUpdateMode and CURSOR_UPDATE_IMMEDIATE == 0 -> view.cursorMonitor = false
        }
        if (cursorUpdateMode and CURSOR_UPDATE_IMMEDIATE != 0) view.pushCursorAnchor()
        return true
    }

    override fun closeConnection() {
        // Android requires calling super first (finishes its composition
        // bookkeeping), then unwind open batches, then invalidate.
        super.closeConnection()
        while (batchDepth > 0) {
            batchDepth--
            if (engine != 0L) JianNative.nativeTextBatchEnd(engine)
        }
        valid = false
    }
}
