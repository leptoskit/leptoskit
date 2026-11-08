package dev.leptoskit.plugins.mailcomposer

import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction

internal data class MailCompose(
    val to: List<String>,
    val cc: List<String>,
    val bcc: List<String>,
    val subject: String,
    val body: String,
) {
    companion object {
        fun decode(payload: ByteBuffer): MailCompose {
            val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
            val to = readList(bytes)
            val cc = readList(bytes)
            val bcc = readList(bytes)
            val subject = bounded(readString(bytes), MAX_SUBJECT_BYTES)
            val body = bounded(readString(bytes), MAX_BODY_BYTES)

            if (bytes.hasRemaining()) throw invalid()

            return MailCompose(to, cc, bcc, subject, body)
        }

        private fun readList(bytes: ByteBuffer): List<String> {
            val count = readU32(bytes)

            if (count > MAX_ADDRESSES) throw PluginException(PluginException.TOO_LARGE)

            return List(count) { validateAddress(readString(bytes)) }
        }

        private fun validateAddress(value: String): String {
            if (value.isEmpty() || value.length > MAX_ADDRESS_BYTES || '\u0000' in value) throw invalid()

            if (!value.all { it.code < 128 } || value.any { it.isWhitespace() }) throw invalid()

            val parts = value.split("@")

            if (parts.size != 2 || parts[0].isEmpty() || parts[1].isEmpty()) throw invalid()

            return value
        }

        private fun bounded(
            value: String,
            max: Int,
        ): String {
            if (value.length > max) throw PluginException(PluginException.TOO_LARGE)

            if ('\u0000' in value) throw invalid()

            return value
        }

        private fun readU32(bytes: ByteBuffer): Int {
            if (bytes.remaining() < 4) throw invalid()

            val value = bytes.int

            if (value < 0) throw invalid()

            return value
        }

        private fun readString(bytes: ByteBuffer): String {
            val size = readU32(bytes)

            if (bytes.remaining() < size) throw invalid()

            val slice = bytes.slice().limit(size).order(ByteOrder.LITTLE_ENDIAN)
            bytes.position(bytes.position() + size)

            return try {
                Charsets.UTF_8
                    .newDecoder()
                    .onMalformedInput(CodingErrorAction.REPORT)
                    .onUnmappableCharacter(CodingErrorAction.REPORT)
                    .decode(slice)
                    .toString()
            } catch (error: CharacterCodingException) {
                throw PluginException(PluginException.INVALID_ARGUMENT, error)
            }
        }

        private fun invalid() = PluginException(PluginException.INVALID_ARGUMENT)

        private const val MAX_ADDRESSES = 32
        private const val MAX_ADDRESS_BYTES = 320
        private const val MAX_SUBJECT_BYTES = 998
        private const val MAX_BODY_BYTES = 64 * 1024
    }
}
