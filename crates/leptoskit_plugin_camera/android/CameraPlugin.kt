package dev.leptoskit.plugins.camera

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.content.ContextWrapper
import android.graphics.BitmapFactory
import android.graphics.ImageFormat
import android.graphics.Rect
import android.graphics.YuvImage
import android.os.Handler
import android.os.Looper
import android.view.ViewGroup
import android.widget.FrameLayout
import androidx.camera.core.Camera
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageCapture
import androidx.camera.core.ImageCaptureException
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.core.content.ContextCompat
import androidx.lifecycle.LifecycleOwner
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import dev.leptoskit.mobile.NativeCameraBinding
import java.io.ByteArrayOutputStream
import java.io.File
import java.nio.ByteBuffer
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executor
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlinx.coroutines.suspendCancellableCoroutine

/**
 * Host-owned CameraX registry. Surfaces attach through [NativeCameraBinding]
 * into a [NativeCameraView] frame. Runtime camera permission is checked here;
 * prompting belongs to the `permissions` plugin.
 */
class CameraPlugin(
    context: Context,
) : NativePlugin {
    override val name = "camera"
    override val methods = intArrayOf(0, 1, 2, 3, 4)

    private val hostContext = context
    private val appContext = context.applicationContext
    private val handler = Handler(Looper.getMainLooper())
    private val mainExecutor: Executor = ContextCompat.getMainExecutor(appContext)
    private val sessions = LinkedHashMap<Int, Session>()
    private val surfaces = HashMap<Int, PreviewView>()
    private var nextId = 1
    private var closed = false
    private var provider: ProcessCameraProvider? = null

    private class Session(
        val facing: Int,
        var camera: Camera?,
        var imageCapture: ImageCapture,
        var preview: Preview,
        var torchOn: Boolean = false,
        var active: Boolean = true,
    ) {
        val torchAvailable: Boolean
            get() = camera?.cameraInfo?.hasFlashUnit() == true
    }

    init {
        NativeCameraBinding.attach = { frame, id -> attach(frame, id.toInt()) }
        NativeCameraBinding.detach = { frame -> detach(frame) }
    }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (closed) throw PluginException(PluginException.CANCELLED)
        if (method !in methods) throw PluginException(PluginException.UNAVAILABLE)
        return when (method) {
            0 -> create(CameraWire.create(payload))
            1 -> {
                val (id, enabled) = CameraWire.torch(payload)
                setTorch(id, enabled)
                ByteArray(0)
            }
            2 -> {
                val (id, maxBytes) = CameraWire.takePicture(payload)
                takePicture(id, maxBytes)
            }
            3 -> status(CameraWire.session(payload))
            else -> {
                release(CameraWire.session(payload))
                ByteArray(0)
            }
        }
    }

    override fun close() {
        closed = true
        ArrayList(sessions.keys).forEach { releaseQuiet(it) }
        if (NativeCameraBinding.attach != null) {
            NativeCameraBinding.attach = null
            NativeCameraBinding.detach = null
        }
        provider = null
    }

    private suspend fun create(facing: Int): ByteArray {
        requireCameraPermission()
        val provider = cameraProvider()
        val selector =
            if (facing == 0) {
                CameraSelector.DEFAULT_BACK_CAMERA
            } else {
                CameraSelector.DEFAULT_FRONT_CAMERA
            }
        val preview = Preview.Builder().build()
        val imageCapture =
            ImageCapture
                .Builder()
                .setCaptureMode(ImageCapture.CAPTURE_MODE_MINIMIZE_LATENCY)
                .build()
        val camera =
            bindOnMain(provider, selector, preview, imageCapture)
                ?: throw PluginException(PluginException.UNAVAILABLE)
        val id = nextId
        nextId = if (nextId == Int.MAX_VALUE) 1 else nextId + 1
        sessions[id] = Session(facing, camera, imageCapture, preview)
        return CameraWire.sessionId(id)
    }

    private fun setTorch(
        id: Int,
        enabled: Boolean,
    ) {
        val session = sessions[id] ?: throw PluginException(PluginException.INVALID_ARGUMENT)
        if (!session.torchAvailable) throw PluginException(PluginException.UNAVAILABLE)
        session.camera?.cameraControl?.enableTorch(enabled)
        session.torchOn = enabled
    }

    private suspend fun takePicture(
        id: Int,
        maxBytes: Int,
    ): ByteArray {
        requireCameraPermission()
        val session = sessions[id] ?: throw PluginException(PluginException.INVALID_ARGUMENT)
        val bytes = captureJpeg(session.imageCapture)
        if (bytes.size > (maxBytes.toLong() and 0xFFFF_FFFFL)) {
            throw PluginException(PluginException.TOO_LARGE)
        }
        val file = File(appContext.cacheDir, "leptoskit-camera-${System.nanoTime()}.jpg")
        file.parentFile?.mkdirs()
        file.writeBytes(bytes)
        val options = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options)
        val width = options.outWidth.takeIf { it > 0 }
        val height = options.outHeight.takeIf { it > 0 }
        return CameraWire.asset(
            uri = file.toURI().toString(),
            mime = "image/jpeg",
            width = width,
            height = height,
            size = bytes.size.toLong(),
        )
    }

    private fun status(id: Int): ByteArray {
        val session = sessions[id] ?: throw PluginException(PluginException.INVALID_ARGUMENT)
        return CameraWire.status(
            facing = session.facing,
            torchOn = session.torchOn,
            torchAvailable = session.torchAvailable,
            active = session.active,
        )
    }

    private fun release(id: Int) {
        if (!sessions.containsKey(id)) throw PluginException(PluginException.INVALID_ARGUMENT)
        releaseQuiet(id)
    }

    private fun releaseQuiet(id: Int) {
        surfaces.remove(id)?.let { view ->
            (view.parent as? ViewGroup)?.removeView(view)
        }
        sessions.remove(id)?.let { session ->
            if (session.torchOn && session.torchAvailable) {
                session.camera?.cameraControl?.enableTorch(false)
            }
            session.active = false
            session.camera = null
        }
        // Rebinding is per-session; releasing unbinds all for MVP honesty.
        provider?.unbindAll()
    }

    private fun requireCameraPermission() {
        val granted =
            ContextCompat.checkSelfPermission(appContext, Manifest.permission.CAMERA) ==
                PackageManager.PERMISSION_GRANTED
        if (!granted) throw PluginException(PluginException.PERMISSION_DENIED)
    }

    private suspend fun cameraProvider(): ProcessCameraProvider {
        provider?.let { return it }
        return suspendCancellableCoroutine { continuation ->
            val future = ProcessCameraProvider.getInstance(appContext)
            future.addListener(
                {
                    try {
                        val value = future.get()
                        provider = value
                        continuation.resume(value)
                    } catch (error: Exception) {
                        continuation.resumeWithException(
                            PluginException(PluginException.UNAVAILABLE, error.message),
                        )
                    }
                },
                mainExecutor,
            )
        }
    }

    private fun bindOnMain(
        provider: ProcessCameraProvider,
        selector: CameraSelector,
        preview: Preview,
        imageCapture: ImageCapture,
    ): Camera? {
        val result = AtomicReference<Camera?>(null)
        val error = AtomicReference<Exception?>(null)
        val done = CountDownLatch(1)
        handler.post {
            try {
                val owner = lifecycleOwner()
                provider.unbindAll()
                result.set(provider.bindToLifecycle(owner, selector, preview, imageCapture))
            } catch (failure: Exception) {
                error.set(failure)
            } finally {
                done.countDown()
            }
        }
        if (!done.await(10, TimeUnit.SECONDS)) {
            throw PluginException(PluginException.UNAVAILABLE)
        }
        error.get()?.let { throw PluginException(PluginException.UNAVAILABLE, it.message) }
        return result.get()
    }

    private suspend fun captureJpeg(imageCapture: ImageCapture): ByteArray =
        suspendCancellableCoroutine { continuation ->
            imageCapture.takePicture(
                mainExecutor,
                object : ImageCapture.OnImageCapturedCallback() {
                    override fun onCaptureSuccess(image: ImageProxy) {
                        try {
                            continuation.resume(imageProxyToJpeg(image))
                        } catch (error: Exception) {
                            continuation.resumeWithException(
                                PluginException(PluginException.NATIVE, error.message),
                            )
                        } finally {
                            image.close()
                        }
                    }

                    override fun onError(exception: ImageCaptureException) {
                        continuation.resumeWithException(
                            PluginException(PluginException.NATIVE, exception.message),
                        )
                    }
                },
            )
        }

    private fun imageProxyToJpeg(image: ImageProxy): ByteArray {
        val buffer = image.planes[0].buffer
        val bytes = ByteArray(buffer.remaining())
        buffer.get(bytes)
        if (image.format == ImageFormat.JPEG) return bytes
        // Fallback for YUV preview buffers when JPEG is unavailable.
        val yuv =
            YuvImage(
                bytes,
                ImageFormat.NV21,
                image.width,
                image.height,
                null,
            )
        val out = ByteArrayOutputStream()
        yuv.compressToJpeg(Rect(0, 0, image.width, image.height), 90, out)
        return out.toByteArray()
    }

    private fun lifecycleOwner(): LifecycleOwner {
        var current: Context? = hostContext
        while (current is ContextWrapper) {
            if (current is LifecycleOwner) return current
            current = current.baseContext
        }
        throw PluginException(PluginException.UNAVAILABLE)
    }

    private fun attach(
        frame: FrameLayout,
        id: Int,
    ) {
        val session = sessions[id] ?: return
        handler.post {
            val existing = surfaces[id]
            if (existing?.parent === frame) return@post
            (existing?.parent as? ViewGroup)?.removeView(existing)
            val view =
                PreviewView(frame.context).apply {
                    layoutParams =
                        FrameLayout.LayoutParams(
                            ViewGroup.LayoutParams.MATCH_PARENT,
                            ViewGroup.LayoutParams.MATCH_PARENT,
                        )
                    scaleType = PreviewView.ScaleType.FILL_CENTER
                }
            session.preview.setSurfaceProvider(view.surfaceProvider)
            surfaces[id] = view
            frame.removeAllViews()
            frame.addView(view)
        }
    }

    private fun detach(frame: FrameLayout) {
        handler.post {
            val child = frame.getChildAt(0) as? PreviewView ?: return@post
            val id = surfaces.entries.firstOrNull { it.value === child }?.key
            frame.removeAllViews()
            if (id != null) {
                sessions[id]?.preview?.setSurfaceProvider(null)
                surfaces.remove(id)
            }
        }
    }
}
