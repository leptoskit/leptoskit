package dev.leptoskit.plugins.calendar

import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction

internal data class CalendarRecord(
    val id: String,
    val title: String,
    val allowsModifications: Boolean,
) {
    fun encode(): ByteArray {
        val idBytes = id.toByteArray(Charsets.UTF_8)
        val titleBytes = title.toByteArray(Charsets.UTF_8)

        return ByteBuffer
            .allocate(12 + idBytes.size + titleBytes.size)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(idBytes.size)
            .put(idBytes)
            .putInt(titleBytes.size)
            .put(titleBytes)
            .putInt(if (allowsModifications) 1 else 0)
            .array()
    }

    companion object {
        const val MAX_CALENDARS = 64
        private const val MAX_ID_BYTES = 256
        private const val MAX_TITLE_BYTES = 256

        fun create(
            id: String,
            title: String,
            allowsModifications: Boolean,
        ): CalendarRecord =
            CalendarRecord(
                id = required(id, MAX_ID_BYTES),
                title = required(title, MAX_TITLE_BYTES),
                allowsModifications = allowsModifications,
            )

        fun encodeList(calendars: List<CalendarRecord>): ByteArray {
            if (calendars.size > MAX_CALENDARS) throw PluginException(PluginException.TOO_LARGE)

            val encoded = calendars.map { it.encode() }
            val size = 4 + encoded.sumOf { it.size }
            val result = ByteBuffer.allocate(size).order(ByteOrder.LITTLE_ENDIAN)
            result.putInt(calendars.size)
            for (bytes in encoded) {
                result.put(bytes)
            }

            return result.array()
        }

        fun decode(payload: ByteBuffer): CalendarRecord {
            // Advance the caller's cursor so a list can decode the next calendar.
            val bytes = payload.order(ByteOrder.LITTLE_ENDIAN)
            val id = required(readString(bytes), MAX_ID_BYTES)
            val title = required(readString(bytes), MAX_TITLE_BYTES)
            val flag = bytes.int
            if (flag != 0 && flag != 1) throw invalid()
            if (bytes.hasRemaining()) throw invalid()

            return CalendarRecord(id, title, flag == 1)
        }

        private fun required(
            value: String,
            max: Int,
        ): String {
            if (value.isEmpty() || value.length > max || '\u0000' in value) throw invalid()

            return value
        }

        private fun readString(bytes: ByteBuffer): String = CalendarEventRecord.readString(bytes)

        private fun invalid() = PluginException(PluginException.INVALID_ARGUMENT)
    }
}

internal data class CalendarEventRecord(
    val id: String,
    val calendarId: String,
    val title: String,
    val startMs: Long,
    val endMs: Long,
    val notes: String?,
    val location: String?,
) {
    fun encode(): ByteArray {
        val idBytes = id.toByteArray(Charsets.UTF_8)
        val calendarBytes = calendarId.toByteArray(Charsets.UTF_8)
        val titleBytes = title.toByteArray(Charsets.UTF_8)
        val notesBytes = (notes ?: "").toByteArray(Charsets.UTF_8)
        val locationBytes = (location ?: "").toByteArray(Charsets.UTF_8)

        return ByteBuffer
            .allocate(
                // Five length prefixes and two timestamps: 5 * 4 + 2 * 8.
                36 + idBytes.size + calendarBytes.size + titleBytes.size + notesBytes.size +
                    locationBytes.size,
            ).order(ByteOrder.LITTLE_ENDIAN)
            .putInt(idBytes.size)
            .put(idBytes)
            .putInt(calendarBytes.size)
            .put(calendarBytes)
            .putInt(titleBytes.size)
            .put(titleBytes)
            .putLong(startMs)
            .putLong(endMs)
            .putInt(notesBytes.size)
            .put(notesBytes)
            .putInt(locationBytes.size)
            .put(locationBytes)
            .array()
    }

    companion object {
        const val MAX_EVENTS = 64
        const val MAX_WINDOW_MS = 31_622_400_000L
        private const val MAX_ID_BYTES = 256
        private const val MAX_TITLE_BYTES = 256
        private const val MAX_NOTES_BYTES = 4096
        private const val MAX_LOCATION_BYTES = 512

        fun create(
            id: String,
            calendarId: String,
            title: String,
            startMs: Long,
            endMs: Long,
            notes: String?,
            location: String?,
        ): CalendarEventRecord {
            if (endMs < startMs) throw invalid()

            return CalendarEventRecord(
                id = required(id, MAX_ID_BYTES),
                calendarId = required(calendarId, MAX_ID_BYTES),
                title = required(title, MAX_TITLE_BYTES),
                startMs = startMs,
                endMs = endMs,
                notes = optional(notes, MAX_NOTES_BYTES),
                location = optional(location, MAX_LOCATION_BYTES),
            )
        }

        fun encodeList(events: List<CalendarEventRecord>): ByteArray {
            if (events.size > MAX_EVENTS) throw PluginException(PluginException.TOO_LARGE)

            val encoded = events.map { it.encode() }
            val size = 4 + encoded.sumOf { it.size }
            val result = ByteBuffer.allocate(size).order(ByteOrder.LITTLE_ENDIAN)
            result.putInt(events.size)
            for (bytes in encoded) {
                result.put(bytes)
            }

            return result.array()
        }

        fun decode(payload: ByteBuffer): CalendarEventRecord {
            val bytes = payload.order(ByteOrder.LITTLE_ENDIAN)
            val id = required(readString(bytes), MAX_ID_BYTES)
            val calendarId = required(readString(bytes), MAX_ID_BYTES)
            val title = required(readString(bytes), MAX_TITLE_BYTES)
            if (bytes.remaining() < 16) throw invalid()

            val startMs = bytes.long
            val endMs = bytes.long
            val notes = optional(readString(bytes).ifEmpty { null }, MAX_NOTES_BYTES)
            val location = optional(readString(bytes).ifEmpty { null }, MAX_LOCATION_BYTES)
            if (bytes.hasRemaining()) throw invalid()

            return create(id, calendarId, title, startMs, endMs, notes, location)
        }

        fun decodeCreate(payload: ByteBuffer): CreateRequest {
            val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
            val calendarId = required(readString(bytes), MAX_ID_BYTES)
            val title = required(readString(bytes), MAX_TITLE_BYTES)
            if (bytes.remaining() < 16) throw invalid()

            val startMs = bytes.long
            val endMs = bytes.long
            if (endMs < startMs) throw invalid()

            val notes = optional(readString(bytes).ifEmpty { null }, MAX_NOTES_BYTES)
            val location = optional(readString(bytes).ifEmpty { null }, MAX_LOCATION_BYTES)
            if (bytes.hasRemaining()) throw invalid()

            return CreateRequest(calendarId, title, startMs, endMs, notes, location)
        }

        fun decodeEventId(payload: ByteBuffer): String {
            val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
            val id = required(readString(bytes), MAX_ID_BYTES)
            if (bytes.hasRemaining()) throw invalid()

            return id
        }

        fun encodeEventId(id: String): ByteArray {
            val value = required(id, MAX_ID_BYTES).toByteArray(Charsets.UTF_8)

            return ByteBuffer
                .allocate(4 + value.size)
                .order(ByteOrder.LITTLE_ENDIAN)
                .putInt(value.size)
                .put(value)
                .array()
        }

        fun decodeListQuery(payload: ByteBuffer): ListQuery {
            val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
            if (bytes.remaining() != 20) throw invalid()

            val startMs = bytes.long
            val endMs = bytes.long
            val limit = bytes.int
            if (endMs < startMs) throw invalid()

            val span = endMs - startMs
            if (span < 0 || span > MAX_WINDOW_MS) throw invalid()
            if (limit < 1 || limit > MAX_EVENTS) throw invalid()

            return ListQuery(startMs, endMs, limit)
        }

        fun readString(bytes: ByteBuffer): String {
            if (bytes.remaining() < 4) throw invalid()

            val size = bytes.int
            if (size < 0 || bytes.remaining() < size) throw invalid()

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

        private fun invalid() = PluginException(PluginException.INVALID_ARGUMENT)
    }
}

internal data class CreateRequest(
    val calendarId: String,
    val title: String,
    val startMs: Long,
    val endMs: Long,
    val notes: String?,
    val location: String?,
)

internal data class ListQuery(
    val startMs: Long,
    val endMs: Long,
    val limit: Int,
)
