package dev.leptoskit.plugins.diagnostics

import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CodingErrorAction

internal enum class Capability(
    val code: Int,
) {
    Restricted(0),
    Available(1),
    Unavailable(2),
}

internal object DiagnosticsWire {
    const val MAX_NAME = 128

    fun capability(value: Capability): ByteArray {
        val buffer = ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN)
        buffer.putInt(value.code)
        return buffer.array()
    }

    fun empty(payload: ByteBuffer) {
        if (payload.duplicate().remaining() != 0) throw invalid()
    }

    fun name(payload: ByteBuffer): String {
        val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        if (bytes.remaining() < 4) throw invalid()
        val length = bytes.int
        if (length <= 0 || length > MAX_NAME || bytes.remaining() != length) throw invalid()
        val slice = ByteArray(length)
        bytes.get(slice)
        val name =
            try {
                Charsets.UTF_8
                    .newDecoder()
                    .onMalformedInput(CodingErrorAction.REPORT)
                    .onUnmappableCharacter(CodingErrorAction.REPORT)
                    .decode(ByteBuffer.wrap(slice))
                    .toString()
            } catch (_: CharacterCodingException) {
                throw invalid()
            }
        if (name.isEmpty() || '\u0000' in name) throw invalid()
        return name
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
}

private fun invalid(): Nothing = throw PluginException(PluginException.INVALID_ARGUMENT)
