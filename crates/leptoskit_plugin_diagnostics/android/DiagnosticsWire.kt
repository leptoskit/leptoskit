package dev.leptoskit.plugins.diagnostics

import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CodingErrorAction

internal object DiagnosticsWire {
    const val MAX_NAME = 128

    fun name(payload: ByteBuffer): String {
        val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        if (bytes.remaining() < 4) throw invalid()
        val length = bytes.int
        if (length <= 0 || length > MAX_NAME || bytes.remaining() != length) throw invalid()
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

    fun id(payload: ByteBuffer): Int {
        val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        if (bytes.remaining() != 4) throw invalid()
        return bytes.int
    }
}

private fun invalid(): Nothing = throw PluginException(PluginException.INVALID_ARGUMENT)
