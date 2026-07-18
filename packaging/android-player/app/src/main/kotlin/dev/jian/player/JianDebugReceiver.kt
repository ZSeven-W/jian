package dev.jian.player

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.Build
import android.util.Log

private const val TAG = "JianPlayer"

/**
 * Debug-only broadcast hooks for scripted acceptance (Task 6 Step 8). Reached
 * via `adb shell am broadcast -a dev.jian.player.<CMD>`. Registered at runtime
 * (RECEIVER_EXPORTED on API 33+) so it exists only in debug builds.
 *
 * Implemented: RESIZE, TEXT_INSERT, THROW_UPCALL, FAIL_NEXT_ATTACH,
 * LOSE_CONTEXT, LOAD_DOC. The elaborate IME_*_TEST / BATCH_RESTART_TEST
 * assertion harnesses (which drive the connection and assert via
 * nativeTextGetState) are pending.
 */
class JianDebugReceiver(private val view: JianSurfaceView) : BroadcastReceiver() {

    fun register(context: Context) {
        val filter = IntentFilter().apply {
            addAction("dev.jian.player.RESIZE")
            addAction("dev.jian.player.TEXT_INSERT")
            addAction("dev.jian.player.THROW_UPCALL")
            addAction("dev.jian.player.FAIL_NEXT_ATTACH")
            addAction("dev.jian.player.LOSE_CONTEXT")
            addAction("dev.jian.player.LOAD_DOC")
            addAction("dev.jian.player.IME_DELETE_TEST")
            addAction("dev.jian.player.IME_FINISH_TEST")
            addAction("dev.jian.player.IME_QUERY_TEST")
            addAction("dev.jian.player.BATCH_RESTART_TEST")
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            context.registerReceiver(this, filter, Context.RECEIVER_EXPORTED)
        } else {
            @Suppress("UnspecifiedRegisterReceiverFlag")
            context.registerReceiver(this, filter)
        }
    }

    fun unregister(context: Context) {
        runCatching { context.unregisterReceiver(this) }
    }

    override fun onReceive(context: Context, intent: Intent) {
        when (intent.action) {
            "dev.jian.player.RESIZE" -> {
                val w = intent.getFloatExtra("w", view.width.toFloat())
                val h = intent.getFloatExtra("h", view.height.toFloat())
                Log.i(TAG, "debug RESIZE ${w}x$h")
                view.debugResize(w, h)
            }
            "dev.jian.player.TEXT_INSERT" -> {
                val s = intent.getStringExtra("s") ?: ""
                Log.i(TAG, "debug TEXT_INSERT '$s'")
                view.debugTextInsert(s)
            }
            "dev.jian.player.THROW_UPCALL" -> {
                Log.i(TAG, "debug THROW_UPCALL armed")
                view.debugThrowNextUpcall()
                view.requestFrame()
            }
            "dev.jian.player.FAIL_NEXT_ATTACH" -> {
                Log.i(TAG, "debug FAIL_NEXT_ATTACH")
                view.debugFailNextAttach()
            }
            "dev.jian.player.LOSE_CONTEXT" -> {
                Log.i(TAG, "debug LOSE_CONTEXT")
                view.debugLoseContext()
            }
            "dev.jian.player.LOAD_DOC" -> {
                val name = intent.getStringExtra("name") ?: "m1_acceptance"
                Log.i(TAG, "debug LOAD_DOC $name")
                view.debugLoadDoc(name)
            }
            // Deterministic IME acceptance harnesses (assert via nativeTextGetState).
            "dev.jian.player.IME_DELETE_TEST" ->
                view.post { JianImeTests.deleteTest(view, intent.getBooleanExtra("codepoints", false)) }
            "dev.jian.player.IME_FINISH_TEST" ->
                view.post { JianImeTests.finishTest(view, intent.getBooleanExtra("outside", false)) }
            "dev.jian.player.IME_QUERY_TEST" ->
                view.post { JianImeTests.queryTest(view) }
            "dev.jian.player.BATCH_RESTART_TEST" ->
                view.post { JianImeTests.batchRestartTest(view) }
        }
    }
}
