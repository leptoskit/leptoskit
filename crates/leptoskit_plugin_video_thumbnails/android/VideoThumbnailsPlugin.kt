package dev.leptoskit.plugins.videothumbnails

import android.content.Context
import android.graphics.Bitmap
import android.media.MediaMetadataRetriever
import android.net.Uri
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.io.File
import java.io.FileOutputStream
import java.nio.ByteBuffer
import java.util.UUID

/** Extracts one JPEG frame from an app-owned video URI. */
class VideoThumbnailsPlugin(
    private val hostContext: Context,
) : NativePlugin {
    override val name: String = "video-thumbnails"
    override val methods: IntArray = intArrayOf(0)

    private var closed = false

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (closed) throw PluginException(PluginException.CANCELLED)
        if (method != 0) throw PluginException(PluginException.UNAVAILABLE)
        val request = VideoThumbnailsWire.getThumbnail(payload)
        return extract(request)
    }

    override fun close() {
        closed = true
    }

    private fun extract(request: GetThumbnail): ByteArray {
        val uri = Uri.parse(request.uri)
        if (uri.scheme != "file" && uri.scheme != "content") {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
        val retriever = MediaMetadataRetriever()
        try {
            if (uri.scheme == "file") {
                retriever.setDataSource(uri.path)
            } else {
                retriever.setDataSource(hostContext, uri)
            }
            val bitmap =
                retriever.getFrameAtTime(
                    request.timeMs * 1000L,
                    MediaMetadataRetriever.OPTION_CLOSEST_SYNC,
                ) ?: throw PluginException(PluginException.UNAVAILABLE)
            val file =
                File(
                    hostContext.cacheDir,
                    "lk-video-thumb-${UUID.randomUUID()}.jpg",
                )
            FileOutputStream(file).use { output ->
                if (!bitmap.compress(Bitmap.CompressFormat.JPEG, request.quality, output)) {
                    throw PluginException(PluginException.UNAVAILABLE)
                }
            }
            return VideoThumbnailsWire.thumbnail(
                uri = file.toURI().toString(),
                width = bitmap.width,
                height = bitmap.height,
            )
        } catch (error: PluginException) {
            throw error
        } catch (_: Exception) {
            throw PluginException(PluginException.UNAVAILABLE)
        } finally {
            runCatching { retriever.release() }
        }
    }
}
