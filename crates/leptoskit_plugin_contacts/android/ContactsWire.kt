package dev.leptoskit.plugins.contacts

import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction

internal data class ContactsRecord(
    val id: String,
    val displayName: String,
    val phone: String?,
    val email: String?,
) {
    fun encode(): ByteArray {
        val idBytes = id.toByteArray(Charsets.UTF_8)
        val nameBytes = displayName.toByteArray(Charsets.UTF_8)
        val phoneBytes = (phone ?: "").toByteArray(Charsets.UTF_8)
        val emailBytes = (email ?: "").toByteArray(Charsets.UTF_8)

        return ByteBuffer
            .allocate(16 + idBytes.size + nameBytes.size + phoneBytes.size + emailBytes.size)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(idBytes.size)
            .put(idBytes)
            .putInt(nameBytes.size)
            .put(nameBytes)
            .putInt(phoneBytes.size)
            .put(phoneBytes)
            .putInt(emailBytes.size)
            .put(emailBytes)
            .array()
    }

    companion object {
        const val MAX_QUERY = 32
        private const val MAX_ID_BYTES = 256
        private const val MAX_NAME_BYTES = 256
        private const val MAX_PHONE_BYTES = 64
        private const val MAX_EMAIL_BYTES = 320

        fun create(
            id: String,
            displayName: String,
            phone: String?,
            email: String?,
        ): ContactsRecord =
            ContactsRecord(
                id = required(id, MAX_ID_BYTES),
                displayName = required(displayName, MAX_NAME_BYTES),
                phone = optional(phone, MAX_PHONE_BYTES),
                email = optional(email, MAX_EMAIL_BYTES),
            )

        fun decodeQueryLimit(payload: ByteBuffer): Int {
            val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
            if (bytes.remaining() != 4) throw invalid()

            val limit = bytes.int
            if (limit < 1 || limit > MAX_QUERY) throw invalid()

            return limit
        }

        fun encodeList(contacts: List<ContactsRecord>): ByteArray {
            if (contacts.size > MAX_QUERY) throw PluginException(PluginException.TOO_LARGE)

            val encoded = contacts.map { it.encode() }
            val size = 4 + encoded.sumOf { it.size }
            val result = ByteBuffer.allocate(size).order(ByteOrder.LITTLE_ENDIAN)
            result.putInt(contacts.size)
            for (bytes in encoded) {
                result.put(bytes)
            }

            return result.array()
        }

        fun decode(payload: ByteBuffer): ContactsRecord {
            val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
            val id = required(readString(bytes), MAX_ID_BYTES)
            val name = required(readString(bytes), MAX_NAME_BYTES)
            val phone = optional(readString(bytes).ifEmpty { null }, MAX_PHONE_BYTES)
            val email = optional(readString(bytes).ifEmpty { null }, MAX_EMAIL_BYTES)
            if (bytes.hasRemaining()) throw invalid()

            return ContactsRecord(id, name, phone, email)
        }

        private fun required(
            value: String,
            max: Int,
        ): String {
            if (value.isEmpty() || value.length > max || '\u0000' in value) throw invalid()

            return value
        }

        private fun optional(
            value: String?,
            max: Int,
        ): String? {
            if (value == null) return null
            if (value.isEmpty() || value.length > max || '\u0000' in value) throw invalid()

            return value
        }

        private fun readString(bytes: ByteBuffer): String {
            if (bytes.remaining() < 4) throw invalid()

            val size = bytes.int
            if (size < 0 || bytes.remaining() < size) throw invalid()

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
    }
}
