package dev.leptoskit.plugins.health

import androidx.health.connect.client.records.ExerciseSessionRecord
import androidx.health.connect.client.records.HeartRateRecord
import androidx.health.connect.client.records.HeartRateVariabilityRmssdRecord
import androidx.health.connect.client.records.OxygenSaturationRecord
import androidx.health.connect.client.records.Record
import androidx.health.connect.client.records.RespiratoryRateRecord
import androidx.health.connect.client.records.RestingHeartRateRecord
import androidx.health.connect.client.records.SleepSessionRecord
import androidx.health.connect.client.records.StepsRecord
import androidx.health.connect.client.records.metadata.DataOrigin
import androidx.health.connect.client.records.metadata.Device
import androidx.health.connect.client.records.metadata.Metadata
import androidx.health.connect.client.units.Percentage
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.leptoskit.bridge.PluginException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.time.Instant
import java.time.ZoneOffset

/** Synthetic records only: these tests never open Health Connect or prompt. */
@RunWith(AndroidJUnit4::class)
class HealthWireTest {
    @Test
    fun heartRateSeriesEncodesItsSamplesAndSources() {
        val record =
            HeartRateRecord(
                Instant.ofEpochMilli(1_000),
                ZoneOffset.UTC,
                Instant.ofEpochMilli(61_000),
                ZoneOffset.UTC,
                listOf(
                    HeartRateRecord.Sample(Instant.ofEpochMilli(1_000), 62L),
                    HeartRateRecord.Sample(Instant.ofEpochMilli(31_000), 64L),
                ),
                Metadata.activelyRecordedWithId(
                    "hr-1",
                    Device(type = 4, manufacturer = "Google", model = "Pixel Watch"),
                ),
            )
        val input = ByteBuffer.wrap(encodeRecord(record)).order(ByteOrder.LITTLE_ENDIAN)
        assertEquals("hr-1", input.stringField())
        assertEquals(TYPE_HEART_RATE, input.int)
        assertEquals(1_000L, input.long)
        assertEquals(61_000L, input.long)
        assertEquals("", input.stringField()) // The default data origin has no package.
        assertEquals(ABSENT_LENGTH, input.int)
        assertEquals("Google Pixel Watch", input.stringField())
        assertEquals(2, input.int)
        assertEquals(1_000L, input.long)
        assertEquals(62.0, input.double, 0.0)
        assertEquals(31_000L, input.long)
        assertEquals(64.0, input.double, 0.0)
        assertFalse(input.hasRemaining())
    }

    @Test
    fun sleepStagesKeepTheirBoundaries() {
        val sleep =
            SleepSessionRecord(
                Instant.ofEpochMilli(0),
                ZoneOffset.UTC,
                Instant.ofEpochMilli(3_600_000),
                ZoneOffset.UTC,
                metadata("sleep-1"),
                title = null,
                notes = null,
                stages =
                    listOf(
                        SleepSessionRecord.Stage(
                            Instant.ofEpochMilli(0),
                            Instant.ofEpochMilli(1_800_000),
                            SleepSessionRecord.STAGE_TYPE_LIGHT,
                        ),
                        SleepSessionRecord.Stage(
                            Instant.ofEpochMilli(1_800_000),
                            Instant.ofEpochMilli(2_400_000),
                            SleepSessionRecord.STAGE_TYPE_REM,
                        ),
                        SleepSessionRecord.Stage(
                            Instant.ofEpochMilli(2_400_000),
                            Instant.ofEpochMilli(3_600_000),
                            SleepSessionRecord.STAGE_TYPE_AWAKE_IN_BED,
                        ),
                    ),
            )
        val input = ByteBuffer.wrap(encodeRecord(sleep)).order(ByteOrder.LITTLE_ENDIAN)
        input.skipHeader("sleep-1")
        assertEquals(3, input.int)
        for (
        (start, end, stage) in
        listOf(
            Triple(0L, 1_800_000L, STAGE_LIGHT),
            Triple(1_800_000L, 2_400_000L, STAGE_REM),
            Triple(2_400_000L, 3_600_000L, STAGE_AWAKE),
        )
        ) {
            assertEquals(start, input.long)
            assertEquals(end, input.long)
            assertEquals(stage, input.int)
        }
        assertFalse(input.hasRemaining())
    }

    @Test
    fun commonActivitiesStayRecognisableAndOthersKeepTheirPlatformCode() {
        val exercise =
            ExerciseSessionRecord(
                Instant.ofEpochMilli(0),
                ZoneOffset.UTC,
                Instant.ofEpochMilli(60_000),
                ZoneOffset.UTC,
                metadata("run-1"),
                ExerciseSessionRecord.EXERCISE_TYPE_RUNNING_TREADMILL,
                "Morning run",
            )
        val input = ByteBuffer.wrap(encodeRecord(exercise)).order(ByteOrder.LITTLE_ENDIAN)
        input.skipHeader("run-1")
        assertEquals(ACTIVITY_RUNNING, input.int)
        assertEquals(ExerciseSessionRecord.EXERCISE_TYPE_RUNNING_TREADMILL, input.int)
        assertEquals("Morning run", input.stringField())
        assertFalse(input.hasRemaining())

        assertEquals(ACTIVITY_SWIMMING, activityCode(ExerciseSessionRecord.EXERCISE_TYPE_SWIMMING_POOL))
        assertEquals(ACTIVITY_CYCLING, activityCode(ExerciseSessionRecord.EXERCISE_TYPE_BIKING_STATIONARY))
        assertEquals(ACTIVITY_OTHER, activityCode(ExerciseSessionRecord.EXERCISE_TYPE_FENCING))
        assertEquals(STAGE_UNKNOWN, stageCode(SleepSessionRecord.STAGE_TYPE_UNKNOWN))
    }

    @Test
    fun scalarsEncodeTheirUnits() {
        val resting = RestingHeartRateRecord(Instant.ofEpochMilli(5), ZoneOffset.UTC, 55L, metadata("rhr"))
        val hrv = HeartRateVariabilityRmssdRecord(Instant.ofEpochMilli(6), ZoneOffset.UTC, 42.5, metadata("hrv"))
        val oxygen = OxygenSaturationRecord(Instant.ofEpochMilli(7), ZoneOffset.UTC, Percentage(97.5), metadata("spo2"))
        val breaths = RespiratoryRateRecord(Instant.ofEpochMilli(8), ZoneOffset.UTC, 14.25, metadata("rr"))
        val steps =
            StepsRecord(
                Instant.ofEpochMilli(9),
                ZoneOffset.UTC,
                Instant.ofEpochMilli(609_000),
                ZoneOffset.UTC,
                1_234L,
                metadata("steps"),
            )

        for (
        (record, identifier, value) in
        listOf(
            Triple<Record, String, Double>(resting, "rhr", 55.0),
            Triple(hrv, "hrv", 42.5),
            Triple(oxygen, "spo2", 97.5),
            Triple(breaths, "rr", 14.25),
        )
        ) {
            val input = ByteBuffer.wrap(encodeRecord(record)).order(ByteOrder.LITTLE_ENDIAN)
            input.skipHeader(identifier)
            assertEquals(value, input.double, 0.0)
            assertFalse(input.hasRemaining())
        }

        val stepInput = ByteBuffer.wrap(encodeRecord(steps)).order(ByteOrder.LITTLE_ENDIAN)
        stepInput.skipHeader("steps")
        assertEquals(1_234L, stepInput.long)
        assertFalse(stepInput.hasRemaining())
    }

    @Test
    fun requestsRejectWhatTheRustCodecRejects() {
        val valid = readRequest(TYPE_STEPS, 1, 2, 10, ABSENT_LENGTH)
        assertEquals(null, ReadRequest(valid).token)

        // SDNN (3) never reaches Health Connect; RMSSD reads are valid here.
        assertRejected { ReadRequest(readRequest(3, 1, 2, 10, ABSENT_LENGTH)) }
        assertRejected { ReadRequest(readRequest(TYPE_STEPS, 2, 2, 10, ABSENT_LENGTH)) }
        assertRejected { ReadRequest(readRequest(TYPE_STEPS, 1, 2, 0, ABSENT_LENGTH)) }
        assertRejected { ReadRequest(readRequest(TYPE_STEPS, 1, 2, 10, 0)) }

        val types =
            ByteBuffer
                .allocate(8)
                .order(ByteOrder.LITTLE_ENDIAN)
                .putInt(ANDROID_TYPES)
                .putInt(0)
                .flipSafely()
        assertEquals(0, TypesRequest(types).flags)

        assertRejected {
            TypesRequest(
                ByteBuffer
                    .allocate(8)
                    .order(ByteOrder.LITTLE_ENDIAN)
                    .putInt(0)
                    .putInt(0)
                    .flipSafely(),
            )
        }
        assertRejected {
            TypesRequest(
                ByteBuffer
                    .allocate(8)
                    .order(ByteOrder.LITTLE_ENDIAN)
                    .putInt(1 shl 3)
                    .putInt(0)
                    .flipSafely(),
            )
        }
        assertRejected {
            TypesRequest(
                ByteBuffer
                    .allocate(8)
                    .order(ByteOrder.LITTLE_ENDIAN)
                    .putInt(TYPE_STEPS)
                    .putInt(4)
                    .flipSafely(),
            )
        }
    }

    @Test
    fun pagesAndMasksMatchTheRustFraming() {
        val page = encodePage(emptyList(), "next")
        val input = ByteBuffer.wrap(page).order(ByteOrder.LITTLE_ENDIAN)
        assertEquals(0, input.int)
        assertEquals("next", input.stringField())
        assertFalse(input.hasRemaining())

        val last = encodePage(emptyList(), null)
        val trailing = ByteBuffer.wrap(last).order(ByteOrder.LITTLE_ENDIAN)
        assertEquals(0, trailing.int)
        assertEquals(ABSENT_LENGTH, trailing.int)

        val mask = ByteBuffer.wrap(encodeMask(ANDROID_TYPES, 5)).order(ByteOrder.LITTLE_ENDIAN)
        assertEquals(ANDROID_TYPES, mask.int)
        assertEquals(5, mask.int)
    }

    /** Test metadata carrying only an id; the public factories set the rest. */
    private fun metadata(id: String): Metadata = Metadata.manualEntryWithId(id, Device(0))

/** A record whose only optional source is the empty default data origin. */
    private fun ByteBuffer.skipHeader(id: String) {
        assertEquals(id, stringField())
        int
        long
        long
        assertEquals("", stringField())
        assertEquals(ABSENT_LENGTH, int)
        assertEquals(ABSENT_LENGTH, int)
    }
}

private fun readRequest(
    type: Int,
    start: Long,
    end: Long,
    pageSize: Int,
    tokenLength: Int,
): ByteBuffer =
    ByteBuffer
        .allocate(28)
        .order(ByteOrder.LITTLE_ENDIAN)
        .putInt(type)
        .putLong(start)
        .putLong(end)
        .putInt(pageSize)
        .putInt(tokenLength)
        .flipSafely()

/** Reads a length-prefixed string at the buffer's position. */
private fun ByteBuffer.stringField(): String {
    val length = int
    val bytes = ByteArray(length)
    get(bytes)
    return String(bytes, Charsets.UTF_8)
}

/** Flips a finished write buffer for reading. */
private fun ByteBuffer.flipSafely(): ByteBuffer = apply { flip() }

private fun assertRejected(block: () -> Unit) {
    try {
        block()
        error("expected an invalid-argument rejection")
    } catch (_: PluginException) {
        // Expected: the Rust codec rejects the same payloads.
    } catch (_: IllegalArgumentException) {
        // Expected: Kotlin's own bounds checks reject truncated payloads.
    }
}
