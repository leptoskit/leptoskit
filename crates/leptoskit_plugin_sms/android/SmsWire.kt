package dev.leptoskit.plugins.sms

import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction

internal data class SmsCompose(
    val recipients: List<String>,
    val body: String,
) {
    companion object {
        fun decode(payload: ByteBuffer): SmsCompose {
            val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
            val count = readU32(bytes)

            if (count > MAX_RECIPIENTS) throw PluginException(PluginException.TOO_LARGE)

            val recipients = ArrayList<String>(count)

            repeat(count) { recipients.add(validateRecipient(readString(bytes))) }

            val body = validateBody(readString(bytes))

            if (bytes.hasRemaining()) throw invalid()

            return SmsCompose(recipients, body)
        }

        private fun validateRecipient(value: String): String {
            if (value.isEmpty() || value.length > MAX_RECIPIENT_BYTES) throw invalid()

            if (value.any { !it.isDigit() && it !in "+- ()" } || value.none { it.isDigit() }) {
                throw invalid()
            }

            return value
        }

        private fun validateBody(value: String): String {
            if (value.length > MAX_BODY_BYTES) throw PluginException(PluginException.TOO_LARGE)

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

            // Android's ByteBuffer.limit returns Buffer, which has no order().
            val slice =
                bytes.slice().apply {
                    limit(size)
                    order(ByteOrder.LITTLE_ENDIAN)
                }
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

        private const val MAX_RECIPIENTS = 32
        private const val MAX_RECIPIENT_BYTES = 64
        private const val MAX_BODY_BYTES = 8 * 1024
    }
}
