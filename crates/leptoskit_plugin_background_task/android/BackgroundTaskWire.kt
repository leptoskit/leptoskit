package dev.leptoskit.plugins.backgroundtask

import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CodingErrorAction

internal enum class Status(
    val code: Int,
) {
    Restricted(0),
    Available(1),
    Unavailable(2),
}

internal data class Register(
    val name: String,
    val intervalSecs: Int,
)

internal object BackgroundTaskWire {
    const val MAX_NAME_BYTES = 128
    const val MIN_INTERVAL = 900
    const val MAX_INTERVAL = 86_400

    fun status(value: Status): ByteArray {
        val buffer = ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN)
        buffer.putInt(value.code)
        return buffer.array()
    }

    fun register(payload: ByteBuffer): Register {
        val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        val interval = readU32(bytes)
        val name = readString(bytes)
        if (bytes.hasRemaining()) throw invalid()
        if (interval !in MIN_INTERVAL..MAX_INTERVAL) throw invalid()
        validateName(name)
        return Register(name, interval)
    }

    fun name(payload: ByteBuffer): String {
        val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        val name = readString(bytes)
        if (bytes.hasRemaining()) throw invalid()
        validateName(name)
        return name
    }

    private fun validateName(name: String) {
        if (name.isEmpty() || '\u0000' in name || name.toByteArray(Charsets.UTF_8).size > MAX_NAME_BYTES) {
            throw invalid()
        }
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
