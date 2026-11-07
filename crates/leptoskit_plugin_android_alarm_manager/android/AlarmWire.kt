package dev.leptoskit.plugins.androidalarmmanager

import dev.leptoskit.bridge.PluginException
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.IOException
import java.nio.BufferUnderflowException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.file.Files
import java.nio.file.StandardCopyOption

// The wire format, the alarm store and the timing rules. Nothing here touches
// Android, so `kotlinc` can run `AlarmWireTest.kt` against it on a desktop JVM.

internal const val STATUS = 0
internal const val SCHEDULE = 1
internal const val CANCEL = 2
internal const val LIST = 3
internal const val REQUEST_EXACT_PERMISSION = 4
internal const val NEXT = 5
internal const val DRAIN = 6
internal const val LAUNCH = 7
internal const val FINISH = 8

internal const val MAX_PARAMS_BYTES = 1024
internal const val MAX_ALARMS = 128
internal const val MAX_FIRED = 64
internal const val MIN_PERIOD_MS = 60_000L
internal const val MAX_PERIOD_MS = 366L * 24 * 60 * 60 * 1000
internal const val MAX_DELAY_MS = 3660L * 24 * 60 * 60 * 1000
internal const val MAX_UNIX_MS = 253_402_300_799_999L
internal const val HEADLESS_BUDGET_MS = 8_000L

internal const val KIND_AFTER = 0
internal const val KIND_AT = 1
internal const val KIND_PERIODIC = 2
private const val FROM_NOW = -1L

internal const val EXACT = 1
internal const val WAKEUP = 1 shl 1
internal const val ALLOW_WHILE_IDLE = 1 shl 2
internal const val ALARM_CLOCK = 1 shl 3
internal const val RESCHEDULE_ON_REBOOT = 1 shl 4
internal const val HEADLESS = 1 shl 5
internal const val INEXACT_FALLBACK = 1 shl 6
private const val KNOWN_FLAGS = (1 shl 7) - 1

internal const val TIMING_EXACT = 0
internal const val TIMING_INEXACT = 1
internal const val TIMING_ALARM_CLOCK = 2

/** A decoded schedule request; times are still relative to the request. */
internal class AlarmRequest(
    val id: Int,
    val kind: Int,
    val flags: Int,
    val time: Long,
    val interval: Long,
    val params: ByteArray,
) {
    /** The first firing, counted from [now]. */
    fun firstFire(now: Long): Long =
        when (kind) {
            KIND_AFTER -> now + time
            KIND_AT -> time
            else -> if (time == FROM_NOW) now + interval else time
        }.coerceIn(0, MAX_UNIX_MS)
}

/** An alarm the store holds as scheduled. [interval] is 0 for one-shot. */
internal class StoredAlarm(
    val id: Int,
    val interval: Long,
    val next: Long,
    val flags: Int,
    val timing: Int,
    val params: ByteArray,
) {
    fun has(flag: Int) = flags and flag != 0

    fun with(
        next: Long = this.next,
        timing: Int = this.timing,
    ) = StoredAlarm(id, interval, next, flags, timing, params)
}

/** An alarm that fired and waits for a guest to acknowledge it. */
internal class FiredRecord(
    val sequence: Long,
    val id: Int,
    val interval: Long,
    val scheduled: Long,
    val fired: Long,
    val params: ByteArray,
)

/** The earliest `first + n * interval` after [now], without overflowing. */
internal fun nextPeriodic(
    first: Long,
    interval: Long,
    now: Long,
): Long {
    require(interval > 0)

    if (first > now) return first
    val steps = (now - first) / interval + 1

    return (first + steps * interval).coerceAtMost(MAX_UNIX_MS)
}

/**
 * How to hand an alarm with [flags] to `AlarmManager` when exact alarms are
 * [exactAllowed]: an alarm clock or exact alarm when allowed, inexact when
 * asked for or as the fallback, and PermissionDenied otherwise.
 */
internal fun chooseTiming(
    flags: Int,
    exactAllowed: Boolean,
): Int =
    when {
        flags and ALARM_CLOCK != 0 && exactAllowed -> TIMING_ALARM_CLOCK
        flags and EXACT != 0 && exactAllowed -> TIMING_EXACT
        flags and (EXACT or ALARM_CLOCK) == 0 || flags and INEXACT_FALLBACK != 0 -> TIMING_INEXACT
        else -> throw PluginException(PluginException.PERMISSION_DENIED)
    }

/** Reads one request strictly, with the bounds the Rust encoder enforces. */
internal fun decodeRequest(payload: ByteBuffer): AlarmRequest {
    val input = Input(payload)
    val id = input.int()
    val kind = input.byte()
    val flags = input.int()
    val time = input.long()
    val interval = input.long()
    val params = input.bytes(MAX_PARAMS_BYTES)
    input.finish()
    validFlags(flags)
    val valid =
        when (kind) {
            KIND_AFTER -> interval == 0L && time in 0..MAX_DELAY_MS
            KIND_AT -> interval == 0L && time in 0..MAX_UNIX_MS
            KIND_PERIODIC -> interval in MIN_PERIOD_MS..MAX_PERIOD_MS && (time == FROM_NOW || time in 0..MAX_UNIX_MS)
            else -> false
        }

    if (!valid) invalid()

    return AlarmRequest(id, kind, flags, time, interval, params)
}

private fun validFlags(flags: Int) {
    if (flags and KNOWN_FLAGS.inv() != 0) invalid()

    if (flags and INEXACT_FALLBACK != 0 && flags and (EXACT or ALARM_CLOCK) == 0) invalid()
}

internal fun encodeScheduled(
    timing: Int,
    first: Long,
): ByteArray = Output().apply { byte(timing).long(first) }.finish()

internal fun encodeList(alarms: List<StoredAlarm>): ByteArray {
    val output = Output().int(alarms.size)

    for (alarm in alarms) {
        output
            .int(alarm.id)
            .long(alarm.next)
            .long(alarm.interval)
            .int(alarm.flags)
            .byte(alarm.timing)
            .bytes(alarm.params)
    }

    return output.finish()
}

internal fun encodeFired(record: FiredRecord): ByteArray = Output().fired(record).finish()

internal fun encodePage(records: List<FiredRecord>): ByteArray {
    val output = Output().int(records.size)

    for (record in records) output.fired(record)

    return output.finish()
}

internal fun encodeLaunch(record: FiredRecord?): ByteArray = if (record == null) byteArrayOf(0) else Output().byte(1).fired(record).finish()

internal fun encodeStatus(
    sdk: Int,
    exact: Boolean,
    headless: Boolean,
    pending: Int,
    dropped: Long,
): ByteArray =
    Output()
        .int(sdk)
        .flag(exact)
        .flag(headless)
        .int(pending)
        .long(dropped)
        .finish()

internal fun encodeFlag(value: Boolean): ByteArray = byteArrayOf(if (value) 1 else 0)

internal fun decodeCancel(payload: ByteBuffer): Int = Input(payload).run { int().also { finish() } }

internal fun decodeNext(payload: ByteBuffer): Long = Input(payload).run { long().also { finish() } }

internal fun decodeDrain(payload: ByteBuffer): Pair<Long, Int> {
    val input = Input(payload)
    val cursor = input.long()
    val max = input.int()
    input.finish()

    if (max !in 0..MAX_FIRED) invalid()

    return cursor to max
}

internal fun decodeFinish(payload: ByteBuffer): Long {
    val sequence = Input(payload).run { long().also { finish() } }

    if (sequence == 0L) invalid()

    return sequence
}

internal fun requireEmpty(payload: ByteBuffer) {
    if (payload.hasRemaining()) invalid()
}

private fun invalid(): Nothing = throw PluginException(PluginException.INVALID_ARGUMENT)

/**
 * Scheduled alarms and the fired-alarm queue. Sequences start at 1 and are
 * never reused; the queue keeps the newest [MAX_FIRED] and counts the rest
 * in [dropped]. Not thread-safe: the plugin confines it to the main thread.
 */
internal class AlarmState(
    alarms: Collection<StoredAlarm> = emptyList(),
    fired: Collection<FiredRecord> = emptyList(),
    nextSequence: Long = 1,
    dropped: Long = 0,
) {
    private val alarmsById = LinkedHashMap<Int, StoredAlarm>().apply { alarms.forEach { put(it.id, it) } }
    private val queue = ArrayDeque(fired)
    var nextSequence = nextSequence
        private set
    var dropped = dropped
        private set

    val alarms: List<StoredAlarm> get() = alarmsById.values.sortedBy { it.id }
    val fired: List<FiredRecord> get() = queue.toList()

    fun copy() = AlarmState(alarmsById.values, queue, nextSequence, dropped)

    fun alarm(id: Int): StoredAlarm? = alarmsById[id]

    /** Whether one more alarm with [id] fits. Replacing an alarm always fits. */
    fun fits(id: Int) = id in alarmsById || alarmsById.size < MAX_ALARMS

    fun put(alarm: StoredAlarm) {
        check(fits(alarm.id)) { "Too many alarms" }
        alarmsById[alarm.id] = alarm
    }

    fun remove(id: Int): Boolean = alarmsById.remove(id) != null

    /** Keeps only the alarms [keep] accepts; returns whether any went. */
    fun retain(keep: (StoredAlarm) -> Boolean): Boolean = alarmsById.values.removeAll { !keep(it) }

    /**
     * Records that alarm [id] fired at [now]: a one-shot alarm leaves the
     * schedule, a periodic one moves to its next firing. Null when the alarm
     * is no longer scheduled, such as a fire racing a cancel.
     */
    fun fire(
        id: Int,
        now: Long,
    ): FiredRecord? {
        val alarm = alarmsById[id] ?: return null
        val record = FiredRecord(nextSequence++, id, alarm.interval, alarm.next, now, alarm.params)

        if (alarm.interval == 0L) {
            alarmsById.remove(id)
        } else {
            // Even a delivery before the due time moves the alarm forward.
            alarmsById[id] = alarm.with(next = nextPeriodic(alarm.next, alarm.interval, maxOf(now, alarm.next)))
        }

        queue.addLast(record)

        while (queue.size > MAX_FIRED) {
            queue.removeFirst()
            dropped++
        }

        return record
    }

    /** Removes fired alarms up to [cursor], except [keep]; returns whether any went. */
    fun acknowledge(
        cursor: Long,
        keep: Long,
    ): Boolean = queue.removeAll { it.sequence <= cursor && it.sequence != keep }

    /** Up to [max] fired alarms after [cursor], oldest first, leaving out [skip]. */
    fun after(
        cursor: Long,
        max: Int,
        skip: Long,
    ): List<FiredRecord> =
        queue
            .asSequence()
            .filter { it.sequence > cursor && it.sequence != skip }
            .take(max)
            .toList()

    fun removeFired(sequence: Long): Boolean = queue.removeAll { it.sequence == sequence }

    fun encode(): ByteArray {
        val output =
            Output()
                .int(MAGIC)
                .int(VERSION)
                .long(nextSequence)
                .long(dropped)
                .int(alarmsById.size)

        for (alarm in alarmsById.values) {
            output
                .int(alarm.id)
                .long(alarm.next)
                .long(alarm.interval)
                .int(alarm.flags)
                .byte(alarm.timing)
                .bytes(alarm.params)
        }

        output.int(queue.size)

        for (record in queue) output.fired(record)

        return output.finish()
    }

    companion object {
        private const val MAGIC = 0x4d414b4c // "LKAM"
        private const val VERSION = 1

        /** Strictly decodes [encode]; a damaged store throws [IOException]. */
        fun decode(bytes: ByteArray): AlarmState =
            try {
                decodeChecked(Input(ByteBuffer.wrap(bytes)))
            } catch (error: PluginException) {
                throw IOException("Damaged alarm store", error)
            }

        private fun decodeChecked(input: Input): AlarmState {
            if (input.int() != MAGIC || input.int() != VERSION) invalid()
            val nextSequence = input.long()
            val dropped = input.long()
            val alarmCount = input.int()

            if (nextSequence < 1 || dropped < 0 || alarmCount !in 0..MAX_ALARMS) invalid()
            val alarms =
                List(alarmCount) {
                    val id = input.int()
                    val next = input.long()
                    val interval = input.long()
                    val flags = input.int()
                    val timing = input.byte()
                    validFlags(flags)

                    if (next !in 0..MAX_UNIX_MS || timing !in TIMING_EXACT..TIMING_ALARM_CLOCK) invalid()

                    if (interval != 0L && interval !in MIN_PERIOD_MS..MAX_PERIOD_MS) invalid()
                    StoredAlarm(id, interval, next, flags, timing, input.bytes(MAX_PARAMS_BYTES))
                }
            val firedCount = input.int()

            if (firedCount !in 0..MAX_FIRED) invalid()
            val fired = List(firedCount) { input.fired() }
            input.finish()

            if (alarms.map { it.id }.toSet().size != alarms.size) invalid()

            if (fired.zipWithNext().any { (a, b) -> a.sequence >= b.sequence }) invalid()

            if (fired.any { it.sequence >= nextSequence }) invalid()

            return AlarmState(alarms, fired, nextSequence, dropped)
        }
    }
}

/** Saves [AlarmState] by writing a sibling file and renaming it over the old one. */
internal class AlarmFile(
    private val file: File,
) {
    fun load(): AlarmState = if (file.isFile) AlarmState.decode(file.readBytes()) else AlarmState()

    fun save(state: AlarmState) {
        val directory = file.parentFile ?: throw IOException("Alarm store has no directory")

        if (!directory.isDirectory && !directory.mkdirs()) throw IOException("Cannot create $directory")
        val temporary = File(directory, "${file.name}.tmp")
        temporary.outputStream().use { stream ->
            stream.write(state.encode())
            stream.fd.sync()
        }
        Files.move(temporary.toPath(), file.toPath(), StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE)
    }
}

private class Input(
    payload: ByteBuffer,
) {
    private val buffer = payload.asReadOnlyBuffer().order(ByteOrder.LITTLE_ENDIAN)

    fun byte(): Int = read { buffer.get().toInt() and 0xff }

    fun int(): Int = read { buffer.int }

    fun long(): Long = read { buffer.long }

    fun bytes(max: Int): ByteArray {
        val length = int()

        if (length !in 0..max || length > buffer.remaining()) invalid()

        return ByteArray(length).also { buffer.get(it) }
    }

    fun fired(): FiredRecord {
        val sequence = long()
        val id = int()
        val interval = long()
        val scheduled = long()
        val fired = long()
        val params = bytes(MAX_PARAMS_BYTES)

        if (sequence < 1 || scheduled !in 0..MAX_UNIX_MS || fired !in 0..MAX_UNIX_MS) invalid()

        if (interval != 0L && interval !in MIN_PERIOD_MS..MAX_PERIOD_MS) invalid()

        return FiredRecord(sequence, id, interval, scheduled, fired, params)
    }

    fun finish() {
        if (buffer.hasRemaining()) invalid()
    }

    private inline fun <T> read(block: () -> T): T =
        try {
            block()
        } catch (_: BufferUnderflowException) {
            invalid()
        }
}

private class Output {
    private val stream = ByteArrayOutputStream()
    private val scratch = ByteBuffer.allocate(8).order(ByteOrder.LITTLE_ENDIAN)

    fun byte(value: Int) = apply { stream.write(value) }

    fun flag(value: Boolean) = byte(if (value) 1 else 0)

    fun int(value: Int) = apply { put(4) { scratch.putInt(value) } }

    fun long(value: Long) = apply { put(8) { scratch.putLong(value) } }

    fun bytes(value: ByteArray) = apply { int(value.size).stream.write(value) }

    fun fired(record: FiredRecord) =
        long(record.sequence)
            .int(record.id)
            .long(record.interval)
            .long(record.scheduled)
            .long(record.fired)
            .bytes(record.params)

    fun finish(): ByteArray = stream.toByteArray()

    private inline fun put(
        size: Int,
        write: () -> Unit,
    ) {
        scratch.clear()
        write()
        stream.write(scratch.array(), 0, size)
    }
}
