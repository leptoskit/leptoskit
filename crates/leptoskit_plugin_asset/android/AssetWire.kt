package dev.leptoskit.plugins.asset

import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CodingErrorAction

internal enum class Capability(val code: Int) { Restricted(0), Available(1), Unavailable(2) }

internal object AssetWire {
    const val MAX_NAME = 256
    fun capability(value: Capability): ByteArray {
        val buffer = ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN)
        buffer.putInt(value.code)
        return buffer.array()
    }
    fun resolvedUnavailable(): ByteArray {
        val buffer = ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN)
        buffer.putInt(0)
        return buffer.array()
    }
    fun name(payload: ByteBuffer): String {
        val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        if (bytes.remaining() < 4) throw invalid()
        val length = bytes.int
        if (length <= 0 || length > MAX_NAME || bytes.remaining() != length) throw invalid()
        val slice = ByteArray(length)
        bytes.get(slice)
        return try {
            Charsets.UTF_8.newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT)
                .decode(ByteBuffer.wrap(slice)).toString()
        } catch (_: CharacterCodingException) { throw invalid() }
    }
}
private fun invalid(): Nothing = throw PluginException(PluginException.INVALID_ARGUMENT)
