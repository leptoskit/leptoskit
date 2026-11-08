package dev.leptoskit.plugins.agerange

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

internal object AgeRangeWire {
    const val MIN_THRESHOLD = 1
    const val MAX_THRESHOLD = 120

    fun capability(value: Capability): ByteArray {
        val buffer = ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN)
        buffer.putInt(value.code)
        return buffer.array()
    }

    fun responseUnavailable(): ByteArray {
        val buffer = ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN)
        buffer.putInt(1)
        return buffer.array()
    }

    fun threshold(payload: ByteBuffer): Int {
        val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        if (bytes.remaining() != 4) throw invalid()
        val value = bytes.int
        if (value < MIN_THRESHOLD || value > MAX_THRESHOLD) throw invalid()
        return value
    }
}

private fun invalid(): Nothing = throw PluginException(PluginException.INVALID_ARGUMENT)
