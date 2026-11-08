package dev.leptoskit.mobile

import android.content.Context
import android.content.pm.PackageManager
import android.content.res.AssetManager
import android.graphics.PixelFormat
import android.util.Log
import android.view.Choreographer
import android.view.Surface
import android.view.SurfaceHolder
import android.view.SurfaceView
import java.io.IOException

/**
 * A `Scene3d` element: a [SurfaceView] that Rust draws the scene into, with OpenGL ES by default.
 *
 * The guest's ops reach the scene through the `leptoskit.scene3d` plugin, never through this view. It
 * attaches while its surface exists, draws on the [Choreographer] only while the scene changes or animates,
 * and is woken by Rust when the scene changes while it is idle.
 */
internal class NativeScene3dView(
    context: Context,
) : SurfaceView(context),
    SurfaceHolder.Callback,
    Choreographer.FrameCallback {
    private var handle = 0L
    private var scene: Int? = null
    private var surface: Surface? = null
    private var surfaceWidth = 0
    private var surfaceHeight = 0
    private var scheduled = false
    private var open = true

    init {
        NativeScene3d.configure(context)
        // Composited above the window like an overlay, with transparency where the scene draws nothing.
        setZOrderMediaOverlay(true)
        holder.setFormat(PixelFormat.TRANSLUCENT)
        holder.addCallback(this)
        importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_YES
    }

    /** Shows [scene], or nothing when it is null. */
    fun show(scene: Long?) {
        val wanted = scene?.toInt()
        if (!open || wanted == this.scene) return
        this.scene = wanted
        reattach()
    }

    fun close() {
        if (!open) return
        open = false
        detach()
        holder.removeCallback(this)
    }

    override fun surfaceCreated(holder: SurfaceHolder) {
        surface = holder.surface
    }

    override fun surfaceChanged(
        holder: SurfaceHolder,
        format: Int,
        width: Int,
        height: Int,
    ) {
        surface = holder.surface
        surfaceWidth = width
        surfaceHeight = height

        if (handle == 0L) reattach() else NativeScene3d.resize(handle, width, height)
        requestFrame()
    }

    override fun surfaceDestroyed(holder: SurfaceHolder) {
        // Rust must let the window go before Android does.
        detach()
        surface = null
    }

    private fun reattach() {
        detach()

        val scene = scene ?: return
        val surface = surface ?: return
        if (!open || surfaceWidth <= 0 || surfaceHeight <= 0 || !NativeScene3d.available()) return
        handle = NativeScene3d.attach(scene, surface, surfaceWidth, surfaceHeight, this)

        if (handle == 0L) {
            Log.w("LeptosKit", "a 3D scene could not attach: ${NativeScene3d.error()}")

            return
        }

        requestFrame()
    }

    private fun detach() {
        if (scheduled) Choreographer.getInstance().removeFrameCallback(this)
        scheduled = false

        if (handle != 0L) NativeScene3d.detach(handle)
        handle = 0L
    }

    /** Starts drawing on the next frame, if anything is attached. */
    fun requestFrame() {
        if (!open || scheduled || handle == 0L) return
        scheduled = true
        Choreographer.getInstance().postFrameCallback(this)
    }

    override fun doFrame(frameTimeNanos: Long) {
        scheduled = false

        if (handle == 0L) return
        if (NativeScene3d.frame(handle, frameTimeNanos / 1_000_000_000.0) == 1) requestFrame()
    }
}

/** The JNI functions behind [NativeScene3dView], in `app.cpp`. */
internal object NativeScene3d {
    private var configured = false

    /**
     * Chooses the graphics API once, from the `dev.leptoskit.scene3d.backend` manifest metadata (`gl`, the
     * default, or `vulkan`), and reads assets from the APK.
     */
    fun configure(context: Context) {
        if (configured) return
        configured = true
        val metadata =
            try {
                context.packageManager.getApplicationInfo(context.packageName, PackageManager.GET_META_DATA).metaData
            } catch (error: PackageManager.NameNotFoundException) {
                null
            }
        metadata?.getString("dev.leptoskit.scene3d.backend")?.let { setBackend(it) }
        setAssetReader(context.applicationContext.assets)
    }

    @JvmStatic external fun available(): Boolean

    @JvmStatic external fun error(): String

    @JvmStatic external fun setBackend(name: String): Int

    @JvmStatic external fun setAssetReader(assets: AssetManager): Int

    @JvmStatic external fun attach(
        scene: Int,
        surface: Surface,
        width: Int,
        height: Int,
        view: NativeScene3dView,
    ): Long

    @JvmStatic external fun resize(
        view: Long,
        width: Int,
        height: Int,
    )

    @JvmStatic external fun frame(
        view: Long,
        time: Double,
    ): Int

    @JvmStatic external fun detach(view: Long)

    /** Rust's waker, from any thread: asks the view for a frame on the UI thread. */
    @JvmStatic
    fun wake(view: NativeScene3dView) {
        view.post { view.requestFrame() }
    }

    /** Reads an asset for Rust's loader thread, or null when the APK has none at [path] (UTF-8 bytes). */
    @JvmStatic
    fun readAsset(
        assets: AssetManager,
        path: ByteArray,
    ): ByteArray? =
        try {
            // `leptoskit build` packs the app's `assets/` folder under the APK's `assets/`.
            assets.open("assets/" + String(path, Charsets.UTF_8)).use { it.readBytes() }
        } catch (error: IOException) {
            null
        }
}
