package dev.leptoskit.plugins.medialibrary

import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction

internal enum class MediaKind(
    val code: Int,
) {
    Images(0),
    Videos(1),
    All(2),
    ;

    companion object {
        fun from(code: Int): MediaKind =
            entries.firstOrNull { it.code == code }
                ?: throw PluginException(PluginException.INVALID_ARGUMENT)
    }
}

internal data class ListOptions(
    val kind: MediaKind,
    val limit: Int,
) {
    companion object {
        fun decode(payload: ByteBuffer): ListOptions {
            val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
            val kind = MediaKind.from(readU32(bytes))
            val limit = readU32(bytes)
            if (bytes.hasRemaining()) throw invalid()
            if (limit <= 0 || limit > MAX_LIST) throw invalid()
            return ListOptions(kind, limit)
        }

        private const val MAX_LIST = 64
    }
}

internal data class SaveRequest(
    val uri: String,
    val kind: MediaKind,
) {
    companion object {
        fun decode(payload: ByteBuffer): SaveRequest {
            val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
            val kind = MediaKind.from(readU32(bytes))
            val uri = readString(bytes)
            if (bytes.hasRemaining()) throw invalid()
            if (kind == MediaKind.All) throw invalid()
            if (uri.isEmpty() || uri.length > MAX_URI || '\u0000' in uri) throw invalid()
            return SaveRequest(uri, kind)
        }

        private const val MAX_URI = 4096
    }
}

internal data class Asset(
    val id: String,
    val kind: MediaKind,
    val uri: String,
    val width: Int?,
    val height: Int?,
    val creationTimeMs: Long?,
) {
    fun encode(): ByteArray {
        val idBytes = id.toByteArray(Charsets.UTF_8)
        val uriBytes = uri.toByteArray(Charsets.UTF_8)
        val buffer =
            ByteBuffer
                .allocate(8 + idBytes.size + 4 + 8 + uriBytes.size + 8 + 8)
                .order(ByteOrder.LITTLE_ENDIAN)
        buffer.putInt(idBytes.size).put(idBytes)
        buffer.putInt(kind.code)
        buffer.putInt(uriBytes.size).put(uriBytes)
        buffer.putInt(width ?: -1)
        buffer.putInt(height ?: -1)
        val creation = creationTimeMs ?: -1L
        buffer.putInt((creation and 0xffffffffL).toInt())
        buffer.putInt(((creation ushr 32) and 0xffffffffL).toInt())
        return buffer.array()
    }
}

internal fun encodeAssets(assets: List<Asset>): ByteArray {
    val frames = assets.map { it.encode() }
    val size = 4 + frames.sumOf { 4 + it.size }
    val buffer = ByteBuffer.allocate(size).order(ByteOrder.LITTLE_ENDIAN)
    buffer.putInt(frames.size)
    for (frame in frames) {
        buffer.putInt(frame.size).put(frame)
    }
    return buffer.array()
}

private fun readU32(bytes: ByteBuffer): Int {
    if (bytes.remaining() < 4) throw invalid()
    val value = bytes.int
    if (value < 0) throw invalid()
    return value
}

private fun readString(bytes: ByteBuffer): String {
    val size = readU32(bytes)
    if (bytes.remaining() < size) throw invalid()
    // Android's ByteBuffer.limit returns Buffer, which has no order().
    val slice =
        bytes.slice().apply {
            limit(size)
            order(ByteOrder.LITTLE_ENDIAN)
        }
    bytes.position(bytes.position() + size)
    return try {
        Charsets.UTF_8
            .newDecoder()
            .onMalformedInput(CodingErrorAction.REPORT)
            .onUnmappableCharacter(CodingErrorAction.REPORT)
            .decode(slice)
            .toString()
    } catch (error: CharacterCodingException) {
        throw PluginException(PluginException.INVALID_ARGUMENT, error)
    }
}

private fun invalid() = PluginException(PluginException.INVALID_ARGUMENT)
