package dev.leptoskit.plugins.recorder

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.math.log10
import kotlin.math.roundToInt
import kotlin.math.sin
import kotlin.math.sqrt

/**
 * Wire, level-math, rotation and index tests over synthetic payloads and
 * temporary directories; these never open the microphone or prompt.
 */
@RunWith(AndroidJUnit4::class)
class RecorderWireTest {
    // MARK: Wire framing

    @Test
    fun permissionAndRecoveryRepliesMatchTheRustFraming() {
        assertEquals(listOf<Byte>(2, 0, 0, 0), encodePermission(PERMISSION_DENIED).toList())
        assertEquals(
            listOf<Byte>(3, 0, 0, 0, 1, 0, 0, 0),
            encodeRecovery(3, 1).toList(),
        )
    }

    @Test
    fun startRequestsClampAndRejectLikeTheRustCodec() {
        val clamped = StartRequest(ByteBuffer.wrap(start("/var/recorder", 2, 16_000, 24_000, -45.0)))
        assertEquals(MIN_SEGMENT_SECS, clamped.segmentSecs)
        assertEquals("/var/recorder", clamped.directory)

        val tuned = StartRequest(ByteBuffer.wrap(start("/d", 60, 48_000, 128_000, -60.0)))
        assertEquals(60, tuned.segmentSecs)
        assertEquals(48_000, tuned.sampleRate)

        for (invalid in listOf("relative", "/a/../b", "/o\u0000k")) {
            assertStartRejected(start(invalid, 60, 16_000, 24_000, -45.0))
        }
        assertStartRejected(start("/d", 60, 4_000, 24_000, -45.0))
        assertStartRejected(start("/d", 60, 16_000, 24_000, 1.0))
        assertStartRejected(start("/d", 60, 16_000, 24_000, -120.0))

        val trailing = start("/d", 60, 16_000, 24_000, -45.0) + byteArrayOf(0)
        assertStartRejected(trailing)
    }

    @Test
    fun segmentRepliesCarryTheirMetrics() {
        val segment =
            SegmentReply(
                "/var/recorder/seg-1.m4a",
                1_700_000_000_000,
                300_000,
                900_000,
                -12.5,
                42_000,
                ENDED_ROTATION,
            )
        val bytes = segment.encode()
        val reader = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)

        assertEquals(1, reader.int)
        assertEquals("/var/recorder/seg-1.m4a", stringField(reader))
        assertEquals(1_700_000_000_000L, reader.long)
        assertEquals(300_000L, reader.long)
        assertEquals(900_000L, reader.long)
        assertEquals(-12.5, Double.fromBits(reader.long), 0.0)
        assertEquals(42_000L, reader.long)
        assertEquals(ENDED_ROTATION, reader.int)
        assertEquals(0, reader.remaining())

        assertEquals(listOf<Byte>(0, 0, 0, 0), encodeAbsentSegment().toList())

        val broken =
            SegmentReply("/d/seg.m4a", 1, 10, 1, -10.0, 11, ENDED_STOP)
        try {
            broken.encode()
            fail("A segment with more active than total time was accepted")
        } catch (error: PluginException) {
            assertEquals(PluginException.NATIVE, error.code)
        }
    }

    @Test
    fun statusRepliesPairStatesWithReasonsAndOpenSegments() {
        val recording = encodeStatus(STATE_RECORDING, 1_700_000_000_000, null)
        val reader = ByteBuffer.wrap(recording).order(ByteOrder.LITTLE_ENDIAN)
        assertEquals(STATE_RECORDING, reader.int)
        assertEquals(1_700_000_000_000L, reader.long)
        assertEquals(ABSENT_LENGTH, reader.int)
        assertEquals(0, reader.remaining())

        val failed = encodeStatus(STATE_FAILED, 0, "engine stopped")
        val failureReader = ByteBuffer.wrap(failed).order(ByteOrder.LITTLE_ENDIAN)
        assertEquals(STATE_FAILED, failureReader.int)
        assertEquals(0L, failureReader.long)
        assertEquals("engine stopped", stringField(failureReader))
        assertEquals(0, failureReader.remaining())
    }

    // MARK: Level math

    @Test
    fun frameLevelsMeasureRmsAndPeakAgainstFullScale() {
        // A full-scale sine: RMS −3.01 dBFS, peak 0 dBFS.
        val loud = RecorderLevels.measure(sine(32767.0f, 480))
        assertEquals(-3.0103, loud.rmsDbfs, 0.001)
        assertEquals(0.0, loud.peakDbfs, 0.001)
        assertTrue(loud.isActive(-45.0))

        // A half-scale sine: RMS −9.03 dBFS, peak −6.02 dBFS.
        val half = RecorderLevels.measure(sine(16384.0f, 480))
        assertEquals(-9.0309, half.rmsDbfs, 0.001)
        assertEquals(-6.0206, half.peakDbfs, 0.001)

        // Digital silence floors at −120 dBFS.
        val silence = RecorderLevels.measure(ShortArray(480))
        assertEquals(RecorderLevels.FLOOR_DBFS, silence.rmsDbfs, 0.0)
        assertEquals(RecorderLevels.FLOOR_DBFS, silence.peakDbfs, 0.0)
        assertFalse(silence.isActive(-45.0))

        // A quiet sine below the threshold.
        val quiet = RecorderLevels.measure(sine(180.0f, 480))
        assertEquals(20.0 * log10(180.0 / 32768.0 / sqrt(2.0)), quiet.rmsDbfs, 0.01)
    }

    @Test
    fun segmentTotalsAccumulateActiveAndQuietFrames() {
        val levels = SegmentLevels(-45.0)
        repeat(10) { levels.add(RecorderLevels.measure(sine(32767.0f, 480))) }
        repeat(5) { levels.add(RecorderLevels.measure(ShortArray(480))) }

        assertEquals(450L, levels.durationMs)
        assertEquals(300L, levels.activeMs)
        assertEquals(0.0, levels.peakDbfs, 0.001)
    }

    // MARK: Rotation timing

    @Test
    fun segmentsRotateExactlyAtTheBoundary() {
        assertEquals(480, RecorderLevels.frameSamples(16_000))

        assertNull(RecorderLevels.split(0, 480, 80_000))
        assertNull(RecorderLevels.split(79_500, 480, 80_000))
        assertEquals(100, RecorderLevels.split(79_900, 480, 80_000))
        assertNull(RecorderLevels.split(80_000, 480, 80_000))

        var written = 0L
        repeat(80_000 / 480) {
            assertNull(RecorderLevels.split(written, 480, 80_000))
            written += 480
        }
        assertEquals(79_680L, written)
        assertEquals(320, RecorderLevels.split(written, 480, 80_000))
    }

    // MARK: Index journal and recovery

    @Test
    fun journalRoundTripsAndSkipsTornLines() {
        val directory = temporaryDirectory()
        val index = RecorderIndex(directory)

        index.recordOpen("seg-1.m4a", 100, 300_000)
        index.recordFinalised("seg-1.m4a", 300_000, 900_000, -12.5, 42_000, ENDED_ROTATION)
        index.recordDelivered("seg-1.m4a")

        val lines = index.read()
        assertEquals(3, lines.size)
        assertEquals(RecorderIndex.Line.Open("seg-1.m4a", 100, 300_000), lines.first())
        assertEquals(RecorderIndex.Line.Delivered("seg-1.m4a"), lines.last())

        val torn = "F\tseg-2.m4a\t100\nC\tseg-2.m4a\t1\t2\t3\nD\nX\tnonsense\n"
        assertEquals(emptyList<RecorderIndex.Line>(), RecorderIndex.parse(torn))

        directory.deleteRecursively()
    }

    @Test
    fun recoveryDeliversUndeliveredFilesAndDeletesTruncatedOnes() {
        val journal =
            "F\tseg-1.m4a\t100\t300000\n" +
                "C\tseg-1.m4a\t300000\t900000\t-12.5\t42000\t0\n" +
                "D\tseg-1.m4a\n" +
                "F\tseg-2.m4a\t300100\t300000\n" +
                "C\tseg-2.m4a\t300000\t900000\t-30.0\t1000\t0\n" +
                "F\tseg-3.m4a\t600200\t300000\n"

        val plan = RecorderIndex.plan(RecorderIndex.parse(journal), listOf("seg-1.m4a", "seg-2.m4a", "seg-3.m4a", "seg-4.m4a", "notes.txt"))

        assertEquals(listOf("seg-2.m4a"), plan.deliverable.map { it.filename })
        assertEquals(300_100L, plan.deliverable.first().startedAtMs)
        assertEquals(-30.0, plan.deliverable.first().peakDbfs, 0.0)
        assertEquals(
            "the truncated file and the unindexed orphan are deleted, the app's other files are not",
            listOf("seg-3.m4a", "seg-4.m4a"),
            plan.discardable,
        )

        val settled =
            RecorderIndex.plan(
                RecorderIndex.parse("F\ts.m4a\t1\t1\nC\ts.m4a\t1\t1\t-1.0\t0\t1\nD\ts.m4a\n"),
                listOf("s.m4a"),
            )
        assertTrue(settled.deliverable.isEmpty())
        assertTrue(settled.discardable.isEmpty())
    }

    @Test
    fun compactionKeepsOnlyUndeliveredEntries() {
        val directory = temporaryDirectory()
        val index = RecorderIndex(directory)

        for (number in 1..3) {
            index.recordOpen("seg-$number.m4a", 100L * number, 1)
            index.recordFinalised("seg-$number.m4a", 1, 10, -1.0, 0, ENDED_ROTATION)
            index.recordDelivered("seg-$number.m4a")
        }
        assertEquals(9, index.read().size)

        val keeping =
            listOf(
                RecorderIndex.Finalised("seg-4.m4a", 400, 1, 10, -1.0, 0, ENDED_ROTATION),
            )
        index.compact(keeping)

        val lines = index.read()
        assertEquals(
            listOf(
                RecorderIndex.Line.Open("seg-4.m4a", 400, 1),
                RecorderIndex.Line.FinalisedLine("seg-4.m4a", 1, 10, -1.0, 0, ENDED_ROTATION),
            ),
            lines,
        )

        directory.deleteRecursively()
    }

    // MARK: Plugin validation

    @Test
    fun statusReportsIdleAndStartRejectsBadDirectories() =
        runBlocking {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val plugin = withContext(Dispatchers.Main) { RecorderPlugin(context) }
            try {
                val status =
                    withContext(Dispatchers.Main) {
                        plugin.invoke(RecorderPlugin.STATUS, ByteBuffer.allocate(0))
                    }
                assertEquals(16, status.size)
                val reader = ByteBuffer.wrap(status).order(ByteOrder.LITTLE_ENDIAN)
                assertEquals(STATE_IDLE, reader.int)
                assertEquals(0L, reader.long)
                assertEquals(ABSENT_LENGTH, reader.int)

                withContext(Dispatchers.Main) {
                    plugin.invoke(
                        RecorderPlugin.START,
                        ByteBuffer.wrap(start("/sdcard/recorder", 60, 16_000, 24_000, -45.0)),
                    )
                }
                fail("A directory outside the app's own data dirs was accepted")
            } catch (error: PluginException) {
                assertEquals(PluginException.INVALID_ARGUMENT, error.code)
            } finally {
                withContext(Dispatchers.Main) { plugin.close() }
            }
        }

    @Test
    fun startRejectsMalformedPayloads() =
        runBlocking {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val plugin = withContext(Dispatchers.Main) { RecorderPlugin(context) }
            try {
                withContext(Dispatchers.Main) {
                    plugin.invoke(RecorderPlugin.START, ByteBuffer.wrap(ByteArray(2)))
                }
                fail("A truncated start payload was accepted")
            } catch (error: PluginException) {
                assertEquals(PluginException.INVALID_ARGUMENT, error.code)
            } finally {
                withContext(Dispatchers.Main) { plugin.close() }
            }
        }
}

/** Builds a start request payload. */
private fun start(
    directory: String,
    segmentSecs: Int,
    sampleRate: Int,
    bitrate: Int,
    threshold: Double,
): ByteArray =
    WireWriter()
        .string(directory)
        .u32(segmentSecs)
        .u32(sampleRate)
        .u32(bitrate)
        .f64(threshold)
        .bytes()

/** Asserts the codec rejects a malformed start payload. */
private fun assertStartRejected(payload: ByteArray) {
    try {
        StartRequest(ByteBuffer.wrap(payload))
        fail("A malformed start payload was accepted")
    } catch (_: PluginException) {
        // Expected: the Rust codec rejects the same bytes.
    }
}

/** Reads one length-prefixed string field. */
private fun stringField(reader: ByteBuffer): String {
    val length = reader.int
    val bytes = ByteArray(length)
    reader.get(bytes)
    return String(bytes, Charsets.UTF_8)
}

/** A sine wave of `count` samples at `amplitude`, full scale at 32768. */
private fun sine(
    amplitude: Float,
    count: Int,
): ShortArray =
    ShortArray(count) { index ->
        (amplitude * sin(index * 2.0 * Math.PI / 12.0)).roundToInt().toShort()
    }

/** A fresh directory under the instrumentation cache, removed by the test. */
private fun temporaryDirectory(): File {
    val directory = File.createTempFile("recorder-index", null)
    directory.delete()
    directory.mkdirs()
    return directory
}
