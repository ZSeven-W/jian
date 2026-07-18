package dev.jian.player

import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.net.Uri
import android.util.Log
import org.json.JSONObject
import java.io.InputStream
import java.net.HttpURLConnection
import java.net.URL
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors
import java.util.concurrent.FutureTask

private const val TAG = "JianPlayer"
private const val BODY_CAP = 64 * 1024 * 1024 // the ABI's 64 MiB cap
private const val DEFAULT_TIMEOUT_MS = 30_000 // cross-host rule: 0 = no authored timeout → 30s guard

// Capability kinds (mirror JianCapabilityKind).
private const val KIND_HTTP_FETCH = 0
private const val KIND_CONFIRM = 1
private const val KIND_CLIPBOARD_READ = 2
private const val KIND_CLIPBOARD_WRITE = 3
private const val KIND_IMAGE_FETCH = 4
private const val KIND_OPEN_URL = 5

/**
 * Platform capability implementations (Task 6 Step 7). Requests arrive on the
 * engine thread via [JianCallbacksImpl.onCapabilityRequest]; HTTP runs on a
 * 2-thread executor and results are delivered back through
 * `nativeCapabilityResult` (itself an engine-thread dispatch). Cancellation is
 * real at every stage: each request registers under one synchronized
 * transition, and a cancel tombstones the id so a late-starting job aborts.
 */
class JianCapabilities(private val view: JianSurfaceView) {

    private val activity: Activity get() = view.context as Activity
    private val engine: Long get() = view.engine
    private val executor = Executors.newFixedThreadPool(2)

    private class Pending(
        val task: FutureTask<*>?,
        @Volatile var connection: HttpURLConnection? = null,
        @Volatile var cancelled: Boolean = false,
    )

    private val pending = ConcurrentHashMap<Long, Pending>()

    fun onRequest(requestId: Long, kind: Int, payloadJson: String, bodyBytes: ByteArray?) {
        val payload = runCatching { JSONObject(payloadJson) }.getOrDefault(JSONObject())
        when (kind) {
            KIND_HTTP_FETCH, KIND_IMAGE_FETCH -> startFetch(requestId, kind, payload, bodyBytes)
            KIND_CONFIRM -> showConfirm(requestId, payload)
            KIND_OPEN_URL -> openUrl(requestId, payload)
            KIND_CLIPBOARD_WRITE, KIND_CLIPBOARD_READ ->
                deliver(requestId, kind, ok = false, error = "clipboard not supported")
            else -> deliver(requestId, kind, ok = false, error = "unknown capability")
        }
    }

    fun onCancelled(requestId: Long) {
        val p = pending.remove(requestId) ?: run {
            // Tombstone: a job that has not started yet will see this and abort.
            pending[requestId] = Pending(task = null, cancelled = true)
            return
        }
        p.cancelled = true
        p.task?.cancel(true)
        p.connection?.let { runCatching { it.disconnect() } }
        Log.i(TAG, "request $requestId aborted")
    }

    fun teardown() {
        pending.keys.toList().forEach { onCancelled(it) }
        executor.shutdownNow()
    }

    // ---- HTTP fetch / image fetch ---------------------------------------

    private fun startFetch(requestId: Long, kind: Int, payload: JSONObject, body: ByteArray?) {
        val method = payload.optString("method", "GET").ifEmpty { "GET" }
        val url = payload.optString("url")
        val hasTimeout = !payload.isNull("timeoutMs")
        val rawTimeout = if (hasTimeout) payload.optLong("timeoutMs", 0L) else 0L
        // 0 = no authored timeout → 30s guard (Android treats 0 as infinite).
        val timeout = when {
            rawTimeout <= 0L -> DEFAULT_TIMEOUT_MS.also {
                Log.i(TAG, "request $requestId: no authored timeout, using ${it}ms guard")
            }
            rawTimeout > Int.MAX_VALUE -> Int.MAX_VALUE
            else -> rawTimeout.toInt()
        }
        val headers = payload.optJSONArray("headers")

        // Register BEFORE execution under one synchronized transition.
        val existing = pending[requestId]
        if (existing != null && existing.cancelled) {
            pending.remove(requestId)
            return // already tombstoned
        }
        val holder = Pending(task = null)
        val task = FutureTask {
            runFetch(requestId, kind, method, url, headers, body, timeout, holder)
        }
        val installed = Pending(task = task)
        pending[requestId] = installed
        Log.i(TAG, "request $requestId started")
        runCatching { executor.execute(task) }.onFailure {
            pending.remove(requestId)
            deliver(requestId, kind, ok = false, error = "executor rejected")
        }
    }

    private fun runFetch(
        requestId: Long,
        kind: Int,
        method: String,
        urlStr: String,
        headers: org.json.JSONArray?,
        body: ByteArray?,
        timeoutMs: Int,
        holder: Pending,
    ) {
        val p = pending[requestId]
        if (p != null && p.cancelled) {
            pending.remove(requestId)
            return
        }
        var conn: HttpURLConnection? = null
        try {
            conn = (URL(urlStr).openConnection() as HttpURLConnection).apply {
                requestMethod = method
                connectTimeout = timeoutMs
                readTimeout = timeoutMs
                if (headers != null) {
                    for (i in 0 until headers.length()) {
                        val h = headers.optJSONArray(i) ?: continue
                        if (h.length() >= 2) setRequestProperty(h.optString(0), h.optString(1))
                    }
                }
                if (body != null && body.isNotEmpty()) {
                    doOutput = true
                    outputStream.use { it.write(body) }
                }
            }
            p?.connection = conn
            if (p?.cancelled == true) {
                conn.disconnect(); pending.remove(requestId); return
            }
            val status = conn.responseCode
            val stream: InputStream? = if (status in 200..299) conn.inputStream else conn.errorStream
            val bytes = stream?.let { readBounded(it) } ?: ByteArray(0)
            val headersJson = marshalHeaders(conn)
            if (pending[requestId]?.cancelled == true) { pending.remove(requestId); return }
            pending.remove(requestId)
            if (kind == KIND_IMAGE_FETCH) {
                deliver(requestId, kind, ok = status in 200..299, bytes = bytes)
            } else {
                deliverHttp(requestId, status in 200..299, status, headersJson, bytes)
            }
        } catch (e: Exception) {
            pending.remove(requestId)
            if (holder.cancelled) return
            Log.w(TAG, "request $requestId fetch failed: ${e.message}")
            deliver(requestId, kind, ok = false, error = e.message ?: "fetch failed")
        } finally {
            runCatching { conn?.disconnect() }
        }
    }

    /** Reads at most [BODY_CAP] bytes so a hostile response can't OOM. */
    private fun readBounded(input: InputStream): ByteArray {
        val out = java.io.ByteArrayOutputStream()
        val buf = ByteArray(64 * 1024)
        var total = 0
        input.use {
            while (true) {
                val n = it.read(buf)
                if (n < 0) break
                total += n
                if (total > BODY_CAP) break
                out.write(buf, 0, n)
            }
        }
        return out.toByteArray()
    }

    private fun marshalHeaders(conn: HttpURLConnection): String {
        val obj = JSONObject()
        for ((k, v) in conn.headerFields) {
            if (k != null) obj.put(k, v.joinToString(","))
        }
        return obj.toString()
    }

    // ---- Confirm / open-url ---------------------------------------------

    private fun showConfirm(requestId: Long, payload: JSONObject) {
        val title = payload.optString("title")
        val message = payload.optString("message")
        activity.runOnUiThread {
            if (pending[requestId]?.cancelled == true) { pending.remove(requestId); return@runOnUiThread }
            val dialog = AlertDialog.Builder(activity)
                .setTitle(title)
                .setMessage(message)
                .setPositiveButton(android.R.string.ok) { _, _ -> deliverConfirm(requestId, true) }
                .setNegativeButton(android.R.string.cancel) { _, _ -> deliverConfirm(requestId, false) }
                .setOnCancelListener { deliverConfirm(requestId, false) }
                .create()
            pending[requestId] = Pending(task = null)
            dialog.show()
        }
    }

    private fun openUrl(requestId: Long, payload: JSONObject) {
        val url = payload.optString("url")
        val ok = runCatching {
            activity.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)).apply {
                addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            })
        }.isSuccess
        deliver(requestId, KIND_OPEN_URL, ok = ok, error = if (ok) null else "no handler for url")
    }

    // ---- Result delivery (nativeCapabilityResult) -----------------------

    private fun deliverHttp(requestId: Long, ok: Boolean, status: Int, headersJson: String, bytes: ByteArray) {
        if (engine == 0L) return
        JianNative.nativeCapabilityResult(
            engine, requestId, KIND_HTTP_FETCH, ok, status, headersJson, bytes, false, null,
        )
    }

    private fun deliverConfirm(requestId: Long, value: Boolean) {
        pending.remove(requestId)
        if (engine == 0L) return
        JianNative.nativeCapabilityResult(
            engine, requestId, KIND_CONFIRM, true, 0, null, null, value, null,
        )
    }

    private fun deliver(
        requestId: Long,
        kind: Int,
        ok: Boolean,
        bytes: ByteArray? = null,
        error: String? = null,
    ) {
        if (engine == 0L) return
        JianNative.nativeCapabilityResult(
            engine, requestId, kind, ok, 0, null, bytes, false, error,
        )
    }
}
