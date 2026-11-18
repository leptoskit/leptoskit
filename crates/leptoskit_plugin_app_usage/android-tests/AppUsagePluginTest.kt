package dev.leptoskit.plugins.appusage

import android.app.usage.UsageEvents
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.leptoskit.bridge.PluginException
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import org.junit.runner.RunWith
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Synthetic events only: these tests never open Usage Access or read the device's usage log. */
@RunWith(AndroidJUnit4::class)
class AppUsagePluginTest {
    private fun query(
        since: Long,
        until: Long,
        max: Int? = null,
    ): ByteBuffer {
        val buffer = ByteBuffer.allocate(if (max == null) 16 else 20).order(ByteOrder.LITTLE_ENDIAN)
        buffer.putLong(since).putLong(until)
        max?.let { buffer.putInt(it) }
        buffer.flip()

        return buffer
    }

    private fun invalid(block: () -> Unit) {
        try {
            block()
            fail("expected an invalid argument")
        } catch (error: PluginException) {
            assertEquals(PluginException.INVALID_ARGUMENT, error.code)
        }
    }

    private fun resumed(
        at: Long,
        app: String,
    ) = TypedEvent(KIND_ACTIVITY_RESUMED, at, app)

    private fun paused(
        at: Long,
        app: String,
    ) = TypedEvent(KIND_ACTIVITY_PAUSED, at, app)

    private fun device(
        kind: Int,
        at: Long,
    ) = TypedEvent(kind, at, null)

    @Test
    fun queriesHoldTheRustBounds() {
        assertEquals(UsageQuery(1, 2, 0), UsageQuery.decode(query(1, 2), hasMax = false))
        assertEquals(UsageQuery(1, 1, MAX_EVENTS), UsageQuery.decode(query(1, 1, MAX_EVENTS), hasMax = true))

        invalid { UsageQuery.decode(query(-1, 2), hasMax = false) }
        invalid { UsageQuery.decode(query(3, 2), hasMax = false) }
        invalid { UsageQuery.decode(query(1, 2, 0), hasMax = true) }
        invalid { UsageQuery.decode(query(1, 2, MAX_EVENTS + 1), hasMax = true) }
        // A range without its cap, and a cap where none belongs.
        invalid { UsageQuery.decode(query(1, 2), hasMax = true) }
        invalid { UsageQuery.decode(query(1, 2, 1), hasMax = false) }
    }

    @Test
    fun systemEventTypesMapToTheWireVocabulary() {
        assertEquals(KIND_ACTIVITY_RESUMED, typedKind(UsageEvents.Event.ACTIVITY_RESUMED))
        assertEquals(KIND_ACTIVITY_PAUSED, typedKind(UsageEvents.Event.ACTIVITY_PAUSED))
        assertEquals(KIND_SCREEN_INTERACTIVE, typedKind(UsageEvents.Event.SCREEN_INTERACTIVE))
        assertEquals(KIND_SCREEN_NON_INTERACTIVE, typedKind(UsageEvents.Event.SCREEN_NON_INTERACTIVE))
        assertEquals(KIND_KEYGUARD_SHOWN, typedKind(UsageEvents.Event.KEYGUARD_SHOWN))
        assertEquals(KIND_KEYGUARD_HIDDEN, typedKind(UsageEvents.Event.KEYGUARD_HIDDEN))
        assertEquals(KIND_DEVICE_STARTUP, typedKind(UsageEvents.Event.DEVICE_STARTUP))
        assertEquals(KIND_DEVICE_SHUTDOWN, typedKind(UsageEvents.Event.DEVICE_SHUTDOWN))
        assertNull(typedKind(UsageEvents.Event.CONFIGURATION_CHANGE))
    }

    @Test
    fun sessionsCloseOnTheNextResumeAndClipToTheRange() {
        val events =
            listOf(
                // Already in the foreground when the range began.
                paused(150, "a"),
                resumed(200, "b"),
                // A resume closes whichever app was open.
                resumed(300, "c"),
                // A pause for an app that is not open changes nothing.
                paused(350, "b"),
                paused(400, "c"),
                resumed(500, "a"),
            )
        val sessions = buildSessions(events, 100, 600) { it.uppercase() }

        assertEquals(
            listOf(
                TypedSession("a", "A", 100, 150),
                TypedSession("b", "B", 200, 300),
                TypedSession("c", "C", 300, 400),
                // Still open at the end, so it closes there.
                TypedSession("a", "A", 500, 600),
            ),
            sessions,
        )
        assertEquals(listOf("a" to 150L, "b" to 100L, "c" to 100L), appTotals(sessions))
        assertTrue(buildSessions(emptyList(), 0, 10) { it }.isEmpty())
        // A zero-length session is dropped.
        assertTrue(buildSessions(listOf(resumed(10, "a"), paused(10, "a")), 0, 10) { it }.isEmpty())
    }

    @Test
    fun screenTimeCountsStraddlingStretchesAndUnlocks() {
        val events =
            listOf(
                // On at the range's start.
                device(KIND_SCREEN_NON_INTERACTIVE, 150),
                device(KIND_SCREEN_INTERACTIVE, 200),
                device(KIND_KEYGUARD_HIDDEN, 210),
                device(KIND_SCREEN_NON_INTERACTIVE, 300),
                device(KIND_KEYGUARD_SHOWN, 301),
                device(KIND_SCREEN_INTERACTIVE, 500),
                device(KIND_KEYGUARD_HIDDEN, 510),
            )

        assertEquals(50L + 100L + 100L to 2, screenOnAndUnlocks(events, 100, 600))
        assertEquals(0L to 0, screenOnAndUnlocks(emptyList(), 100, 600))
    }

    @Test
    fun repliesFrameLittleEndianFieldsTheRustCrateDecodes() {
        val events = encodeEventBatch(listOf(resumed(1, "a"), device(KIND_KEYGUARD_SHOWN, 2)))
        val expected =
            ByteBuffer
                .allocate(4 + (4 + 8 + 4 + 1) + (4 + 8 + 4))
                .order(ByteOrder.LITTLE_ENDIAN)
                .putInt(2)
                .putInt(KIND_ACTIVITY_RESUMED)
                .putLong(1)
                .putInt(1)
                .put('a'.code.toByte())
                .putInt(KIND_KEYGUARD_SHOWN)
                .putLong(2)
                .putInt(ABSENT)
                .array()
        assertArrayEquals(expected, events)

        val summary = ByteBuffer.wrap(encodeSummary(5, 1, listOf("a" to 7L))).order(ByteOrder.LITTLE_ENDIAN)
        assertEquals(5L, summary.long)
        assertEquals(1, summary.int)
        assertEquals(1, summary.int)
        assertEquals(1, summary.int)
        assertEquals('a'.code.toByte(), summary.get())
        assertEquals(7L, summary.long)
        assertEquals(0, summary.remaining())

        val session = ByteBuffer.wrap(encodeSessionBatch(listOf(TypedSession("a", "", 1, 2)))).order(ByteOrder.LITTLE_ENDIAN)
        assertEquals(1, session.int)
        assertEquals(1, session.int)
        assertEquals('a'.code.toByte(), session.get())
        assertEquals(0, session.int)
        assertEquals(1L, session.long)
        assertEquals(2L, session.long)
        assertEquals(0, session.remaining())
    }

    @Test
    fun labelsAndPackagesStayWithinTheWireBounds() {
        assertEquals("Chat", boundedLabel("Chat"))
        val long = "é".repeat(MAX_LABEL_BYTES)
        val bounded = boundedLabel(long)
        assertTrue(bounded.toByteArray().size <= MAX_LABEL_BYTES)
        assertTrue(long.startsWith(bounded))

        try {
            TypedEvent(KIND_ACTIVITY_RESUMED, 1, null)
            fail("an activity event needs its package")
        } catch (_: IllegalArgumentException) {
        }

        try {
            TypedEvent(KIND_KEYGUARD_SHOWN, 1, "a")
            fail("a device event names no package")
        } catch (_: IllegalArgumentException) {
        }
    }
}
