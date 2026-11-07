package dev.leptoskit.plugins.liveactivities

import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction
import java.nio.charset.StandardCharsets

/**
 * Android has no ActivityKit Live Activities; absence is never a synthetic
 * success. Payloads are validated before Unavailable so malformed calls stay
 * InvalidArgument on every host.
 */
class LiveActivitiesPlugin : NativePlugin {
    override val name = "live-activities"
    override val methods = intArrayOf(0, 1, 2, 3, 4, 5, 6)
    private var closed = false

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (closed) throw PluginException(PluginException.CANCELLED)

        if (method !in methods) throw PluginException(PluginException.UNAVAILABLE)

        when (method) {
            0, 1, 6 -> requireEmpty(payload)
            2 -> StartMessage.decode(payload)
            3 -> UpdateMessage.decode(payload)
            4 -> EndMessage.decode(payload)
            5 -> {
                val reader = Reader(payload, 68)
                reader.identifier()
                reader.finish()
            }
            else -> throw PluginException(PluginException.UNAVAILABLE)
        }

        throw PluginException(PluginException.UNAVAILABLE)
    }

    override fun close() {
        closed = true
    }
}

private fun requireEmpty(payload: ByteBuffer) {
    if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
}

private object StartMessage {
    fun decode(payload: ByteBuffer) {
        val reader = Reader(payload, 4096)
        reader.identifier()
        reader.identifier()
        reader.text(128, required = true)

        when (reader.u32()) {
            0, 1 -> Unit
            else -> throw PluginException(PluginException.INVALID_ARGUMENT)
        }
        reader.fields()
        reader.finish()
    }
}

private object UpdateMessage {
    fun decode(payload: ByteBuffer) {
        val reader = Reader(payload, 4096)
        reader.identifier()
        reader.fields()
        reader.finish()
    }
}

private object EndMessage {
    fun decode(payload: ByteBuffer) {
        val reader = Reader(payload, 4096)
        reader.identifier()
        val kind = reader.u32()
        val after = reader.u32()

        when (kind) {
            0, 1 -> if (after != 0) throw PluginException(PluginException.INVALID_ARGUMENT)
            2 -> if (after !in 1..86_400) throw PluginException(PluginException.INVALID_ARGUMENT)
            else -> throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        when (reader.u32()) {
            0 -> Unit
            1 -> reader.fields()
            else -> throw PluginException(PluginException.INVALID_ARGUMENT)
        }
        reader.finish()
    }
}

private class Reader(
    private val buffer: ByteBuffer,
    maximum: Int,
) {
    private val endExclusive = buffer.position() + buffer.remaining()

    init {
        if (buffer.remaining() > maximum) throw PluginException(PluginException.INVALID_ARGUMENT)
        buffer.order(ByteOrder.LITTLE_ENDIAN)
    }

    fun u32(): Int {
        if (buffer.remaining() < 4) throw PluginException(PluginException.INVALID_ARGUMENT)
        return buffer.int
    }

    fun identifier(): String {
        val value = text(64, required = true)

        if (!value.all { it.isLetterOrDigit() || it == '-' || it == '_' || it == '.' }) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
        return value
    }

    fun text(
        maximum: Int,
        required: Boolean,
    ): String {
        if (buffer.remaining() < 4) throw PluginException(PluginException.INVALID_ARGUMENT)
        val length = buffer.int

        if (length < 0 || length > maximum || length > buffer.remaining()) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        val bytes = ByteArray(length)
        buffer.get(bytes)
        val value =
            try {
                StandardCharsets.UTF_8
                    .newDecoder()
                    .onMalformedInput(CodingErrorAction.REPORT)
                    .onUnmappableCharacter(CodingErrorAction.REPORT)
                    .decode(ByteBuffer.wrap(bytes))
                    .toString()
            } catch (_: CharacterCodingException) {
                throw PluginException(PluginException.INVALID_ARGUMENT)
            }

        if (required && value.trim().isEmpty()) throw PluginException(PluginException.INVALID_ARGUMENT)

        if (value.any { it.code <= 31 || it.code in 127..159 }) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
        return value
    }

    fun fields() {
        val count = u32()

        if (count !in 0..16) throw PluginException(PluginException.INVALID_ARGUMENT)
        val seen = HashSet<String>()

        repeat(count) {
            val key = identifier()

            if (!seen.add(key)) throw PluginException(PluginException.INVALID_ARGUMENT)
            text(256, required = false)
        }
    }

    fun finish() {
        if (buffer.position() != endExclusive) throw PluginException(PluginException.INVALID_ARGUMENT)
    }
}
