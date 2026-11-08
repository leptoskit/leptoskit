package dev.leptoskit.plugins.video

import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CodingErrorAction

internal const val WIRE_VERSION = 1
internal const val MAX_URL_BYTES = 2048
internal const val RATE_MIN = 250
internal const val RATE_MAX = 2000

internal object VideoWire {
    const val TIME_UNKNOWN_U = -1 // UInt32.MAX as little-endian Int

    fun source(payload: ByteBuffer): String {
        val input = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        if (input.remaining() < 8 || input.int != WIRE_VERSION) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
        val url = readString(input, MAX_URL_BYTES)
        if (url.isEmpty()) throw PluginException(PluginException.INVALID_ARGUMENT)
        requireFinished(input)
        return url
    }

    fun replace(payload: ByteBuffer): Pair<Int, String> {
        val input = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        if (input.remaining() < 12) throw PluginException(PluginException.INVALID_ARGUMENT)
        val id = input.int
        if (input.int != WIRE_VERSION) throw PluginException(PluginException.INVALID_ARGUMENT)
        val url = readString(input, MAX_URL_BYTES)
        if (url.isEmpty()) throw PluginException(PluginException.INVALID_ARGUMENT)
        requireFinished(input)
        return id to url
    }

    fun player(payload: ByteBuffer): Int {
        val input = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        if (input.remaining() != 4) throw PluginException(PluginException.INVALID_ARGUMENT)
        return input.int
    }

    fun seek(payload: ByteBuffer): Pair<Int, Int> {
        val input = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        if (input.remaining() != 8) throw PluginException(PluginException.INVALID_ARGUMENT)
        val id = input.int
        val position = input.int
        if (position == TIME_UNKNOWN_U) throw PluginException(PluginException.INVALID_ARGUMENT)
        return id to position
    }

    fun flag(payload: ByteBuffer): Pair<Int, Boolean> {
        val input = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        if (input.remaining() != 8) throw PluginException(PluginException.INVALID_ARGUMENT)
        val id = input.int
        val flag = input.int
        if (flag !in 0..1) throw PluginException(PluginException.INVALID_ARGUMENT)
        return id to (flag == 1)
    }

    fun rate(payload: ByteBuffer): Pair<Int, Int> {
        val input = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        if (input.remaining() != 8) throw PluginException(PluginException.INVALID_ARGUMENT)
        val id = input.int
        val thousandths = input.int
        if (thousandths !in RATE_MIN..RATE_MAX) throw PluginException(PluginException.INVALID_ARGUMENT)
        return id to thousandths
    }

    fun playerId(id: Int): ByteArray = words(id)

    fun status(
        playing: Boolean,
        currentMs: Int,
        durationMs: Int,
    ): ByteArray = words(WIRE_VERSION, if (playing) 1 else 0, currentMs, durationMs)

    private fun words(vararg values: Int): ByteArray {
        val buffer = ByteBuffer.allocate(values.size * 4).order(ByteOrder.LITTLE_ENDIAN)
        values.forEach(buffer::putInt)
        return buffer.array()
    }

    private fun readString(
        input: ByteBuffer,
        max: Int,
    ): String {
        if (input.remaining() < 4) throw PluginException(PluginException.INVALID_ARGUMENT)
        val length = input.int
        if (length < 0) throw PluginException(PluginException.INVALID_ARGUMENT)
        if (length > max) throw PluginException(PluginException.TOO_LARGE)
        if (input.remaining() < length) throw PluginException(PluginException.INVALID_ARGUMENT)
        val bytes = ByteArray(length)
        input.get(bytes)
        return try {
            Charsets.UTF_8
                .newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT)
                .decode(ByteBuffer.wrap(bytes))
                .toString()
        } catch (_: CharacterCodingException) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
    }

    private fun requireFinished(input: ByteBuffer) {
        if (input.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
    }
}
