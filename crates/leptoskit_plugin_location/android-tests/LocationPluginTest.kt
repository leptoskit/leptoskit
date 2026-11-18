package dev.leptoskit.plugins.location

import android.location.Location
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.leptoskit.bridge.PluginException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Synthetic inputs only: these tests never request permission or access user location. */
@RunWith(AndroidJUnit4::class)
class LocationPluginTest {
    @Test
    fun queuedLocationsCannotCrossPermissionOrPrecisionRevocation() {
        assertTrue(permitsLocationDelivery(true, true, true))
        assertTrue(permitsLocationDelivery(false, true, false))
        assertTrue(permitsLocationDelivery(false, true, true))
        assertTrue(!permitsLocationDelivery(true, true, false))
        assertTrue(!permitsLocationDelivery(false, false, false))
        assertTrue(!permitsLocationDelivery(true, false, true))
    }

    @Test
    fun freshnessIgnoresWallClockCorrectionsButRespectsExplicitCursor() {
        val options = LocationOptions(false, 1000, 1000, null)
        val location =
            Location("test").apply {
                time = 1 // Wall clock moved backwards; measurement is still fresh.
                elapsedRealtimeNanos = 1_900_000_000
            }
        assertTrue(acceptsLocation(location, options, 1_800_000_000, 2_000_000_000))
        assertTrue(!acceptsLocation(location, options.copy(after = 2.0), 1_800_000_000, 2_000_000_000))
        location.time = Long.MAX_VALUE
        location.elapsedRealtimeNanos = STALE_NANOS
        assertTrue(!acceptsLocation(location, options, 1_800_000_000, 2_000_000_000))

        location.elapsedRealtimeNanos = 1_700_000_000
        assertTrue(!acceptsLocation(location, options.copy(maximumAge = 0), 1_800_000_000, 2_000_000_000))
    }

    @Test
    fun syntheticCoordinatesPreserveUnknownFieldsAndUnits() {
        val location =
            Location("test").apply {
                time = 1_800_000_000_000
                latitude = 37.0
                longitude = -122.0
                accuracy = 5f
            }
        val bytes = encodeLocation(location)
        assertEquals(84, bytes.size)
        val input = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        assertEquals(0, input.int)
        assertEquals(0, input.int) // Optional values are absent, not fabricated zeros.
        assertEquals(0, input.int) // Unknown provider remains unknown.

        input.position(DOUBLES_OFFSET)
        assertEquals(location.time.toDouble(), input.double, 0.0)
        assertEquals(37.0, input.double, 0.0)
        assertEquals(-122.0, input.double, 0.0)
        assertEquals(5.0, input.double, 0.0)
        location.altitude = 0.0
        location.speed = 0f
        val withZero = ByteBuffer.wrap(encodeLocation(location)).order(ByteOrder.LITTLE_ENDIAN)

        assertEquals(9, withZero.getInt(4))
        location.latitude = MAX_LATITUDE + 1.0
        expectInvalid(PluginException.NATIVE) { encodeLocation(location) }
    }

    /** Rejects bounds the Rust crate would refuse, including a non-finite cursor. */
    @Test
    fun malformedBoundsAndPreviousTimestampsAreRejected() {
        fun payload(
            timeout: Int,
            previous: Double? = null,
        ): ByteBuffer {
            val bytes =
                ByteBuffer
                    .allocate(if (previous == null) BOUNDS_BYTES else PREVIOUS_BYTES)
                    .order(ByteOrder.LITTLE_ENDIAN)
                    .putInt(1)
                    .putInt(timeout)
                    .putInt(0)

            if (previous != null) bytes.putDouble(previous)
            bytes.rewind()

            return bytes
        }
        val valid = LocationOptions.decode(payload(1000), false)
        assertTrue(valid.highAccuracy)
        assertEquals(1000, valid.timeout)

        for (timeout in intArrayOf(0, MAX_TIMEOUT_MS + 1)) {
            expectInvalid(PluginException.INVALID_ARGUMENT) { LocationOptions.decode(payload(timeout), false) }
        }

        expectInvalid(PluginException.INVALID_ARGUMENT) { LocationOptions.decode(payload(1000, Double.NaN), true) }
        expectInvalid(PluginException.INVALID_ARGUMENT) { LocationOptions.decode(payload(1000), true) }
    }

    /** Rejects update options the Rust crate would refuse. */
    @Test
    fun updateOptionsDecodeEveryFieldAndRejectBadWire() {
        fun payload(
            accuracy: Int = 1,
            distance: Double = 12.5,
            interval: Int = 5_000,
            background: Int = 1,
            pauses: Int = 0,
            indicator: Int = 1,
        ): ByteBuffer {
            val bytes =
                ByteBuffer
                    .allocate(UPDATE_OPTIONS_BYTES)
                    .order(ByteOrder.LITTLE_ENDIAN)
                    .putInt(accuracy)
                    .putDouble(distance)
                    .putInt(interval)
                    .putInt(background)
                    .putInt(pauses)
                    .putInt(indicator)
            bytes.rewind()

            return bytes
        }

        val valid = UpdateOptions.decode(payload())
        assertEquals(1, valid.accuracy)
        assertEquals(12.5, valid.distanceFilterM, 0.0)
        assertEquals(5_000, valid.intervalMs)
        assertTrue(valid.background)
        assertTrue(!valid.pausesAutomatically)
        assertTrue(valid.showBackgroundIndicator)

        for (accuracy in intArrayOf(3, -1)) {
            expectInvalid(PluginException.INVALID_ARGUMENT) { UpdateOptions.decode(payload(accuracy = accuracy)) }
        }

        for (distance in doubleArrayOf(-1.0, MAX_DISTANCE_FILTER_M + 1.0, Double.NaN)) {
            expectInvalid(PluginException.INVALID_ARGUMENT) { UpdateOptions.decode(payload(distance = distance)) }
        }

        for (interval in intArrayOf(0, MAX_INTERVAL_MS + 1)) {
            expectInvalid(PluginException.INVALID_ARGUMENT) { UpdateOptions.decode(payload(interval = interval)) }
        }

        for (flag in intArrayOf(2, -1)) {
            expectInvalid(PluginException.INVALID_ARGUMENT) {
                UpdateOptions.decode(payload(background = flag))
            }
        }

        expectInvalid(PluginException.INVALID_ARGUMENT) {
            UpdateOptions.decode(ByteBuffer.allocate(UPDATE_OPTIONS_BYTES - 1))
        }
    }

    /** The bounded buffer drops the oldest fix and reports the drop once. */
    @Test
    fun updateBufferDropsOldestRecordsAndCountsThem() {
        val buffer = UpdateBuffer(capacity = 3)
        val records = (0..2).map { byteArrayOf(it.toByte()) }
        records.forEach(buffer::append)
        assertEquals(records.map { it.toList() }, buffer.drain().map { it.toList() })
        assertEquals(0, buffer.takeDropped())
        assertTrue(buffer.isEmpty())

        records.forEach(buffer::append)
        buffer.append(byteArrayOf(3))
        buffer.append(byteArrayOf(4))
        assertEquals(2, buffer.takeDropped())
        assertEquals(
            listOf(byteArrayOf(2), byteArrayOf(3), byteArrayOf(4)).map { it.toList() },
            buffer.drain().map { it.toList() },
        )
        assertEquals(0, buffer.takeDropped())
    }

    /** One batch is its drop count, its record count, then the records. */
    @Test
    fun updateBatchesFrameDropsAndRecordsInOrder() {
        val location =
            Location("test").apply {
                time = 1_800_000_000_000
                latitude = 37.0
                longitude = -122.0
                accuracy = 5f
            }
        val record = fixRecord(location)
        assertEquals(RECORD_BYTES, record.size)

        // The record is the acquisition reply without its leading tag.
        assertEquals(encodeLocation(location).drop(4), record.toList())

        val reply = encodeBatch(9, listOf(record, record))
        assertEquals(8 + 2 * RECORD_BYTES, reply.size)
        val input = ByteBuffer.wrap(reply).order(ByteOrder.LITTLE_ENDIAN)
        assertEquals(9, input.int)
        assertEquals(2, input.int)
        assertEquals(record.toList(), ByteArray(RECORD_BYTES).also { input.get(it) }.toList())
        assertEquals(record.toList(), ByteArray(RECORD_BYTES).also { input.get(it) }.toList())
        assertEquals(0, input.remaining())
    }

    private companion object {
        /** Three bounds fields, or those plus one timestamp. */
        const val BOUNDS_BYTES = 12
        const val PREVIOUS_BYTES = 20

        /** An update request is six little-endian fields. */
        const val UPDATE_OPTIONS_BYTES = 28

        /** One fix record is five u32 fields and eight doubles. */
        const val RECORD_BYTES = 80

        /** Where the eight doubles start: five u32 fields precede them. */
        const val DOUBLES_OFFSET = 20

        /** A measurement older than the caller's request started. */
        const val STALE_NANOS = 900_000_000L

        /** Values the decoder must reject. */
        const val MAX_TIMEOUT_MS = 120_000
        const val MAX_LATITUDE = 90.0
        const val MAX_INTERVAL_MS = 3_600_000
        const val MAX_DISTANCE_FILTER_M = 10_000.0
    }

    private fun expectInvalid(
        code: Int,
        action: () -> Any,
    ) {
        try {
            action()
            throw AssertionError("Expected native failure $code")
        } catch (
            error: PluginException,
        ) {
            assertEquals(code, error.code)
        }
    }
}
