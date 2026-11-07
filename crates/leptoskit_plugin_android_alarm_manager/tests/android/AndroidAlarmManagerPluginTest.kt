package dev.leptoskit.plugins.androidalarmmanager

import android.content.Context
import android.content.Intent
import android.os.Build
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.leptoskit.bridge.PluginException
import dev.leptoskit.mobile.NativeHeadlessGuest
import dev.leptoskit.mobile.NativeHeadlessOutcome
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * Schedules real alarms through `AlarmManager`, delivers them to the
 * receivers, and starts the suite's guest headlessly. Each test starts from
 * an empty store.
 */
@RunWith(AndroidJUnit4::class)
class AndroidAlarmManagerPluginTest {
    private val context: Context get() = InstrumentationRegistry.getInstrumentation().targetContext

    @Before
    @After
    fun emptyStore() =
        onMain {
            for (alarm in AlarmRegistry.state(context).alarms) AlarmScheduler.disarm(context, alarm.id)
            AlarmRegistry.storeFile(context).delete()
            AlarmRegistry.reload()
        }

    @Test
    fun statusAndMalformedCalls() =
        onMain {
            withPlugin { plugin ->
                val status = reply(plugin.invoke(STATUS, empty()))
                assertEquals(18, status.remaining())
                assertEquals(Build.VERSION.SDK_INT, status.int)
                assertEquals(if (AlarmScheduler.canScheduleExact(context)) 1 else 0, status.get().toInt())
                assertEquals(1, status.get().toInt()) // The suite ships its guest without code push.
                assertEquals(0, status.int)
                assertEquals(0L, status.long)

                for ((method, payload) in listOf(
                    STATUS to bytes(0),
                    LIST to bytes(0),
                    LAUNCH to bytes(0),
                    SCHEDULE to bytes(),
                    SCHEDULE to ONE_SHOT_REQUEST + bytes(0),
                    CANCEL to bytes(0, 0, 0),
                    NEXT to bytes(),
                    DRAIN to ByteArray(8) + bytes(MAX_FIRED + 1, 0, 0, 0),
                    FINISH to ByteArray(8),
                )) {
                    expectError(PluginException.INVALID_ARGUMENT) { plugin.invoke(method, buffer(payload)) }
                }

                expectError(PluginException.UNAVAILABLE) { plugin.invoke(42, empty()) }
                // A guest started normally has no launch alarm to report or finish.
                assertArrayEquals(bytes(0), plugin.invoke(LAUNCH, empty()))
                expectError(PluginException.UNAVAILABLE) { plugin.invoke(FINISH, buffer(long(1))) }
            }
        }

    @Test
    fun anExactOneShotAlarmFiresIntoTheQueue() =
        runBlocking {
            allowExactAlarms()
            onMain {
                withPlugin { plugin ->
                    val before = System.currentTimeMillis()
                    val options = EXACT or WAKEUP or ALLOW_WHILE_IDLE
                    val scheduled = reply(plugin.invoke(SCHEDULE, request(4242, KIND_AFTER, options, 1_500, params = "fire")))
                    assertEquals(TIMING_EXACT, scheduled.get().toInt())
                    val first = scheduled.long
                    assertTrue(first in before + 1_500..System.currentTimeMillis() + 1_500)
                    assertEquals(listOf(4242), ids(plugin))
                    assertTrue(AlarmScheduler.armed(context, 4242))

                    val delivered = withTimeout(60_000) { plugin.invoke(NEXT, buffer(long(0))) }
                    val record = fired(delivered)
                    assertEquals(4242, record.id)
                    assertEquals(first, record.scheduled)
                    assertTrue(record.fired >= record.scheduled)
                    assertArrayEquals("fire".toByteArray(), record.params)
                    assertEquals(emptyList<Int>(), ids(plugin))

                    // It stays on the device until a later call acknowledges it.
                    AlarmRegistry.reload()
                    assertEquals(listOf(record.sequence), AlarmRegistry.state(context).fired.map { it.sequence })
                    assertEquals(0, reply(plugin.invoke(DRAIN, drain(record.sequence, 0))).int)
                    AlarmRegistry.reload()
                    assertTrue(AlarmRegistry.state(context).fired.isEmpty())
                }
            }
        }

    @Test
    fun cancelForgetsTheAlarm() =
        runBlocking {
            allowExactAlarms()
            onMain {
                withPlugin { plugin ->
                    plugin.invoke(SCHEDULE, request(7, KIND_AT, ALARM_CLOCK, System.currentTimeMillis() + 3_600_000))
                    assertEquals(TIMING_ALARM_CLOCK, AlarmRegistry.state(context).alarm(7)?.timing)
                    assertArrayEquals(bytes(1), plugin.invoke(CANCEL, buffer(int(7))))
                    assertFalse(AlarmScheduler.armed(context, 7))
                    assertArrayEquals(bytes(0), plugin.invoke(CANCEL, buffer(int(7))))
                    assertEquals(emptyList<Int>(), ids(plugin))
                    AlarmRegistry.reload()
                    assertNull(AlarmRegistry.state(context).alarm(7))
                }
            }
        }

    @Test
    fun scheduleReplacesTheSameIdAndBoundsTheStore() =
        onMain {
            withPlugin { plugin ->
                val later = System.currentTimeMillis() + 3_600_000
                plugin.invoke(SCHEDULE, request(1, KIND_AT, 0, later))
                plugin.invoke(SCHEDULE, request(1, KIND_PERIODIC, 0, -1, interval = MIN_PERIOD_MS, params = "again"))
                val alarm = checkNotNull(AlarmRegistry.state(context).alarm(1))
                assertEquals(MIN_PERIOD_MS, alarm.interval)
                assertArrayEquals("again".toByteArray(), alarm.params)
                assertEquals(listOf(1), ids(plugin))

                for (id in 2..MAX_ALARMS) plugin.invoke(SCHEDULE, request(id, KIND_AT, 0, later))
                expectError(PluginException.BUSY) { plugin.invoke(SCHEDULE, request(MAX_ALARMS + 1, KIND_AT, 0, later)) }
                // Replacing an existing alarm still fits.
                plugin.invoke(SCHEDULE, request(MAX_ALARMS, KIND_AT, 0, later + 1))
                assertEquals(MAX_ALARMS, ids(plugin).size)
            }
        }

    @Test
    fun receiversDeliverFiredAlarmsToWaitersAndPeriodicAlarmsMoveOn() =
        onMain {
            withPlugin { plugin ->
                val later = System.currentTimeMillis() + 3_600_000
                plugin.invoke(SCHEDULE, request(77, KIND_AT, 0, later, params = "one"))
                plugin.invoke(SCHEDULE, request(78, KIND_PERIODIC, 0, later, interval = MIN_PERIOD_MS))

                // One waiter at a time; a cancelled waiter loses nothing.
                val cancelled = async(start = CoroutineStart.UNDISPATCHED) { plugin.invoke(NEXT, buffer(long(0))) }
                expectError(PluginException.BUSY) { plugin.invoke(NEXT, buffer(long(0))) }
                cancelled.cancelAndJoin()
                val waiting = async(start = CoroutineStart.UNDISPATCHED) { plugin.invoke(NEXT, buffer(long(0))) }
                assertFalse(waiting.isCompleted)

                // A fire for an alarm that was cancelled meanwhile is ignored.
                fire(999)
                assertFalse(waiting.isCompleted)
                fire(77)
                val first = fired(withTimeout(10_000) { waiting.await() })
                assertEquals(77, first.id)
                assertEquals(later, first.scheduled)
                assertArrayEquals("one".toByteArray(), first.params)
                assertEquals(listOf(78), ids(plugin))

                fire(78)
                val second = fired(plugin.invoke(NEXT, buffer(long(first.sequence))))
                assertEquals(78, second.id)
                assertEquals(MIN_PERIOD_MS, second.interval)
                assertEquals(later + MIN_PERIOD_MS, AlarmRegistry.state(context).alarm(78)?.next)
                assertTrue(AlarmScheduler.armed(context, 78))
                // Asking after the second acknowledged the first.
                assertEquals(listOf(second.sequence), AlarmRegistry.state(context).fired.map { it.sequence })
            }
        }

    @Test
    fun theQueueKeepsTheNewestAlarmsAndCountsTheRest() =
        onMain {
            withPlugin { plugin ->
                plugin.invoke(SCHEDULE, request(5, KIND_PERIODIC, 0, -1, interval = MIN_PERIOD_MS))
                repeat(MAX_FIRED + 6) { fire(5) }
                val status = reply(plugin.invoke(STATUS, empty()))
                status.position(6)
                assertEquals(MAX_FIRED, status.int)
                assertEquals(6L, status.long)

                val page = reply(plugin.invoke(DRAIN, drain(0, MAX_FIRED)))
                assertEquals(MAX_FIRED, page.int)
                val sequences = List(MAX_FIRED) { record(page).sequence }
                assertEquals((7L..MAX_FIRED + 6L).toList(), sequences)
                assertEquals(0, reply(plugin.invoke(DRAIN, drain(sequences.last(), MAX_FIRED))).int)
                assertTrue(AlarmRegistry.state(context).fired.isEmpty())
            }
        }

    @Test
    fun alarmsSurviveProcessDeathAndOnlyRebootAlarmsSurviveARestart() =
        runBlocking {
            allowExactAlarms()
            onMain {
                val later = System.currentTimeMillis() + 3_600_000
                withPlugin { plugin ->
                    plugin.invoke(SCHEDULE, request(1, KIND_AT, RESCHEDULE_ON_REBOOT or EXACT, later))
                    plugin.invoke(SCHEDULE, request(2, KIND_AT, 0, later))
                    plugin.invoke(SCHEDULE, request(3, KIND_PERIODIC, RESCHEDULE_ON_REBOOT, later, interval = MIN_PERIOD_MS))
                }

                // A new process reads the same store.
                AlarmRegistry.reload()
                assertEquals(listOf(1, 2, 3), AlarmRegistry.state(context).alarms.map { it.id })

                // A reboot clears AlarmManager; the restore receiver arms the persistent ones again.
                for (id in 1..3) AlarmScheduler.disarm(context, id)
                AlarmRestoreReceiver().onReceive(context, Intent(Intent.ACTION_BOOT_COMPLETED))
                assertEquals(listOf(1, 3), AlarmRegistry.state(context).alarms.map { it.id })
                assertTrue(AlarmScheduler.armed(context, 1) && AlarmScheduler.armed(context, 3))
                assertEquals(TIMING_EXACT, AlarmRegistry.state(context).alarm(1)?.timing)
                AlarmRegistry.reload()
                assertEquals(listOf(1, 3), AlarmRegistry.state(context).alarms.map { it.id })

                // Granting exact alarms arms every stored alarm again.
                AlarmScheduler.disarm(context, 3)
                AlarmRestoreReceiver().onReceive(
                    context,
                    Intent(android.app.AlarmManager.ACTION_SCHEDULE_EXACT_ALARM_PERMISSION_STATE_CHANGED),
                )
                assertTrue(AlarmScheduler.armed(context, 3))
                // Other broadcasts change nothing.
                AlarmRestoreReceiver().onReceive(context, Intent(Intent.ACTION_TIME_CHANGED))
                assertEquals(listOf(1, 3), AlarmRegistry.state(context).alarms.map { it.id })
            }
        }

    @Test
    fun aHeadlessAlarmStartsTheGuestWhichFinishesIt() =
        onMain {
            // The suite ships its guest without code push.
            assertTrue(NativeHeadlessGuest.available(context))
            AlarmRegistry.schedule(
                context,
                decodeRequest(request(99, KIND_AT, HEADLESS, System.currentTimeMillis() + 3_600_000, params = "headless fixture")),
                System.currentTimeMillis(),
            )
            val (record, flags) = checkNotNull(AlarmRegistry.fire(context, 99, System.currentTimeMillis()))
            assertTrue(flags and HEADLESS != 0)
            val outcome = CompletableDeferred<NativeHeadlessOutcome?>()
            // Another test's Activity may still run in this process; outside tests it blocks the launch.
            AlarmHeadless.start(context, record, budgetMillis = 30_000, ignoreLiveGuests = true) { outcome.complete(it) }
            // A second alarm while one runs never starts a second guest.
            val second = CompletableDeferred<NativeHeadlessOutcome?>()
            AlarmHeadless.start(context, record, budgetMillis = 30_000, ignoreLiveGuests = true) { second.complete(it) }
            assertNull(second.await())

            assertEquals(NativeHeadlessOutcome.FINISHED, withTimeout(60_000) { outcome.await() })
            AlarmRegistry.reload()
            assertTrue(AlarmRegistry.state(context).fired.none { it.sequence == record.sequence })
        }

    @Test
    fun aHeadlessAlarmTheGuestDoesNotFinishStaysQueued() =
        onMain {
            assertTrue(NativeHeadlessGuest.available(context))
            AlarmRegistry.schedule(
                context,
                decodeRequest(request(98, KIND_AT, HEADLESS, System.currentTimeMillis() + 3_600_000, params = "other")),
                System.currentTimeMillis(),
            )
            val (launched, _) = checkNotNull(AlarmRegistry.fire(context, 98, System.currentTimeMillis()))
            val outcome = CompletableDeferred<NativeHeadlessOutcome?>()
            AlarmHeadless.start(context, launched, budgetMillis = 3_000, ignoreLiveGuests = true) { outcome.complete(it) }
            assertEquals(NativeHeadlessOutcome.TIMED_OUT, withTimeout(60_000) { outcome.await() })

            withPlugin { plugin ->
                val page = reply(plugin.invoke(DRAIN, drain(0, MAX_FIRED)))
                assertEquals(1, page.int)
                assertEquals(launched.sequence, record(page).sequence)
            }
        }

    @Test
    fun aRunningGuestPreventsAHeadlessLaunch() =
        onMain {
            withPlugin {
                val record = FiredRecord(1, 1, 0, 0, 0, byteArrayOf())
                val outcome = CompletableDeferred<NativeHeadlessOutcome?>()
                AlarmHeadless.start(context, record) { outcome.complete(it) }
                assertNull(outcome.await())
            }
        }

    private fun fire(id: Int) {
        AlarmReceiver().onReceive(context, Intent(context, AlarmReceiver::class.java).setAction(ACTION_FIRE).putExtra(EXTRA_ID, id))
    }

    private suspend fun ids(plugin: AndroidAlarmManagerPlugin): List<Int> {
        val list = reply(plugin.invoke(LIST, empty()))

        return List(list.int) {
            val id = list.int
            list.position(list.position() + 8 + 8 + 4 + 1)
            list.position(list.position() + list.int)
            id
        }
    }

    private suspend fun allowExactAlarms() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) appops("allow")
    }

    private suspend fun appops(mode: String) =
        withContext(Dispatchers.IO) {
            InstrumentationRegistry
                .getInstrumentation()
                .uiAutomation
                .executeShellCommand("appops set ${context.packageName} SCHEDULE_EXACT_ALARM $mode")
                .close()
            withTimeout(10_000) {
                while (AlarmScheduler.canScheduleExact(context) != (mode == "allow")) kotlinx.coroutines.delay(50)
            }
        }

    private suspend fun withPlugin(block: suspend kotlinx.coroutines.CoroutineScope.(AndroidAlarmManagerPlugin) -> Unit) {
        val plugin = AndroidAlarmManagerPlugin(context)
        try {
            kotlinx.coroutines.coroutineScope { block(plugin) }
        } finally {
            plugin.close()
        }
    }

    private suspend fun expectError(
        code: Int,
        block: suspend () -> Unit,
    ) {
        try {
            block()
            fail("expected PluginException($code)")
        } catch (error: PluginException) {
            assertEquals(code, error.code)
        }
    }
}

private fun <T> onMain(block: suspend kotlinx.coroutines.CoroutineScope.() -> T): T =
    runBlocking {
        withContext(Dispatchers.Main) { block() }
    }

private fun empty(): ByteBuffer = ByteBuffer.allocate(0)

private fun buffer(bytes: ByteArray): ByteBuffer = ByteBuffer.wrap(bytes)

private fun reply(bytes: ByteArray): ByteBuffer = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)

private fun int(value: Int) =
    ByteBuffer
        .allocate(4)
        .order(ByteOrder.LITTLE_ENDIAN)
        .putInt(value)
        .array()

private fun long(value: Long) =
    ByteBuffer
        .allocate(8)
        .order(ByteOrder.LITTLE_ENDIAN)
        .putLong(value)
        .array()

private fun drain(
    cursor: Long,
    max: Int,
): ByteBuffer = buffer(long(cursor) + int(max))

private fun request(
    id: Int,
    kind: Int,
    flags: Int,
    time: Long,
    interval: Long = 0,
    params: String = "",
): ByteBuffer {
    val bytes = params.toByteArray()

    return buffer(int(id) + byteArrayOf(kind.toByte()) + int(flags) + long(time) + long(interval) + int(bytes.size) + bytes)
}

private fun fired(bytes: ByteArray): FiredRecord {
    val input = reply(bytes)
    val record = record(input)
    assertFalse(input.hasRemaining())

    return record
}

private fun record(input: ByteBuffer): FiredRecord {
    val sequence = input.long
    val id = input.int
    val interval = input.long
    val scheduled = input.long
    val fired = input.long
    val params = ByteArray(input.int).also { input.get(it) }
    assertNotNull(params)

    return FiredRecord(sequence, id, interval, scheduled, fired, params)
}
