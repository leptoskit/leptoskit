package dev.leptoskit.mobile

import android.content.Context
import android.content.pm.PackageManager
import android.content.res.AssetManager
import android.graphics.PixelFormat
import android.view.Choreographer
import android.view.Surface
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.ViewGroup
import java.io.IOException

/**
 * A `Scene3d` element: a [SurfaceView] that Rust draws the scene into, with OpenGL ES by default.
 *
 * The guest's ops reach the scene through the `leptoskit.scene3d` plugin, never through this view. It
 * attaches while its surface exists, draws on the [Choreographer] only while the scene changes or animates,
 * and is woken by Rust when the scene changes while it is idle.
 *
 * While the scene draws a live camera picture behind it (its backdrop is the camera and an XR session is
 * running), the view covers its whole area, so it moves below the window's own views and the interface laid
 * over it stays visible. Otherwise it stays above them, transparent around its models.
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
    private var overlay = true

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
            NativeLog.warn(NativeLog.GPU, "a 3D scene could not attach: ${NativeScene3d.error()}")

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
        NativeScene3d.setRotation(handle, display?.rotation ?: Surface.ROTATION_0)
        val more = NativeScene3d.frame(handle, frameTimeNanos / 1_000_000_000.0) == 1
        restack(NativeScene3d.showsCamera(handle))
        if (more) requestFrame()
    }

    /**
     * Puts the surface below the window's views while it shows the camera, and above them otherwise. A
     * [SurfaceView]'s layer only changes while it is out of its window, so the view leaves its parent and
     * comes back in the same place; its surface is made again and it reattaches.
     */
    private fun restack(camera: Boolean) {
        val wanted = !camera
        if (wanted == overlay) return
        overlay = wanted
        val parent = parent as? ViewGroup ?: return
        post {
            val index = parent.indexOfChild(this)
            if (!open || index < 0) return@post
            val params = layoutParams
            parent.removeViewAt(index)
            setZOrderMediaOverlay(overlay)
            parent.addView(this, index, params)
        }
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

    @JvmStatic external fun setRotation(
        view: Long,
        quarterTurns: Int,
    )

    @JvmStatic external fun showsCamera(view: Long): Boolean

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
