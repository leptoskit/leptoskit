package dev.leptoskit.plugins.blob

import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder

internal enum class Capability(
    val code: Int,
) {
    Restricted(0),
    Available(1),
    Unavailable(2),
}

internal object BlobWire {
    const val MAX_CREATE = 65536
    const val MAX_READ = 8192

    fun capability(value: Capability): ByteArray {
        val buffer = ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN)
        buffer.putInt(value.code)
        return buffer.array()
    }

    fun empty(payload: ByteBuffer) {
        if (payload.duplicate().remaining() != 0) throw invalid()
    }

    fun create(payload: ByteBuffer): ByteArray {
        val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        if (bytes.remaining() < 4) throw invalid()
        val length = bytes.int
        if (length <= 0 || length > MAX_CREATE || bytes.remaining() != length) throw invalid()
        val data = ByteArray(length)
        bytes.get(data)
        return data
    }

    fun read(payload: ByteBuffer): Triple<Int, Int, Int> {
        val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        if (bytes.remaining() != 12) throw invalid()
        val id = bytes.int
        val offset = bytes.int
        val length = bytes.int
        if (length <= 0 || length > MAX_READ) throw invalid()
        return Triple(id, offset, length)
    }

    fun id(payload: ByteBuffer): Int {
        val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        if (bytes.remaining() != 4) throw invalid()
        return bytes.int
    }

    fun encodeId(id: Int): ByteArray {
        val buffer = ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN)
        buffer.putInt(id)
        return buffer.array()
    }

    fun encodeSlice(data: ByteArray): ByteArray {
        if (data.size > MAX_READ) throw invalid()
        val buffer = ByteBuffer.allocate(4 + data.size).order(ByteOrder.LITTLE_ENDIAN)
        buffer.putInt(data.size)
        buffer.put(data)
        return buffer.array()
    }
}

private fun invalid(): Nothing = throw PluginException(PluginException.INVALID_ARGUMENT)
