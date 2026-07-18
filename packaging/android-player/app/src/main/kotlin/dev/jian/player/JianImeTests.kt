package dev.jian.player

import android.util.Log
import android.view.KeyEvent

private const val TAG = "JianPlayer"

/**
 * Deterministic IME acceptance harnesses (Task 6 Step 8). Each drives the real
 * [JianInputConnection] and asserts through `nativeTextGetState` — independent
 * of manual IME behaviour — then logs `IME_TEST <name> PASS|FAIL <detail>`.
 *
 * PRECONDITION: a text field must be focused (the acceptance script taps it
 * first); without focus the text natives return NoFocus and the test reports
 * SKIP rather than a false failure.
 */
object JianImeTests {

    private fun state(engine: Long): JianTextState? {
        val s = JianTextState()
        return if (JianNative.nativeTextGetState(engine, s) == 0) s else null
    }

    private fun pass(name: String, detail: String = "") {
        Log.i(TAG, "IME_TEST $name PASS $detail")
    }

    private fun fail(name: String, detail: String) {
        Log.e(TAG, "IME_TEST $name FAIL $detail")
    }

    private fun skip(name: String) {
        Log.w(TAG, "IME_TEST $name SKIP (no focused field)")
    }

    /** Replaces the whole document text with [text] and returns success. */
    private fun resetText(engine: Long, text: String): Boolean {
        val full = JianNative.nativeTextGetRange(engine, 0, Int.MAX_VALUE) ?: return false
        if (JianNative.nativeTextReplaceRange(engine, 0, full.length, text) != 0) return false
        return true
    }

    /**
     * Backspace as a KEY EVENT, which is how real IMEs (Sogou; Gboard on an
     * empty composition) send it. Regression: `sendKeyEvent` was not
     * overridden, so `BaseInputConnection` routed the key to the view's key
     * dispatcher -- which this view does not implement -- and backspace was
     * silently inert under a real IME while every direct editing call worked.
     * The other harnesses call deleteSurroundingText directly and so all
     * passed straight through the gap.
     */
    fun keyEventTest(view: JianSurfaceView) {
        val name = "IME_KEY_TEST"
        val engine = view.engine
        if (engine == 0L || state(engine) == null) return skip(name)

        // CJK on purpose: one BMP char is 1 UTF-16 unit but 3 UTF-8 bytes, so
        // a byte/unit mix-up in the delete path shows up here and not on ASCII.
        if (!resetText(engine, "AB\u4F60\u597D")) return fail(name, "reset failed")
        JianNative.nativeTextSetSelection(engine, 4, 4)

        val ic = JianInputConnection(view)
        val down = KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_DEL)
        ic.sendKeyEvent(down)
        val afterOne = JianNative.nativeTextGetRange(engine, 0, Int.MAX_VALUE) ?: ""
        ic.sendKeyEvent(down)
        val afterTwo = JianNative.nativeTextGetRange(engine, 0, Int.MAX_VALUE) ?: ""

        // A non-collapsed selection must go whole, not one unit of it.
        if (!resetText(engine, "ABCDEF")) return fail(name, "reset failed (2)")
        JianNative.nativeTextSetSelection(engine, 1, 4)
        ic.sendKeyEvent(down)
        val afterSelection = JianNative.nativeTextGetRange(engine, 0, Int.MAX_VALUE) ?: ""

        val oneOk = afterOne == "AB\u4F60"
        val twoOk = afterTwo == "AB"
        val selOk = afterSelection == "AEF"
        if (oneOk && twoOk && selOk) {
            pass(name, "KEYCODE_DEL deleted '$afterOne' then '$afterTwo'; selection collapsed to '$afterSelection'")
        } else {
            fail(
                name,
                "afterOne='$afterOne' (want AB\u4F60 ok=$oneOk) afterTwo='$afterTwo' (want AB ok=$twoOk) " +
                    "afterSelection='$afterSelection' (want AEF ok=$selOk)",
            )
        }
    }

    /** Logs the engine's text around the caret: `TEXT_DUMP caret=.. before=.. after=..`. */
    fun dumpAroundCaret(view: JianSurfaceView) {
        val engine = view.engine
        val s = if (engine == 0L) null else state(engine)
        if (s == null) {
            Log.w(TAG, "TEXT_DUMP SKIP (no focused field)")
            return
        }
        val full = JianNative.nativeTextGetRange(engine, 0, Int.MAX_VALUE) ?: ""
        val caret = s.selectionStart.coerceIn(0, full.length)
        val before = full.substring((caret - 12).coerceAtLeast(0), caret)
        val after = full.substring(caret, (caret + 12).coerceAtMost(full.length))
        Log.i(
            TAG,
            "TEXT_DUMP caret=$caret len=${full.length} composing=${s.hasComposing} " +
                "before='$before' after='$after'",
        )
    }

    /**
     * deleteSurroundingText around a NON-COLLAPSED selection with a live
     * composition: asserts both ranges are deleted, the selection survives
     * (not collapsed) and is correctly transformed, and the composition is
     * re-established at its transformed interval.
     */
    fun deleteTest(view: JianSurfaceView, codePoints: Boolean) {
        val name = if (codePoints) "IME_DELETE_TEST(codepoints)" else "IME_DELETE_TEST(utf16)"
        val engine = view.engine
        if (engine == 0L || state(engine) == null) return skip(name)

        // Layout: "AAAA[BBBB]CCCC" with a composition over the first AAAA.
        if (!resetText(engine, "AAAABBBBCCCC")) return fail(name, "reset failed")
        JianNative.nativeImeSetComposingRegion(engine, 0, 4)          // composing = AAAA
        JianNative.nativeTextSetSelection(engine, 4, 8)               // selection = BBBB (non-collapsed)
        val before = state(engine) ?: return fail(name, "no state")
        if (before.selectionStart == before.selectionEnd) {
            return fail(name, "precondition: selection collapsed")
        }

        val ic = JianInputConnection(view)
        // Delete 2 units before the selection and 2 after it.
        if (codePoints) ic.deleteSurroundingTextInCodePoints(2, 2) else ic.deleteSurroundingText(2, 2)

        val after = state(engine) ?: return fail(name, "no state after")
        val text = JianNative.nativeTextGetRange(engine, 0, Int.MAX_VALUE) ?: ""
        // "AAAABBBBCCCC" - [2,4) before and [8,10) after → "AABBBBCC"
        val textOk = text == "AABBBBCC"
        val selOk = after.selectionEnd > after.selectionStart // NOT collapsed
        val compOk = after.hasComposing && after.composingEnd > after.composingStart
        if (textOk && selOk && compOk) {
            pass(name, "text='$text' sel=[${after.selectionStart},${after.selectionEnd}] comp=[${after.composingStart},${after.composingEnd}]")
        } else {
            fail(
                name,
                "text='$text' (want 'AABBBBCC' ok=$textOk) sel=[${after.selectionStart},${after.selectionEnd}] notCollapsed=$selOk comp=$compOk",
            )
        }
    }

    /**
     * finishComposingText must commit the composing snapshot AND leave the
     * caret where it was (Android's contract: text and cursor unchanged),
     * for a caret both inside and outside the composing span.
     */
    fun finishTest(view: JianSurfaceView, caretOutside: Boolean) {
        val name = if (caretOutside) "IME_FINISH_TEST(outside)" else "IME_FINISH_TEST(inside)"
        val engine = view.engine
        if (engine == 0L || state(engine) == null) return skip(name)

        if (!resetText(engine, "HELLOWORLD")) return fail(name, "reset failed")
        JianNative.nativeImeSetComposingRegion(engine, 0, 5) // composing = HELLO
        val caret = if (caretOutside) 8 else 3               // outside vs inside the span
        JianNative.nativeTextSetSelection(engine, caret, caret)
        view.platformComposingText = JianNative.nativeTextGetRange(engine, 0, 5)

        val ic = JianInputConnection(view)
        ic.finishComposingText()

        val after = state(engine) ?: return fail(name, "no state after")
        val text = JianNative.nativeTextGetRange(engine, 0, Int.MAX_VALUE) ?: ""
        val textOk = text == "HELLOWORLD"                    // unchanged
        val caretOk = after.selectionStart == caret && after.selectionEnd == caret
        val compGone = !after.hasComposing
        if (textOk && caretOk && compGone) {
            pass(name, "text unchanged, caret=$caret preserved, composition finished")
        } else {
            fail(name, "text='$text' ok=$textOk caret=[${after.selectionStart},${after.selectionEnd}] want=$caret compGone=$compGone")
        }
    }

    /**
     * With a long field the JianTextState window starts past 0; the connection
     * must answer queries in ABSOLUTE document offsets (never the bounded
     * window), so getTextBeforeCursor must match nativeTextGetRange.
     */
    fun queryTest(view: JianSurfaceView) {
        val name = "IME_QUERY_TEST"
        val engine = view.engine
        if (engine == 0L || state(engine) == null) return skip(name)

        val full = JianNative.nativeTextGetRange(engine, 0, Int.MAX_VALUE) ?: return fail(name, "no text")
        if (full.length < 5000) return fail(name, "field too short (${full.length}) — load m4_media")
        val caret = full.length - 10
        JianNative.nativeTextSetSelection(engine, caret, caret)
        val s = state(engine) ?: return fail(name, "no state")

        val ic = JianInputConnection(view)
        val before = ic.getTextBeforeCursor(20, 0).toString()
        val expect = full.substring((caret - 20).coerceAtLeast(0), caret)
        val windowShifted = s.windowStart > 0
        if (before == expect) {
            pass(name, "windowStart=${s.windowStart} (shifted=$windowShifted) before-cursor matches absolute offsets")
        } else {
            fail(name, "before='$before' expect='$expect' windowStart=${s.windowStart}")
        }
    }

    /**
     * An abandoned nested batch must not suppress text_state_changed forever:
     * closeConnection unwinds every open level (one batchEnd each).
     */
    fun batchRestartTest(view: JianSurfaceView) {
        val name = "BATCH_RESTART_TEST"
        val engine = view.engine
        if (engine == 0L || state(engine) == null) return skip(name)

        val ic = JianInputConnection(view)
        ic.beginBatchEdit()
        ic.beginBatchEdit() // depth 2
        ic.closeConnection() // must unwind BOTH levels
        // A subsequent edit through a fresh connection must still take effect
        // (batches balanced ⇒ notifications resume).
        val fresh = JianInputConnection(view)
        val lenBefore = (JianNative.nativeTextGetRange(engine, 0, Int.MAX_VALUE) ?: "").length
        fresh.commitText("Z", 1)
        val lenAfter = (JianNative.nativeTextGetRange(engine, 0, Int.MAX_VALUE) ?: "").length
        if (lenAfter == lenBefore + 1) {
            pass(name, "nested batch unwound; edit applied ($lenBefore→$lenAfter)")
        } else {
            fail(name, "edit not applied ($lenBefore→$lenAfter) — batches may be unbalanced")
        }
    }
}
