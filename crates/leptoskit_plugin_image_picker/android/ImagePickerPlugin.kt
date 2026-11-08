package dev.leptoskit.plugins.imagepicker

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.graphics.BitmapFactory
import android.net.Uri
import android.os.Looper
import android.provider.OpenableColumns
import android.webkit.MimeTypeMap
import dev.leptoskit.bridge.NativeActivityResultHost
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import java.io.File
import java.io.IOException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.UUID

/**
 * Picks one library image or video through the activity-result broker, copies
 * it into the app cache, and returns a temporary `file:` URI with metadata.
 * Camera capture is deferred to a later camera plugin.
 */
class ImagePickerPlugin(
    context: Context,
) : NativePlugin {
    override val name = "image-picker"
    override val methods = intArrayOf(PICK)
    private val activity = context as? Activity
    private val host = context as? NativeActivityResultHost
    private var busy = false
    private var closed = false
    private var pendingJob: Job? = null

    override fun close() {
        closed = true
        pendingJob?.cancel()
    }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        check(Looper.myLooper() == Looper.getMainLooper()) { "Image presentation requires the UI thread" }

        if (closed) throw PluginException(PluginException.CANCELLED)

        if (method != PICK) throw PluginException(PluginException.UNAVAILABLE)

        val options = readOptions(payload)

        if (busy) throw PluginException(PluginException.BUSY)

        val activity = activity ?: throw PluginException(PluginException.UNAVAILABLE)
        val host = host ?: throw PluginException(PluginException.UNAVAILABLE)
        busy = true
        pendingJob = currentCoroutineContext()[Job]

        try {
            return pick(activity, host, options)
        } finally {
            busy = false
            pendingJob = null
        }
    }

    private fun readOptions(payload: ByteBuffer): Options {
        if (payload.remaining() != REQUEST_BYTES) throw PluginException(PluginException.INVALID_ARGUMENT)

        val buffer = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        val media = buffer.int
        val maxBytes = buffer.int

        if (media !in IMAGES..ALL) throw PluginException(PluginException.INVALID_ARGUMENT)

        if (maxBytes !in 0..MAX_BYTES) throw PluginException(PluginException.TOO_LARGE)

        return Options(media, maxBytes)
    }

    private suspend fun pick(
        activity: Activity,
        host: NativeActivityResultHost,
        options: Options,
    ): ByteArray {
        val result =
            try {
                host.launchPluginActivity(openLibrary(options.media))
            } catch (error: SecurityException) {
                throw PluginException(PluginException.PERMISSION_DENIED, error)
            }

        if (result.resultCode == Activity.RESULT_CANCELED) throw PluginException(PluginException.CANCELLED)

        if (result.resultCode != Activity.RESULT_OK) throw PluginException(PluginException.NATIVE)

        val uri = result.data?.data ?: throw PluginException(PluginException.NATIVE)

        if (uri.scheme != CONTENT_SCHEME && uri.scheme != FILE_SCHEME) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        val asset =
            try {
                withContext(Dispatchers.IO) { import(activity, uri, options.maxBytes) }
            } catch (error: SecurityException) {
                throw PluginException(PluginException.PERMISSION_DENIED, error)
            } catch (error: IOException) {
                throw PluginException(PluginException.NATIVE, error)
            }

        if (closed) throw PluginException(PluginException.CANCELLED)

        return encode(asset)
    }

    private fun openLibrary(media: Int): Intent =
        Intent(Intent.ACTION_GET_CONTENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            when (media) {
                VIDEOS -> {
                    type = "video/*"
                }

                ALL -> {
                    type = "*/*"
                    putExtra(Intent.EXTRA_MIME_TYPES, arrayOf("image/*", "video/*"))
                }

                else -> {
                    type = "image/*"
                }
            }
        }

    private suspend fun import(
        activity: Activity,
        uri: Uri,
        limit: Int,
    ): Asset {
        currentCoroutineContext().ensureActive()
        val resolver = activity.contentResolver
        val size = readSize(resolver, uri)
        if (size != null && size > limit) throw PluginException(PluginException.TOO_LARGE)

        val mime = resolver.getType(uri).orEmpty()
        if (mime.length > MAX_MIME_BYTES) throw PluginException(PluginException.TOO_LARGE)

        val extension =
            MimeTypeMap.getSingleton().getExtensionFromMimeType(mime)
                ?: uri.lastPathSegment?.substringAfterLast('.', "")?.takeIf { it.length in 1..16 }
                ?: "bin"
        val directory = File(activity.cacheDir, "leptoskit-image-picker").apply { mkdirs() }
        val destination = File(directory, "${UUID.randomUUID()}.$extension")
        resolver.openInputStream(uri)?.use { input ->
            destination.outputStream().use { output ->
                val buffer = ByteArray(CHUNK_BYTES)
                var total = 0
                while (true) {
                    currentCoroutineContext().ensureActive()
                    val count = input.read(buffer)
                    if (count < 0) break
                    if (count == 0) throw PluginException(PluginException.NATIVE)
                    if (total + count > limit) throw PluginException(PluginException.TOO_LARGE)
                    output.write(buffer, 0, count)
                    total += count
                }
            }
        } ?: throw PluginException(PluginException.NATIVE)

        val written = destination.length()
        if (written > limit) {
            destination.delete()
            throw PluginException(PluginException.TOO_LARGE)
        }

        val dimensions = if (mime.startsWith("image/")) imageSize(destination) else null
        return Asset(
            uri = Uri.fromFile(destination).toString(),
            mime = mime,
            width = dimensions?.first,
            height = dimensions?.second,
            size = written.toULong(),
        )
    }

    private fun readSize(
        resolver: android.content.ContentResolver,
        uri: Uri,
    ): Long? {
        resolver.query(uri, arrayOf(OpenableColumns.SIZE), null, null, null)?.use { cursor ->
            if (cursor.moveToFirst()) {
                val index = cursor.getColumnIndex(OpenableColumns.SIZE)
                if (index >= 0 && !cursor.isNull(index)) return cursor.getLong(index)
            }
        }

        return null
    }

    private fun imageSize(file: File): Pair<Int, Int>? {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeFile(file.absolutePath, bounds)
        if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null

        return bounds.outWidth to bounds.outHeight
    }

    private fun encode(asset: Asset): ByteArray {
        val uri = asset.uri.toByteArray(Charsets.UTF_8)
        val mime = asset.mime.toByteArray(Charsets.UTF_8)
        if (uri.isEmpty() || uri.size > MAX_URI_BYTES || mime.size > MAX_MIME_BYTES) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        val size = asset.size.toLong()
        return ByteBuffer
            .allocate(24 + uri.size + mime.size)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(uri.size)
            .put(uri)
            .putInt(mime.size)
            .put(mime)
            .putInt(asset.width ?: -1)
            .putInt(asset.height ?: -1)
            .putInt(size.toInt())
            .putInt((size ushr 32).toInt())
            .array()
    }

    private data class Options(
        val media: Int,
        val maxBytes: Int,
    )

    private data class Asset(
        val uri: String,
        val mime: String,
        val width: Int?,
        val height: Int?,
        val size: ULong,
    )

    private companion object {
        const val PICK = 0
        const val REQUEST_BYTES = 8
        const val MAX_BYTES = 16 * 1024 * 1024
        const val MAX_URI_BYTES = 4096
        const val MAX_MIME_BYTES = 256
        const val CHUNK_BYTES = 65536
        const val IMAGES = 0
        const val VIDEOS = 1
        const val ALL = 2
        const val CONTENT_SCHEME = "content"
        const val FILE_SCHEME = "file"
    }
}
