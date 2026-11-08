package dev.leptoskit.plugins.livephoto

import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CodingErrorAction

internal object LivePhotoWire {
    const val MAX_URI_BYTES = 4096
    const val WIRE_VERSION = 1

    fun create(payload: ByteBuffer): String {
        val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        val uri = readString(bytes)
        if (bytes.hasRemaining()) throw invalid()
        if (uri.isEmpty() || '\u0000' in uri || uri.toByteArray(Charsets.UTF_8).size > MAX_URI_BYTES) {
            throw invalid()
        }
        return uri
    }

    fun session(payload: ByteBuffer): Int {
        val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        val id = readU32(bytes)
        if (bytes.hasRemaining()) throw invalid()
        return id
    }

    fun startPlayback(payload: ByteBuffer): Pair<Int, Int> {
        val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        val id = readU32(bytes)
        val style = readU32(bytes)
        if (bytes.hasRemaining()) throw invalid()
        if (style > 1) throw invalid()
        return id to style
    }

    fun sessionId(id: Int): ByteArray {
        val buffer = ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN)
        buffer.putInt(id)
        return buffer.array()
    }

    fun status(
        active: Boolean,
        playing: Boolean,
    ): ByteArray {
        val buffer = ByteBuffer.allocate(12).order(ByteOrder.LITTLE_ENDIAN)
        buffer.putInt(WIRE_VERSION)
        buffer.putInt(if (active) 1 else 0)
        buffer.putInt(if (playing) 1 else 0)
        return buffer.array()
    }
}

private fun readU32(bytes: ByteBuffer): Int {
    if (bytes.remaining() < 4) throw invalid()
    val value = bytes.int
    if (value < 0) throw invalid()
    return value
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
