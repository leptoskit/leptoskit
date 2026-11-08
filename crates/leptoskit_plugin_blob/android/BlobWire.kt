package dev.leptoskit.plugins.blob

import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder

internal object BlobWire {
    const val MAX_CREATE = 65536
    const val MAX_READ = 8192

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
}

private fun invalid(): Nothing = throw PluginException(PluginException.INVALID_ARGUMENT)
