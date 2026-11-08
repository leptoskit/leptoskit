package dev.leptoskit.plugins.calendar

import android.Manifest
import android.app.Activity
import android.content.ContentUris
import android.content.ContentValues
import android.content.Context
import android.content.pm.PackageManager
import android.database.Cursor
import android.os.Looper
import android.provider.CalendarContract
import dev.leptoskit.bridge.NativePermissionHost
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.TimeZone

/**
 * Calendar permission plus bounded calendar and event access. Denied access
 * never invents calendars or events. Purpose / manifest declarations stay
 * app-owned.
 */
class CalendarPlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "calendar"
    override val methods =
        intArrayOf(STATUS, REQUEST_PERMISSION, LIST_CALENDARS, CREATE_EVENT, GET_EVENT, LIST_EVENTS)
    private val activity = context as? Activity
    private val permissionHost = context as? NativePermissionHost
    private var busy = false
    private var closed = false
    private var pendingJob: Job? = null

    override fun close() {
        closed = true
        pendingJob?.cancel()
    }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        check(Looper.myLooper() == Looper.getMainLooper()) { "Calendar UI requires the UI thread" }

        if (closed) throw PluginException(PluginException.CANCELLED)

        if (method !in methods) throw PluginException(PluginException.UNAVAILABLE)

        return when (method) {
            STATUS -> {
                if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

                encode(status())
            }
            REQUEST_PERMISSION -> {
                if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

                exclusive { requestPermission() }
            }
            LIST_CALENDARS -> {
                if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

                exclusive { listCalendars() }
            }
            CREATE_EVENT -> {
                val request = CalendarEventRecord.decodeCreate(payload)
                exclusive { createEvent(request) }
            }
            GET_EVENT -> {
                val id = CalendarEventRecord.decodeEventId(payload)
                exclusive { getEvent(id) }
            }
            else -> {
                val query = CalendarEventRecord.decodeListQuery(payload)
                exclusive { listEvents(query) }
            }
        }
    }

    private suspend fun exclusive(operation: suspend () -> ByteArray): ByteArray {
        if (busy) throw PluginException(PluginException.BUSY)

        busy = true
        pendingJob = currentCoroutineContext()[Job]
        try {
            return operation()
        } finally {
            busy = false
            pendingJob = null
        }
    }

    private fun status(): Int {
        if (granted()) return GRANTED

        val activity = activity
        val rationale =
            activity?.shouldShowRequestPermissionRationale(Manifest.permission.READ_CALENDAR) == true ||
                activity?.shouldShowRequestPermissionRationale(Manifest.permission.WRITE_CALENDAR) == true

        return if (rationale) DENIED else PROMPT
    }

    private fun granted(): Boolean =
        granted(Manifest.permission.READ_CALENDAR) && granted(Manifest.permission.WRITE_CALENDAR)

    private fun granted(permission: String): Boolean =
        context.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED

    private fun declaresPermissions(): Boolean {
        val declared =
            context.packageManager
                .getPackageInfo(context.packageName, PackageManager.GET_PERMISSIONS)
                .requestedPermissions
                ?.toSet()
                .orEmpty()

        return Manifest.permission.READ_CALENDAR in declared &&
            Manifest.permission.WRITE_CALENDAR in declared
    }

    private suspend fun requestPermission(): ByteArray {
        if (granted()) return encode(GRANTED)

        if (!declaresPermissions()) throw PluginException(PluginException.INVALID_ARGUMENT)

        val host = permissionHost ?: throw PluginException(PluginException.UNAVAILABLE)
        val result =
            host.requestNativePermissions(
                arrayOf(Manifest.permission.READ_CALENDAR, Manifest.permission.WRITE_CALENDAR),
            )
        if (result.size != 2) throw PluginException(PluginException.NATIVE)
        if (closed) throw PluginException(PluginException.CANCELLED)

        return encode(if (granted()) GRANTED else DENIED)
    }

    private suspend fun listCalendars(): ByteArray {
        if (!granted()) throw PluginException(PluginException.PERMISSION_DENIED)

        currentCoroutineContext().ensureActive()
        val calendars =
            try {
                withContext(Dispatchers.IO) { readCalendars() }
            } catch (error: SecurityException) {
                throw PluginException(PluginException.PERMISSION_DENIED, error)
            }

        if (closed) throw PluginException(PluginException.CANCELLED)

        return CalendarRecord.encodeList(calendars)
    }

    private suspend fun createEvent(request: CreateRequest): ByteArray {
        if (!granted()) throw PluginException(PluginException.PERMISSION_DENIED)

        currentCoroutineContext().ensureActive()
        val id =
            try {
                withContext(Dispatchers.IO) { insertEvent(request) }
            } catch (error: SecurityException) {
                throw PluginException(PluginException.PERMISSION_DENIED, error)
            }

        if (closed) throw PluginException(PluginException.CANCELLED)

        return CalendarEventRecord.encodeEventId(id)
    }

    private suspend fun getEvent(id: String): ByteArray {
        if (!granted()) throw PluginException(PluginException.PERMISSION_DENIED)

        currentCoroutineContext().ensureActive()
        val event =
            try {
                withContext(Dispatchers.IO) { readEvent(id) }
            } catch (error: SecurityException) {
                throw PluginException(PluginException.PERMISSION_DENIED, error)
            }

        if (closed) throw PluginException(PluginException.CANCELLED)

        return event.encode()
    }

    private suspend fun listEvents(query: ListQuery): ByteArray {
        if (!granted()) throw PluginException(PluginException.PERMISSION_DENIED)

        currentCoroutineContext().ensureActive()
        val events =
            try {
                withContext(Dispatchers.IO) { readEvents(query) }
            } catch (error: SecurityException) {
                throw PluginException(PluginException.PERMISSION_DENIED, error)
            }

        if (closed) throw PluginException(PluginException.CANCELLED)

        return CalendarEventRecord.encodeList(events)
    }

    private fun readCalendars(): List<CalendarRecord> {
        val calendars = ArrayList<CalendarRecord>(CalendarRecord.MAX_CALENDARS)
        context.contentResolver
            .query(
                CalendarContract.Calendars.CONTENT_URI,
                arrayOf(
                    CalendarContract.Calendars._ID,
                    CalendarContract.Calendars.CALENDAR_DISPLAY_NAME,
                    CalendarContract.Calendars.CALENDAR_ACCESS_LEVEL,
                ),
                null,
                null,
                "${CalendarContract.Calendars.CALENDAR_DISPLAY_NAME} ASC",
            )?.use { cursor ->
                while (cursor.moveToNext() && calendars.size < CalendarRecord.MAX_CALENDARS) {
                    val id = cursor.string(CalendarContract.Calendars._ID) ?: continue
                    val title =
                        cursor
                            .string(CalendarContract.Calendars.CALENDAR_DISPLAY_NAME)
                            ?.ifBlank { null }
                            ?: id
                    val access =
                        cursor.getInt(
                            cursor.getColumnIndexOrThrow(CalendarContract.Calendars.CALENDAR_ACCESS_LEVEL),
                        )
                    val allows =
                        access >= CalendarContract.Calendars.CAL_ACCESS_CONTRIBUTOR
                    calendars.add(CalendarRecord.create(id, title, allows))
                }
            }

        return calendars
    }

    private fun insertEvent(request: CreateRequest): String {
        val calendarId =
            request.calendarId.toLongOrNull()
                ?: throw PluginException(PluginException.INVALID_ARGUMENT)
        if (!calendarAllowsModifications(calendarId)) {
            throw PluginException(PluginException.PERMISSION_DENIED)
        }

        val values =
            ContentValues().apply {
                put(CalendarContract.Events.CALENDAR_ID, calendarId)
                put(CalendarContract.Events.TITLE, request.title)
                put(CalendarContract.Events.DTSTART, request.startMs)
                put(CalendarContract.Events.DTEND, request.endMs)
                put(CalendarContract.Events.EVENT_TIMEZONE, TimeZone.getDefault().id)
                request.notes?.let { put(CalendarContract.Events.DESCRIPTION, it) }
                request.location?.let { put(CalendarContract.Events.EVENT_LOCATION, it) }
            }
        val uri =
            context.contentResolver.insert(CalendarContract.Events.CONTENT_URI, values)
                ?: throw PluginException(PluginException.NATIVE)
        val id = ContentUris.parseId(uri)
        if (id <= 0L) throw PluginException(PluginException.NATIVE)

        return id.toString()
    }

    private fun calendarAllowsModifications(calendarId: Long): Boolean {
        context.contentResolver
            .query(
                ContentUris.withAppendedId(CalendarContract.Calendars.CONTENT_URI, calendarId),
                arrayOf(CalendarContract.Calendars.CALENDAR_ACCESS_LEVEL),
                null,
                null,
                null,
            )?.use { cursor ->
                if (!cursor.moveToFirst()) return false

                val access =
                    cursor.getInt(
                        cursor.getColumnIndexOrThrow(CalendarContract.Calendars.CALENDAR_ACCESS_LEVEL),
                    )

                return access >= CalendarContract.Calendars.CAL_ACCESS_CONTRIBUTOR
            }

        return false
    }

    private fun readEvent(id: String): CalendarEventRecord {
        val eventId = id.toLongOrNull() ?: throw PluginException(PluginException.INVALID_ARGUMENT)
        context.contentResolver
            .query(
                ContentUris.withAppendedId(CalendarContract.Events.CONTENT_URI, eventId),
                EVENT_PROJECTION,
                null,
                null,
                null,
            )?.use { cursor ->
                if (!cursor.moveToFirst()) throw PluginException(PluginException.INVALID_ARGUMENT)

                return eventFromCursor(cursor)
            } ?: throw PluginException(PluginException.NATIVE)
    }

    private suspend fun readEvents(query: ListQuery): List<CalendarEventRecord> {
        val events = ArrayList<CalendarEventRecord>(query.limit.coerceAtMost(CalendarEventRecord.MAX_EVENTS))
        val selection =
            "(${CalendarContract.Events.DTEND} >= ?) AND (${CalendarContract.Events.DTSTART} <= ?) AND " +
                "(${CalendarContract.Events.DELETED} != 1)"
        context.contentResolver
            .query(
                CalendarContract.Events.CONTENT_URI,
                EVENT_PROJECTION,
                selection,
                arrayOf(query.startMs.toString(), query.endMs.toString()),
                "${CalendarContract.Events.DTSTART} ASC",
            )?.use { cursor ->
                while (cursor.moveToNext() && events.size < query.limit) {
                    currentCoroutineContext().ensureActive()
                    events.add(eventFromCursor(cursor))
                }
            }

        return events
    }

    private fun eventFromCursor(cursor: Cursor): CalendarEventRecord {
        val id =
            cursor.string(CalendarContract.Events._ID)
                ?: throw PluginException(PluginException.NATIVE)
        val calendarId =
            cursor.string(CalendarContract.Events.CALENDAR_ID)
                ?: throw PluginException(PluginException.NATIVE)
        val title =
            cursor.string(CalendarContract.Events.TITLE)?.ifBlank { null } ?: id
        val start = cursor.getLong(cursor.getColumnIndexOrThrow(CalendarContract.Events.DTSTART))
        val endIndex = cursor.getColumnIndexOrThrow(CalendarContract.Events.DTEND)
        val end =
            if (cursor.isNull(endIndex)) {
                start
            } else {
                cursor.getLong(endIndex).coerceAtLeast(start)
            }
        val notes = cursor.string(CalendarContract.Events.DESCRIPTION)?.ifBlank { null }
        val location = cursor.string(CalendarContract.Events.EVENT_LOCATION)?.ifBlank { null }

        return CalendarEventRecord.create(id, calendarId, title, start, end, notes, location)
    }

    private fun Cursor.string(column: String): String? {
        val index = getColumnIndex(column)
        if (index < 0 || isNull(index)) return null

        return getString(index)
    }

    private fun encode(status: Int): ByteArray =
        ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN).putInt(status).array()

    private companion object {
        const val STATUS = 0
        const val REQUEST_PERMISSION = 1
        const val LIST_CALENDARS = 2
        const val CREATE_EVENT = 3
        const val GET_EVENT = 4
        const val LIST_EVENTS = 5
        const val PROMPT = 1
        const val DENIED = 2
        const val GRANTED = 3

        val EVENT_PROJECTION =
            arrayOf(
                CalendarContract.Events._ID,
                CalendarContract.Events.CALENDAR_ID,
                CalendarContract.Events.TITLE,
                CalendarContract.Events.DTSTART,
                CalendarContract.Events.DTEND,
                CalendarContract.Events.DESCRIPTION,
                CalendarContract.Events.EVENT_LOCATION,
            )
    }
}
