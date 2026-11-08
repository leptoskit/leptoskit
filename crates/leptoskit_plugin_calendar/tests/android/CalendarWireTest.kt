package dev.leptoskit.plugins.calendar

import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder

private fun calendarBytes(
    id: String,
    title: String,
    allows: Boolean,
): ByteBuffer {
    val record = CalendarRecord.create(id, title, allows)

    return ByteBuffer.wrap(record.encode()).order(ByteOrder.LITTLE_ENDIAN)
}

private fun eventBytes(
    id: String,
    calendarId: String,
    title: String,
    startMs: Long,
    endMs: Long,
    notes: String?,
    location: String?,
): ByteBuffer {
    val record = CalendarEventRecord.create(id, calendarId, title, startMs, endMs, notes, location)

    return ByteBuffer.wrap(record.encode()).order(ByteOrder.LITTLE_ENDIAN)
}

/** Exercises calendar wire framing without opening the calendar store. */
fun main() {
    val calendar = CalendarRecord.decode(calendarBytes("1", "Work", true))
    check(calendar.id == "1")
    check(calendar.title == "Work")
    check(calendar.allowsModifications)

    for (bad in listOf("", "a\u0000b")) {
        try {
            CalendarRecord.create(bad, "n", false)
            error("Expected rejection: $bad")
        } catch (error: PluginException) {
            check(error.code == PluginException.INVALID_ARGUMENT)
        }
    }

    val event =
        CalendarEventRecord.decode(
            eventBytes("e1", "1", "Standup", 1_700_000_000_000L, 1_700_000_360_000L, "daily", "Room A"),
        )
    check(event.id == "e1")
    check(event.calendarId == "1")
    check(event.title == "Standup")
    check(event.notes == "daily")
    check(event.location == "Room A")

    try {
        CalendarEventRecord.create("e", "c", "t", 2, 1, null, null)
        error("Expected inverted range rejection")
    } catch (error: PluginException) {
        check(error.code == PluginException.INVALID_ARGUMENT)
    }

    fun query(
        start: Long,
        end: Long,
        limit: Int,
    ): ByteBuffer =
        ByteBuffer
            .allocate(20)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putLong(start)
            .putLong(end)
            .putInt(limit)
            .also { it.flip() }

    try {
        CalendarEventRecord.decodeListQuery(query(0, 1, 0))
        error("Expected zero-limit rejection")
    } catch (error: PluginException) {
        check(error.code == PluginException.INVALID_ARGUMENT)
    }

    try {
        CalendarEventRecord.decodeListQuery(query(0, 1, 65))
        error("Expected over-limit rejection")
    } catch (error: PluginException) {
        check(error.code == PluginException.INVALID_ARGUMENT)
    }

    check(CalendarEventRecord.decodeListQuery(query(0, 1_000, 8)).limit == 8)

    val create =
        ByteBuffer
            .allocate(64)
            .order(ByteOrder.LITTLE_ENDIAN)
    val calendarId = "9".toByteArray(Charsets.UTF_8)
    val title = "Meet".toByteArray(Charsets.UTF_8)
    create
        .putInt(calendarId.size)
        .put(calendarId)
        .putInt(title.size)
        .put(title)
        .putLong(10)
        .putLong(20)
        .putInt(0)
        .putInt(0)
        .flip()
    val decoded = CalendarEventRecord.decodeCreate(create)
    check(decoded.calendarId == "9")
    check(decoded.title == "Meet")
    check(decoded.notes == null)

    val list = CalendarRecord.encodeList(listOf(CalendarRecord.create("1", "Work", false)))
    val buffer = ByteBuffer.wrap(list).order(ByteOrder.LITTLE_ENDIAN)
    check(buffer.int == 1)
    check(CalendarRecord.decode(buffer).title == "Work")
    check(!buffer.hasRemaining())

    val id = CalendarEventRecord.encodeEventId("42")
    check(CalendarEventRecord.decodeEventId(ByteBuffer.wrap(id).order(ByteOrder.LITTLE_ENDIAN)) == "42")

    println("Calendar synthetic wire and bounds checks passed")
}
