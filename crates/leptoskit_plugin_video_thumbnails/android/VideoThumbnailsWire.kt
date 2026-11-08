package dev.leptoskit.plugins.videothumbnails

import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CodingErrorAction

internal data class GetThumbnail(
    val uri: String,
    val timeMs: Long,
    val quality: Int,
)

internal object VideoThumbnailsWire {
    const val MAX_URI_BYTES = 4096
    const val MIN_QUALITY = 1
    const val MAX_QUALITY = 100

    fun getThumbnail(payload: ByteBuffer): GetThumbnail {
        val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        val timeMs = readU64(bytes)
        val quality = readU32(bytes)
        val uri = readString(bytes)
        if (bytes.hasRemaining()) throw invalid()
        if (uri.isEmpty() || '\u0000' in uri || uri.toByteArray(Charsets.UTF_8).size > MAX_URI_BYTES) {
            throw invalid()
        }
        if (timeMs == -1L) throw invalid()
        if (quality !in MIN_QUALITY..MAX_QUALITY) throw invalid()
        return GetThumbnail(uri, timeMs, quality)
    }

    fun thumbnail(
        uri: String,
        width: Int,
        height: Int,
    ): ByteArray {
        if (uri.isEmpty() || '\u0000' in uri || uri.toByteArray(Charsets.UTF_8).size > MAX_URI_BYTES) {
            throw invalid()
        }
        if (width <= 0 || height <= 0) throw invalid()
        val utf8 = uri.toByteArray(Charsets.UTF_8)
        val buffer = ByteBuffer.allocate(12 + utf8.size).order(ByteOrder.LITTLE_ENDIAN)
        buffer.putInt(width)
        buffer.putInt(height)
        buffer.putInt(utf8.size).put(utf8)
        return buffer.array()
    }
}

private fun readU32(bytes: ByteBuffer): Int {
    if (bytes.remaining() < 4) throw invalid()
    val value = bytes.int
    if (value < 0) throw invalid()
    return value
}

private fun readU64(bytes: ByteBuffer): Long {
    if (bytes.remaining() < 8) throw invalid()
    val low = bytes.int.toLong() and 0xffffffffL
    val high = bytes.int.toLong() and 0xffffffffL
    return low or (high shl 32)
}

private fun readString(bytes: ByteBuffer): String {
    val length = readU32(bytes)
    if (bytes.remaining() < length) throw invalid()
    val slice = ByteArray(length)
    bytes.get(slice)
    return try {
        Charsets.UTF_8
            .newDecoder()
            .onMalformedInput(CodingErrorAction.REPORT)
            .onUnmappableCharacter(CodingErrorAction.REPORT)
            .decode(ByteBuffer.wrap(slice))
            .toString()
    } catch (_: CharacterCodingException) {
        throw invalid()
    }
}

private fun invalid(): Nothing = throw PluginException(PluginException.INVALID_ARGUMENT)
