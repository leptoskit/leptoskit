package dev.leptoskit.plugins.appintegrity

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

internal object AppIntegrityWire {
    const val MAX_CHALLENGE = 256

    fun capability(value: Capability): ByteArray {
        val buffer = ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN)
        buffer.putInt(value.code)
        return buffer.array()
    }

    fun attestationUnavailable(): ByteArray {
        val buffer = ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN)
        buffer.putInt(0)
        return buffer.array()
    }

    fun challenge(payload: ByteBuffer): ByteArray {
        val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        if (bytes.remaining() < 4) throw invalid()
        val length = bytes.int
        if (length <= 0 || length > MAX_CHALLENGE || bytes.remaining() != length) throw invalid()
        val challenge = ByteArray(length)
        bytes.get(challenge)
        return challenge
    }
}

private fun invalid(): Nothing = throw PluginException(PluginException.INVALID_ARGUMENT)
