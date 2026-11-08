package dev.leptoskit.plugins.camera

import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CodingErrorAction

internal const val WIRE_VERSION = 1
internal const val MAX_BYTES = 16 * 1024 * 1024
internal const val MAX_URI_BYTES = 4096
internal const val MAX_MIME_BYTES = 256

internal object CameraWire {
    fun create(payload: ByteBuffer): Int {
        val input = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        if (input.remaining() != 4) throw PluginException(PluginException.INVALID_ARGUMENT)
        val facing = input.int
        if (facing !in 0..1) throw PluginException(PluginException.INVALID_ARGUMENT)
        return facing
    }

    fun session(payload: ByteBuffer): Int {
        val input = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        if (input.remaining() != 4) throw PluginException(PluginException.INVALID_ARGUMENT)
        return input.int
    }

    fun torch(payload: ByteBuffer): Pair<Int, Boolean> {
        val input = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        if (input.remaining() != 8) throw PluginException(PluginException.INVALID_ARGUMENT)
        val id = input.int
        val flag = input.int
        if (flag !in 0..1) throw PluginException(PluginException.INVALID_ARGUMENT)
        return id to (flag == 1)
    }

    fun takePicture(payload: ByteBuffer): Pair<Int, Int> {
        val input = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        if (input.remaining() != 8) throw PluginException(PluginException.INVALID_ARGUMENT)
        val id = input.int
        val maxBytes = input.int
        if (maxBytes.toLong() and 0xFFFF_FFFFL > MAX_BYTES.toLong()) {
            throw PluginException(PluginException.TOO_LARGE)
        }
        return id to maxBytes
    }

    fun sessionId(id: Int): ByteArray = words(id)

    fun status(
        facing: Int,
        torchOn: Boolean,
        torchAvailable: Boolean,
        active: Boolean,
    ): ByteArray =
        words(
            WIRE_VERSION,
            facing,
            if (torchOn) 1 else 0,
            if (torchAvailable) 1 else 0,
            if (active) 1 else 0,
        )

    fun asset(
        uri: String,
        mime: String?,
        width: Int?,
        height: Int?,
        size: Long?,
    ): ByteArray {
        if (uri.isEmpty() || uri.toByteArray(Charsets.UTF_8).size > MAX_URI_BYTES) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
        val mimeText = mime.orEmpty()
        if (mime != null && mime.isEmpty()) throw PluginException(PluginException.INVALID_ARGUMENT)
        if (mimeText.toByteArray(Charsets.UTF_8).size > MAX_MIME_BYTES) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
        val uriBytes = uri.toByteArray(Charsets.UTF_8)
        val mimeBytes = mimeText.toByteArray(Charsets.UTF_8)
        val buffer =
            ByteBuffer
                .allocate(8 + uriBytes.size + mimeBytes.size + 16)
                .order(ByteOrder.LITTLE_ENDIAN)
        buffer.putInt(uriBytes.size)
        buffer.put(uriBytes)
        buffer.putInt(mimeBytes.size)
        buffer.put(mimeBytes)
        buffer.putInt(width ?: -1)
        buffer.putInt(height ?: -1)
        val sizeValue = size ?: -1L
        buffer.putInt((sizeValue and 0xFFFF_FFFFL).toInt())
        buffer.putInt(((sizeValue ushr 32) and 0xFFFF_FFFFL).toInt())
        return buffer.array()
    }

    private fun words(vararg values: Int): ByteArray {
        val buffer = ByteBuffer.allocate(values.size * 4).order(ByteOrder.LITTLE_ENDIAN)
        values.forEach { buffer.putInt(it) }
        return buffer.array()
    }

    @Suppress("unused")
    private fun readString(
        input: ByteBuffer,
        max: Int,
    ): String {
        if (input.remaining() < 4) throw PluginException(PluginException.INVALID_ARGUMENT)
        val length = input.int
        if (length < 0 || length > max) {
            throw if (length > max) {
                PluginException(PluginException.TOO_LARGE)
            } else {
                PluginException(PluginException.INVALID_ARGUMENT)
            }
        }
        if (input.remaining() < length) throw PluginException(PluginException.INVALID_ARGUMENT)
        val bytes = ByteArray(length)
        input.get(bytes)
        return Charsets.UTF_8
            .newDecoder()
            .onMalformedInput(CodingErrorAction.REPORT)
            .onUnmappableCharacter(CodingErrorAction.REPORT)
            .decode(ByteBuffer.wrap(bytes))
            .toString()
    }
}
