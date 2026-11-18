package dev.leptoskit.plugins.appusage

import android.app.usage.UsageEvents
import dev.leptoskit.bridge.PluginException
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** The length that marks an absent optional string, as the wire's u32 maximum. */
internal const val ABSENT = -1

/** Most events one query may return; the guest matches this cap. */
internal const val MAX_EVENTS = 1024

/** Most sessions one query may return; the guest matches this cap. */
internal const val MAX_SESSIONS = 4096

/** Longest accepted package name and app label, in bytes. */
internal const val MAX_PACKAGE_BYTES = 256
internal const val MAX_LABEL_BYTES = 256

/** One typed event the wire carries, already in the plugin's vocabulary. */
internal data class TypedEvent(
    val kind: Int,
    val timestampMs: Long,
    val packageName: String?,
) {
    init {
        if (carriesPackage(kind) != (packageName != null)) {
            throw IllegalArgumentException("activity events carry a package; others do not")
        }

        if (packageName != null && packageName.toByteArray().size > MAX_PACKAGE_BYTES) {
            throw IllegalArgumentException("package exceeds the wire bound")
        }
    }
}

/** One app's continuous stretch in the foreground, clipped to the range. */
internal data class TypedSession(
    val packageName: String,
    val label: String,
    val startMs: Long,
    val endMs: Long,
)

/** A range and event cap a query carries. */
internal data class UsageQuery(
    val sinceMs: Long,
    val untilMs: Long,
    val max: Int,
) {
    companion object {
        /** A query is a range, or a range plus one u32 cap. */
        const val RANGE_BYTES = 16
        const val EVENT_BYTES = 20

        /** Rejects bounds the Rust crate would refuse. */
        fun decode(
            payload: ByteBuffer,
            hasMax: Boolean,
        ): UsageQuery {
            val expected = if (hasMax) EVENT_BYTES else RANGE_BYTES

            if (payload.remaining() != expected) throw PluginException(PluginException.INVALID_ARGUMENT)

            val input = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
            val since = input.long
            val until = input.long

            if (since < 0 || until < since) throw PluginException(PluginException.INVALID_ARGUMENT)

            if (!hasMax) return UsageQuery(since, until, 0)

            val max = input.int

            if (max !in 1..MAX_EVENTS) throw PluginException(PluginException.INVALID_ARGUMENT)

            return UsageQuery(since, until, max)
        }
    }
}

/** The kind codes the wire carries, in Rust order. */
internal const val KIND_ACTIVITY_RESUMED = 0
internal const val KIND_ACTIVITY_PAUSED = 1
internal const val KIND_SCREEN_INTERACTIVE = 2
internal const val KIND_SCREEN_NON_INTERACTIVE = 3
internal const val KIND_KEYGUARD_SHOWN = 4
internal const val KIND_KEYGUARD_HIDDEN = 5
internal const val KIND_DEVICE_STARTUP = 6
internal const val KIND_DEVICE_SHUTDOWN = 7

/** Only the activity kinds name one app. */
internal fun carriesPackage(kind: Int): Boolean = kind == KIND_ACTIVITY_RESUMED || kind == KIND_ACTIVITY_PAUSED

/** Maps one system event type to the wire's vocabulary, or null to ignore it. */
internal fun typedKind(eventType: Int): Int? =
    when (eventType) {
        UsageEvents.Event.ACTIVITY_RESUMED -> KIND_ACTIVITY_RESUMED
        UsageEvents.Event.ACTIVITY_PAUSED -> KIND_ACTIVITY_PAUSED
        UsageEvents.Event.SCREEN_INTERACTIVE -> KIND_SCREEN_INTERACTIVE
        UsageEvents.Event.SCREEN_NON_INTERACTIVE -> KIND_SCREEN_NON_INTERACTIVE
        UsageEvents.Event.KEYGUARD_SHOWN -> KIND_KEYGUARD_SHOWN
        UsageEvents.Event.KEYGUARD_HIDDEN -> KIND_KEYGUARD_HIDDEN
        UsageEvents.Event.DEVICE_STARTUP -> KIND_DEVICE_STARTUP
        UsageEvents.Event.DEVICE_SHUTDOWN -> KIND_DEVICE_SHUTDOWN
        else -> null
    }

/** Reads one system event batch into the wire's vocabulary, oldest first. */
internal fun typedEvents(manager: UsageEvents): List<TypedEvent> {
    val events = mutableListOf<TypedEvent>()
    val holder = UsageEvents.Event()

    while (manager.hasNextEvent() && manager.getNextEvent(holder)) {
        val kind = typedKind(holder.eventType) ?: continue
        val packageName = if (carriesPackage(kind)) holder.packageName ?: continue else null

        // A package name past the wire bound cannot be a real one; skip it
        // rather than fail the whole query.
        if (packageName != null && packageName.toByteArray().size > MAX_PACKAGE_BYTES) continue

        if (holder.timeStamp < 0) continue

        events.add(TypedEvent(kind, holder.timeStamp, packageName))
    }

    return events
}

/**
 * Builds per-app foreground sessions from the typed events, oldest first.
 * A resume closes whichever app was open before opening the new one, and a
 * pause ends only the app it names. When the first activity event is a
 * pause, that app was already in the foreground at the range's start, so
 * its session starts at `sinceMs`. A session still open at the end closes
 * at `endMs`, which the caller sets to the earlier of the range's end and
 * now, so an open session never runs into the future. Every edge clips to
 * the range, and empty sessions are dropped.
 */
internal fun buildSessions(
    events: List<TypedEvent>,
    sinceMs: Long,
    endMs: Long,
    labelFor: (String) -> String,
): List<TypedSession> {
    val sessions = mutableListOf<TypedSession>()
    var open: Pair<String, Long>? = null
    var sawActivity = false

    fun close(
        packageName: String,
        started: Long,
        ended: Long,
    ) {
        val start = started.coerceIn(sinceMs, endMs)
        val end = ended.coerceIn(start, endMs)

        if (end > start) sessions.add(TypedSession(packageName, labelFor(packageName), start, end))
    }

    for (event in events) {
        val packageName = event.packageName ?: continue

        when (event.kind) {
            KIND_ACTIVITY_RESUMED -> {
                open?.let { (current, started) -> close(current, started, event.timestampMs) }
                open = packageName to event.timestampMs
            }

            KIND_ACTIVITY_PAUSED -> {
                val current = open

                if (current?.first == packageName) {
                    close(packageName, current.second, event.timestampMs)
                    open = null
                } else if (!sawActivity) {
                    close(packageName, sinceMs, event.timestampMs)
                }
            }
        }

        sawActivity = true
    }

    open?.let { (packageName, started) -> close(packageName, started, endMs) }

    return sessions
}

/**
 * The range's screen-on time and unlock count, from the same events. When
 * the first screen event turns the screen off, it was on at the range's
 * start; a screen still on at the end counts up to `endMs`.
 */
internal fun screenOnAndUnlocks(
    events: List<TypedEvent>,
    sinceMs: Long,
    endMs: Long,
): Pair<Long, Int> {
    var screenOn = 0L
    var interactiveSince: Long? = null
    var sawScreen = false

    fun add(
        from: Long,
        to: Long,
    ) {
        val start = from.coerceIn(sinceMs, endMs)
        screenOn += to.coerceIn(start, endMs) - start
    }

    for (event in events) {
        when (event.kind) {
            KIND_SCREEN_INTERACTIVE -> {
                if (interactiveSince == null) interactiveSince = event.timestampMs
                sawScreen = true
            }

            KIND_SCREEN_NON_INTERACTIVE -> {
                val since = interactiveSince ?: if (sawScreen) null else sinceMs
                since?.let { add(it, event.timestampMs) }
                interactiveSince = null
                sawScreen = true
            }
        }
    }

    interactiveSince?.let { add(it, endMs) }

    return screenOn to events.count { it.kind == KIND_KEYGUARD_HIDDEN }
}

/** Shortens a label to the wire bound without splitting a character. */
internal fun boundedLabel(label: String): String {
    if (label.toByteArray().size <= MAX_LABEL_BYTES) return label

    var end = label.length

    while (end > 0 && label.substring(0, end).toByteArray().size > MAX_LABEL_BYTES) {
        end = label.offsetByCodePoints(end, -1)
    }

    return label.substring(0, end)
}

/** Per-app foreground totals from clipped sessions, sorted by package. */
internal fun appTotals(sessions: List<TypedSession>): List<Pair<String, Long>> =
    sessions
        .groupBy({ it.packageName }) { it.endMs - it.startMs }
        .map { (packageName, spans) -> packageName to spans.sum() }
        .sortedBy { it.first }

/** Frames an event batch: one u32 count, then the events. */
internal fun encodeEventBatch(events: List<TypedEvent>): ByteArray {
    val out = WireBuffer()
    out.int(events.size)

    for (event in events) {
        out.int(event.kind)
        out.long(event.timestampMs)
        out.text(event.packageName)
    }

    return out.finish()
}

/** Frames a session batch: one u32 count, then the sessions. */
internal fun encodeSessionBatch(sessions: List<TypedSession>): ByteArray {
    val out = WireBuffer()
    out.int(sessions.size)

    for (session in sessions) {
        out.text(session.packageName)
        out.text(session.label)
        out.long(session.startMs)
        out.long(session.endMs)
    }

    return out.finish()
}

/** Frames a summary: screen-on time, unlocks, then per-app totals. */
internal fun encodeSummary(
    screenOnMs: Long,
    unlocks: Int,
    apps: List<Pair<String, Long>>,
): ByteArray {
    val out = WireBuffer()
    out.long(screenOnMs)
    out.int(unlocks)
    out.int(apps.size)

    for ((packageName, total) in apps) {
        out.text(packageName)
        out.long(total)
    }

    return out.finish()
}

/** Little-endian field appender; strings carry a u32 byte length. */
private class WireBuffer {
    private val bytes = ByteArrayOutputStream()

    fun int(value: Int) {
        bytes.write(
            byteArrayOf(
                (value and 0xFF).toByte(),
                ((value ushr 8) and 0xFF).toByte(),
                ((value ushr 16) and 0xFF).toByte(),
                ((value ushr 24) and 0xFF).toByte(),
            ),
        )
    }

    fun long(value: Long) {
        int(value.toInt())
        int((value ushr 32).toInt())
    }

    fun text(value: String?) {
        if (value == null) {
            int(ABSENT)

            return
        }

        val utf8 = value.toByteArray()
        require(utf8.size != ABSENT) { "absent length is reserved" }
        int(utf8.size)
        bytes.write(utf8)
    }

    fun finish(): ByteArray = bytes.toByteArray()
}
