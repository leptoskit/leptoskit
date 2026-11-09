package dev.leptoskit.plugins.androidalarmmanager

import dev.leptoskit.bridge.PluginException
import java.io.IOException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.file.Files

// Golden bytes shared with the Rust tests in `src/tests.rs`.
internal val ONE_SHOT_REQUEST =
    bytes(
        7,
        0,
        0,
        0,
        0,
        3,
        0,
        0,
        0,
        0xdc,
        5,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        2,
        0,
        0,
        0,
        'h'.code,
        'i'.code,
    )
internal val PERIODIC_REQUEST =
    bytes(
        0xff,
        0xff,
        0xff,
        0xff,
        2,
        48,
        0,
        0,
        0,
        0xff,
        0xff,
        0xff,
        0xff,
        0xff,
        0xff,
        0xff,
        0xff,
        0x60,
        0xea,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
    )
internal val FIRED =
    bytes(
        3,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        7,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0xe8,
        3,
        0,
        0,
        0,
        0,
        0,
        0,
        0xe2,
        4,
        0,
        0,
        0,
        0,
        0,
        0,
        2,
        0,
        0,
        0,
        'h'.code,
        'i'.code,
    )
internal val STATUS_REPLY = bytes(35, 0, 0, 0, 1, 0, 2, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0)
internal val SCHEDULED_REPLY = bytes(0, 0x00, 0x68, 0xe5, 0xcf, 0x8b, 0x01, 0x00, 0x00)

internal fun bytes(vararg values: Int) = ByteArray(values.size) { values[it].toByte() }

private fun buffer(bytes: ByteArray): ByteBuffer = ByteBuffer.wrap(bytes)

private fun expectInvalid(block: () -> Unit) {
    try {
        block()
    } catch (error: PluginException) {
        check(error.code == PluginException.INVALID_ARGUMENT) { "Expected InvalidArgument, got ${error.code}" }

        return
    }
    error("Expected InvalidArgument")
}

private fun record(
    sequence: Long,
    id: Int = 7,
) = FiredRecord(sequence, id, 0, 1000, 1250, "hi".toByteArray())

private fun alarm(
    id: Int,
    interval: Long = 0,
    next: Long = 1_000,
    flags: Int = 0,
) = StoredAlarm(id, interval, next, flags, TIMING_INEXACT, byteArrayOf(id.toByte()))

private fun ByteArray.with(
    offset: Int,
    value: ByteArray,
) = copyOf().also { value.copyInto(it, offset) }

private fun long(value: Long) =
    ByteBuffer
        .allocate(8)
        .order(ByteOrder.LITTLE_ENDIAN)
        .putLong(value)
        .array()

fun main() {
    goldenBytes()
    malformedRequests()
    periodicTiming()
    timingChoice()
    firingAndTheQueue()
    persistence()
    println("Alarm wire, store and timing checks passed")
}

private fun goldenBytes() {
    val oneShot = decodeRequest(buffer(ONE_SHOT_REQUEST))
    check(oneShot.id == 7 && oneShot.kind == KIND_AFTER && oneShot.flags == EXACT or WAKEUP)
    check(oneShot.time == 1500L && oneShot.interval == 0L && oneShot.params.contentEquals("hi".toByteArray()))
    check(oneShot.firstFire(10_000) == 11_500L)
    val periodic = decodeRequest(buffer(PERIODIC_REQUEST))
    check(periodic.id == -1 && periodic.kind == KIND_PERIODIC && periodic.flags == RESCHEDULE_ON_REBOOT or HEADLESS)
    check(periodic.interval == MIN_PERIOD_MS && periodic.params.isEmpty())
    check(periodic.firstFire(10_000) == 10_000 + MIN_PERIOD_MS)
    check(decodeRequest(buffer(ONE_SHOT_REQUEST.with(4, bytes(1)))).firstFire(99) == 1500L)

    check(encodeFired(record(3)).contentEquals(FIRED))
    check(encodeStatus(35, exact = true, headless = false, pending = 2, dropped = 5).contentEquals(STATUS_REPLY))
    check(encodeScheduled(TIMING_EXACT, 1_700_000_000_000).contentEquals(SCHEDULED_REPLY))
    check(encodeLaunch(null).contentEquals(bytes(0)))
    check(encodeLaunch(record(3)).contentEquals(bytes(1) + FIRED))
    check(encodePage(emptyList()).contentEquals(bytes(0, 0, 0, 0)))
    check(encodePage(listOf(record(3))).contentEquals(bytes(1, 0, 0, 0) + FIRED))
    check(encodeFlag(true).contentEquals(bytes(1)) && encodeFlag(false).contentEquals(bytes(0)))
    val list = encodeList(listOf(StoredAlarm(1, 0, 5, EXACT or WAKEUP, TIMING_EXACT, byteArrayOf())))
    check(list.contentEquals(bytes(1, 0, 0, 0, 1, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0) + ByteArray(8) + bytes(3, 0, 0, 0, 0, 0, 0, 0, 0)))
    val two =
        encodeList(
            listOf(
                StoredAlarm(1, 0, 5, EXACT or WAKEUP, TIMING_EXACT, "again".toByteArray()),
                StoredAlarm(2, 0, 9, 0, TIMING_INEXACT, byteArrayOf()),
            ),
        )
    check(alarmIds(two) == listOf(1, 2))

    check(decodeCancel(buffer(bytes(0xfe, 0xff, 0xff, 0xff))) == -2)
    check(decodeNext(buffer(long(Long.MAX_VALUE))) == Long.MAX_VALUE)
    check(decodeDrain(buffer(bytes(3, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0))) == 3L to 2)
    check(decodeFinish(buffer(long(5))) == 5L)
}

private fun malformedRequests() {
    val tooLong = ONE_SHOT_REQUEST + bytes(0)
    val params = ONE_SHOT_REQUEST.copyOf(25) + bytes(1, 4, 0, 0) + ByteArray(MAX_PARAMS_BYTES + 1)

    for (payload in listOf(
        byteArrayOf(),
        ONE_SHOT_REQUEST.copyOf(ONE_SHOT_REQUEST.size - 1),
        tooLong,
        ONE_SHOT_REQUEST.with(4, bytes(3)),
        ONE_SHOT_REQUEST.with(5, bytes(0x80)),
        ONE_SHOT_REQUEST.with(5, bytes(64)),
        ONE_SHOT_REQUEST.with(9, long(-1)),
        ONE_SHOT_REQUEST.with(9, long(MAX_DELAY_MS + 1)),
        ONE_SHOT_REQUEST.with(17, long(60_000)),
        ONE_SHOT_REQUEST.with(4, bytes(1)).with(9, long(-5)),
        ONE_SHOT_REQUEST.with(4, bytes(1)).with(9, long(MAX_UNIX_MS + 1)),
        ONE_SHOT_REQUEST.with(25, bytes(3, 0, 0, 0)),
        ONE_SHOT_REQUEST.with(25, bytes(0xff, 0xff, 0xff, 0xff)),
        params,
        PERIODIC_REQUEST.with(9, long(-2)),
        PERIODIC_REQUEST.with(17, long(MIN_PERIOD_MS - 1)),
        PERIODIC_REQUEST.with(17, long(0)),
        PERIODIC_REQUEST.with(17, long(MAX_PERIOD_MS + 1)),
    )) {
        expectInvalid { decodeRequest(buffer(payload)) }
    }

    // A fallback with exact timing or an alarm clock to fall back from is fine.
    decodeRequest(buffer(ONE_SHOT_REQUEST.with(5, bytes(EXACT or INEXACT_FALLBACK))))
    decodeRequest(buffer(ONE_SHOT_REQUEST.with(5, bytes(ALARM_CLOCK or INEXACT_FALLBACK))))
    decodeRequest(buffer(PERIODIC_REQUEST.with(9, long(0)).with(17, long(MAX_PERIOD_MS))))

    expectInvalid { decodeCancel(buffer(bytes(0, 0, 0))) }
    expectInvalid { decodeNext(buffer(ByteArray(9))) }
    expectInvalid { decodeDrain(buffer(ByteArray(8) + bytes(MAX_FIRED + 1, 0, 0, 0))) }
    expectInvalid { decodeFinish(buffer(long(0))) }
    expectInvalid { requireEmpty(buffer(bytes(0))) }
}

private fun periodicTiming() {
    check(nextPeriodic(first = 5_000, interval = 60_000, now = 1_000) == 5_000L)
    check(nextPeriodic(first = 5_000, interval = 60_000, now = 5_000) == 65_000L)
    check(nextPeriodic(first = 5_000, interval = 60_000, now = 64_999) == 65_000L)
    check(nextPeriodic(first = 5_000, interval = 60_000, now = 65_000) == 125_000L)
    // A device that slept through many periods skips them rather than firing each one.
    check(nextPeriodic(first = 0, interval = 60_000, now = 3_600_000 + 1) == 3_660_000L)
    check(nextPeriodic(first = MAX_UNIX_MS - 1, interval = MAX_PERIOD_MS, now = MAX_UNIX_MS) == MAX_UNIX_MS)
}

private fun timingChoice() {
    for (allowed in listOf(true, false)) {
        check(chooseTiming(0, allowed) == TIMING_INEXACT)
        check(chooseTiming(WAKEUP or ALLOW_WHILE_IDLE or RESCHEDULE_ON_REBOOT or HEADLESS, allowed) == TIMING_INEXACT)
    }

    check(chooseTiming(EXACT, exactAllowed = true) == TIMING_EXACT)
    check(chooseTiming(ALARM_CLOCK, exactAllowed = true) == TIMING_ALARM_CLOCK)
    check(chooseTiming(ALARM_CLOCK or EXACT, exactAllowed = true) == TIMING_ALARM_CLOCK)
    check(chooseTiming(EXACT or INEXACT_FALLBACK, exactAllowed = true) == TIMING_EXACT)
    check(chooseTiming(EXACT or INEXACT_FALLBACK, exactAllowed = false) == TIMING_INEXACT)
    check(chooseTiming(ALARM_CLOCK or INEXACT_FALLBACK, exactAllowed = false) == TIMING_INEXACT)

    for (flags in listOf(EXACT, ALARM_CLOCK, EXACT or WAKEUP or ALLOW_WHILE_IDLE)) {
        try {
            chooseTiming(flags, exactAllowed = false)
            error("Expected PermissionDenied for $flags")
        } catch (error: PluginException) {
            check(error.code == PluginException.PERMISSION_DENIED)
        }
    }
}

private fun firingAndTheQueue() {
    val state = AlarmState()
    state.put(alarm(1, next = 1_000))
    state.put(alarm(2, interval = 60_000, next = 2_000))

    check(state.fire(9, 5_000) == null)
    val first = checkNotNull(state.fire(1, 1_200))
    check(first.sequence == 1L && first.scheduled == 1_000L && first.fired == 1_200L && first.params.contentEquals(bytes(1)))
    check(state.alarm(1) == null) { "A one-shot alarm leaves the schedule" }
    val second = checkNotNull(state.fire(2, 2_100))
    check(second.sequence == 2L && second.interval == 60_000L && second.scheduled == 2_000L)
    check(state.alarm(2)?.next == 62_000L) { "A periodic alarm moves to its next firing" }
    // A delivery before the due time still moves it one interval on.
    state.fire(2, 0)
    check(state.alarm(2)?.next == 122_000L)
    check(state.removeFired(3))

    check(state.after(0, 64, skip = 0).map { it.sequence } == listOf(1L, 2L))
    check(state.after(0, 1, skip = 0).map { it.sequence } == listOf(1L))
    check(state.after(0, 64, skip = 1).map { it.sequence } == listOf(2L))
    check(!state.acknowledge(0, keep = 0))
    check(state.acknowledge(2, keep = 1))
    check(state.fired.map { it.sequence } == listOf(1L)) { "The launch alarm is kept until it is finished" }
    check(state.removeFired(1) && !state.removeFired(1))

    repeat(MAX_FIRED + 3) { state.fire(2, 10_000L + it) }
    check(state.fired.size == MAX_FIRED && state.dropped == 3L)
    check(state.fired.first().sequence == 7L && state.fired.last().sequence == MAX_FIRED + 6L)

    val full = AlarmState()

    for (id in 0 until MAX_ALARMS) full.put(alarm(id))
    check(!full.fits(MAX_ALARMS) && full.fits(0))
    full.put(alarm(0, next = 9))
    check(full.alarm(0)?.next == 9L && full.alarms.size == MAX_ALARMS)
    check(full.remove(0) && !full.remove(0) && full.fits(MAX_ALARMS))
    check(full.retain { it.id < 10 } && full.alarms.map { it.id } == (1 until 10).toList())
}

private fun persistence() {
    val state = AlarmState()
    state.put(alarm(5, interval = MIN_PERIOD_MS, next = 7_000, flags = RESCHEDULE_ON_REBOOT or EXACT))
    state.put(alarm(-3, next = 9_000, flags = HEADLESS))
    state.fire(-3, 9_500)
    val decoded = AlarmState.decode(state.encode())
    check(decoded.encode().contentEquals(state.encode()))
    check(decoded.alarms.map { it.id } == listOf(5) && decoded.fired.single().id == -3 && decoded.nextSequence == 2L)

    val encoded = state.encode()

    for (damaged in listOf(
        byteArrayOf(),
        encoded.copyOf(encoded.size - 1),
        encoded + bytes(0),
        encoded.with(0, bytes(0)),
        encoded.with(4, bytes(2)),
        // The next sequence must stay ahead of every queued alarm.
        encoded.with(8, long(1)),
    )) {
        try {
            AlarmState.decode(damaged)
            error("Expected a damaged store to be rejected")
        } catch (_: IOException) {
        }
    }

    val directory = Files.createTempDirectory("alarm-store").toFile()
    try {
        val file = AlarmFile(directory.resolve("nested/alarms.bin"))
        check(file.load().alarms.isEmpty())
        file.save(state)
        check(file.load().encode().contentEquals(encoded))
        check(directory.resolve("nested").list()?.toList() == listOf("alarms.bin")) { "No temporary file is left" }
    } finally {
        directory.deleteRecursively()
    }
}

/** The same cursor the instrumented list test uses: params length, then the bytes. */
private fun alarmIds(encoded: ByteArray): List<Int> {
    val list = ByteBuffer.wrap(encoded).order(ByteOrder.LITTLE_ENDIAN)

    return List(list.int) {
        val id = list.int
        list.position(list.position() + 8 + 8 + 4 + 1)
        val params = list.int
        list.position(list.position() + params)
        id
    }
}
