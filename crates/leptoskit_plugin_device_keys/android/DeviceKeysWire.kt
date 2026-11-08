package dev.leptoskit.plugins.devicekeys

import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction

/** Wire version every request and reply carries. */
internal const val WIRE_VERSION = 1

/** Every wire integer is one little-endian u32. */
private const val FIELD_BYTES = 4

/** Terminal status codes the replies carry. */
internal const val STATUS_OK = 0
internal const val STATUS_NOT_FOUND = 1
internal const val STATUS_ALREADY_EXISTS = 2
internal const val STATUS_CANCELLED = 3
internal const val STATUS_LOCKED_OUT = 4
internal const val STATUS_NOT_ENROLLED = 5
internal const val STATUS_INVALIDATED = 6
internal const val STATUS_UNAVAILABLE = 7
internal const val STATUS_FAILED = 8

/** Hardware kinds the availability reply carries. */
internal const val HARDWARE_SECURE_ENCLAVE = 0
internal const val HARDWARE_STRONGBOX = 1
internal const val HARDWARE_TEE = 2
internal const val HARDWARE_NONE = 3

/** Biometric hardware kinds the availability reply carries. */
internal const val BIOMETRY_TOUCH_ID = 0
internal const val BIOMETRY_FACE_ID = 1
internal const val BIOMETRY_OPTIC_ID = 2
internal const val BIOMETRY_FINGERPRINT = 3
internal const val BIOMETRY_FACE = 4
internal const val BIOMETRY_IRIS = 5
internal const val BIOMETRY_NONE = 6

/** Authentication policies the requests carry. */
internal const val POLICY_BIOMETRY = 0
internal const val POLICY_BIOMETRY_OR_PASSCODE = 1

/** Longest accepted alias, in ASCII bytes. */
internal const val MAX_ALIAS_BYTES = 64

/** Longest accepted message, in bytes. */
internal const val MAX_MESSAGE_BYTES = 4096

/** Longest accepted reason, in UTF-8 bytes. */
internal const val MAX_REASON_BYTES = 200

/** Builds a little-endian u32 field payload from the given values. */
internal fun deviceKeysFields(vararg values: Int): ByteArray {
    val output = ByteBuffer.allocate(values.size * FIELD_BYTES).order(ByteOrder.LITTLE_ENDIAN)

    for (value in values) output.putInt(value)

    return output.array()
}

/** A decoded `generate` request. */
internal data class GenerateRequest(
    val policy: Int,
    val alias: String,
) {
    companion object {
        fun decode(payload: ByteBuffer): GenerateRequest {
            val input = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)

            if (input.remaining() < 2 * FIELD_BYTES || input.int != WIRE_VERSION) {
                throw PluginException(PluginException.INVALID_ARGUMENT)
            }

            val policy = input.int

            if (policy != POLICY_BIOMETRY && policy != POLICY_BIOMETRY_OR_PASSCODE) {
                throw PluginException(PluginException.INVALID_ARGUMENT)
            }
            val alias = alias(input)

            if (input.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

            return GenerateRequest(policy, alias)
        }
    }
}

/** A decoded `sign` request. */
internal data class SignRequest(
    val alias: String,
    val message: ByteArray,
    val reason: String,
) {
    companion object {
        fun decode(payload: ByteBuffer): SignRequest {
            val input = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)

            if (input.remaining() < FIELD_BYTES || input.int != WIRE_VERSION) {
                throw PluginException(PluginException.INVALID_ARGUMENT)
            }
            val alias = alias(input)
            val message = data(input, MAX_MESSAGE_BYTES, nonEmpty = true)
            val reason = text(input, MAX_REASON_BYTES)

            if (input.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

            return SignRequest(alias, message, reason)
        }
    }
}

/** Decodes a length-prefixed alias a public-key or delete request carries. */
internal fun decodeAlias(payload: ByteBuffer): String {
    val input = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
    val alias = alias(input)

    if (input.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

    return alias
}

/** One alias: 1–64 ASCII letters, digits, `-`, `.`, or `_`. */
private fun alias(input: ByteBuffer): String {
    val value = field(input, MAX_ALIAS_BYTES)

    if (value.isEmpty() || value.any { !isNameByte(it) }) {
        throw PluginException(PluginException.INVALID_ARGUMENT)
    }

    return value.toString(Charsets.US_ASCII)
}

/** One non-blank UTF-8 string, rejecting NUL. */
private fun text(
    input: ByteBuffer,
    maxBytes: Int,
): String {
    val value = field(input, maxBytes)
    val decoded = decodeUtf8(value)

    if (decoded.isBlank() || '\u0000' in decoded) throw PluginException(PluginException.INVALID_ARGUMENT)

    return decoded
}

/** One length-prefixed binary field. */
private fun data(
    input: ByteBuffer,
    maxBytes: Int,
    nonEmpty: Boolean,
): ByteArray {
    val value = field(input, maxBytes)

    if (nonEmpty && value.isEmpty()) throw PluginException(PluginException.INVALID_ARGUMENT)

    return value
}

/** Reads one length-prefixed field, refusing one past its limit. */
private fun field(
    input: ByteBuffer,
    maxBytes: Int,
): ByteArray {
    if (input.remaining() < FIELD_BYTES) throw PluginException(PluginException.INVALID_ARGUMENT)
    val count = input.int

    if (count < 0 || count > maxBytes) throw PluginException(PluginException.TOO_LARGE)

    if (count > input.remaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

    return ByteArray(count).also(input::get)
}

/** Whether one byte may appear in an alias. */
private fun isNameByte(byte: Byte): Boolean {
    val code = byte.toInt()

    return code in 65..90 || code in 97..122 || code in 48..57 || code == 45 || code == 46 || code == 95
}

/** Decodes strict UTF-8, so malformed bytes are an argument error. */
private fun decodeUtf8(bytes: ByteArray): String =
    try {
        Charsets.UTF_8
            .newDecoder()
            .onMalformedInput(CodingErrorAction.REPORT)
            .onUnmappableCharacter(CodingErrorAction.REPORT)
            .decode(ByteBuffer.wrap(bytes))
            .toString()
    } catch (error: CharacterCodingException) {
        throw PluginException(PluginException.INVALID_ARGUMENT, error)
    }
