package dev.leptoskit.mobile

import android.content.Context
import android.content.pm.PackageManager
import android.content.res.AssetManager
import android.graphics.SurfaceTexture
import android.view.Choreographer
import android.view.Surface
import android.view.TextureView
import java.io.IOException

/**
 * A `Scene3d` element: a transparent [TextureView] that Rust draws into, with OpenGL ES by default.
 *
 * The guest's ops reach the scene through the `leptoskit.scene3d` plugin, never through this view. It
 * attaches while its surface exists, draws on the [Choreographer] only while the scene changes or animates,
 * and is woken by Rust when the scene changes while it is idle.
 *
 * The GPU surface participates in the normal view hierarchy: page backgrounds stay behind the model,
 * and controls placed after it stay above it, including over a live camera backdrop. A SurfaceView's
 * media-overlay layer sits behind the window; putting it above the window instead hides the controls.
 */
internal class NativeScene3dView(
    context: Context,
) : TextureView(context),
    TextureView.SurfaceTextureListener,
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
        isOpaque = false
        surfaceTextureListener = this
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
        releaseSurface()
        surfaceTextureListener = null
    }

    override fun onSurfaceTextureAvailable(
        texture: SurfaceTexture,
        width: Int,
        height: Int,
    ) {
        if (!open) return
        surface = Surface(texture)
        surfaceWidth = width
        surfaceHeight = height
        reattach()
    }

    override fun onSurfaceTextureSizeChanged(
        texture: SurfaceTexture,
        width: Int,
        height: Int,
    ) {
        surfaceWidth = width
        surfaceHeight = height
        if (handle == 0L) reattach() else NativeScene3d.resize(handle, width, height)
        requestFrame()
    }

    override fun onSurfaceTextureDestroyed(texture: SurfaceTexture): Boolean {
        // Rust must let the window go before Android does.
        detach()
        releaseSurface()
        return true
    }

    override fun onSurfaceTextureUpdated(texture: SurfaceTexture) = Unit

    private fun releaseSurface() {
        surface?.release()
        surface = null
        surfaceWidth = 0
        surfaceHeight = 0
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
        if (more) requestFrame()
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
