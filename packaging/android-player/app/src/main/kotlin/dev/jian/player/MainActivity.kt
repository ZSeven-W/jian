package dev.jian.player

import android.app.Activity
import android.os.Bundle
import android.util.Log
import android.view.View
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import java.io.File

private const val TAG = "JianPlayer"

/**
 * Hosts the [JianSurfaceView]. Edge-to-edge so `$viewport` spans the full
 * window and the IME arrives only as an inset; `configChanges` on the
 * activity (manifest) keeps the engine alive across rotation. `onDestroy`
 * always tears the engine down (§6.7).
 */
class MainActivity : Activity() {

    private lateinit var surfaceView: JianSurfaceView
    private var debugReceiver: JianDebugReceiver? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        // Edge-to-edge: the surface always spans the full window (§6.2).
        WindowCompat.setDecorFitsSystemWindows(window, false)

        val density = resources.displayMetrics.density
        val docName = intent.getStringExtra("doc") ?: "m1_acceptance"
        val noAssetBase = intent.getBooleanExtra("noAssetBase", false)

        val assetBase = if (noAssetBase) null else extractAssets()
        val doc = readDoc(docName)
        val font = readAsset("fonts/player-font.ttf")

        surfaceView = JianSurfaceView(this).apply {
            configure(doc, assetBase, font)
        }
        setContentView(surfaceView)

        // Real inset path (no island): system bars + cutout → safe area,
        // IME inset height → keyboard, both in logical px.
        surfaceView.setOnApplyWindowInsetsListener { _, insets ->
            val compat = WindowInsetsCompat.toWindowInsetsCompat(insets, surfaceView)
            val bars = compat.getInsets(
                WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout(),
            )
            surfaceView.updateSafeArea(
                bars.top / density,
                bars.right / density,
                bars.bottom / density,
                bars.left / density,
            )
            val ime = compat.getInsets(WindowInsetsCompat.Type.ime())
            surfaceView.updateKeyboard(ime.bottom / density)
            insets
        }
        surfaceView.systemUiVisibility = View.SYSTEM_UI_FLAG_LAYOUT_STABLE or
            View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN

        if (BuildConfig.DEBUG) {
            debugReceiver = JianDebugReceiver(surfaceView).also { it.register(this) }
        }
    }

    override fun onDestroy() {
        super.onDestroy()
        debugReceiver?.unregister(this)
        // §6.7 teardown, unconditionally (rotation never reaches here thanks
        // to configChanges).
        surfaceView.destroy()
    }

    /**
     * Copies packaged `media/` assets to a filesystem directory the engine can
     * read (APK assets are not plain files) and returns that root as the
     * asset base.
     */
    private fun extractAssets(): String {
        val root = File(filesDir, "assets")
        copyAssetDir("media", File(root, "media"))
        return root.absolutePath
    }

    private fun copyAssetDir(assetPath: String, dest: File) {
        val entries = try {
            assets.list(assetPath) ?: emptyArray()
        } catch (e: Exception) {
            emptyArray()
        }
        if (entries.isEmpty()) return // not a directory (or empty)
        dest.mkdirs()
        for (name in entries) {
            val child = "$assetPath/$name"
            val sub = assets.list(child)
            if (sub != null && sub.isNotEmpty()) {
                copyAssetDir(child, File(dest, name))
            } else {
                runCatching {
                    assets.open(child).use { input ->
                        File(dest, name).outputStream().use { input.copyTo(it) }
                    }
                }.onFailure { Log.w(TAG, "asset copy failed: $child", it) }
            }
        }
    }

    private fun readDoc(name: String): ByteArray = readAsset("$name.op") ?: ByteArray(0)

    /** Exposed for JianDebugReceiver's LOAD_DOC recreate. */
    fun readDocPublic(name: String): ByteArray = readDoc(name)

    private fun readAsset(path: String): ByteArray? = try {
        assets.open(path).use { it.readBytes() }
    } catch (e: Exception) {
        Log.w(TAG, "could not read asset $path", e)
        null
    }
}
