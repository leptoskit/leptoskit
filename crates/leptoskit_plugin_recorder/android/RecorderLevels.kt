package dev.leptoskit.plugins.recorder

import kotlin.math.abs
import kotlin.math.log10
import kotlin.math.max
import kotlin.math.sqrt

/**
 * The level math shared by the engine and its tests: 30-millisecond frames
 * of converted samples, measured in dBFS, and the segment totals they add
 * up to. Digital silence floors at [FLOOR_DBFS] so replies stay finite.
 */
internal object RecorderLevels {
    /** The quietest level a segment reports, in dBFS. */
    const val FLOOR_DBFS = -120.0

    /** Every frame covers 30 milliseconds of audio. */
    const val FRAME_MS = 30

    /** Below this magnitude a level is digital silence. */
    private const val SILENCE = 1e-6

    /** One measured frame. */
    data class Frame(
        val rmsDbfs: Double,
        val peakDbfs: Double,
    ) {
        /** Whether the frame counts as active at `thresholdDbfs`. */
        fun isActive(thresholdDbfs: Double): Boolean = rmsDbfs > thresholdDbfs
    }

    /** Measures one frame of PCM samples. */
    fun measure(frame: ShortArray): Frame {
        var sum = 0.0
        var peak = 0.0

        for (sample in frame) {
            val magnitude = abs(sample.toDouble()) / 32768.0
            sum += magnitude * magnitude
            peak = max(peak, magnitude)
        }

        val rms = sqrt(sum / frame.size)
        return Frame(dbfs(rms), dbfs(peak))
    }

    /** A magnitude in dBFS, floored for digital silence. */
    fun dbfs(magnitude: Double): Double = max(20.0 * log10(max(magnitude, SILENCE)), FLOOR_DBFS)

    /** How many samples one 30-millisecond frame holds at `sampleRate`. */
    fun frameSamples(sampleRate: Int): Int = max(sampleRate * FRAME_MS / 1000, 1)

    /**
     * Where a chunk of `count` samples, arriving after `written` samples of
     * the current segment, splits at the `segmentFrames` boundary: the
     * number of samples that still belong to the current segment, or `null`
     * when the whole chunk fits.
     */
    fun split(
        written: Long,
        count: Int,
        segmentFrames: Long,
    ): Int? {
        if (written + count <= segmentFrames || written >= segmentFrames) return null
        return (segmentFrames - written).toInt()
    }
}

/** The running totals of one segment, in the units the reply carries. */
internal class SegmentLevels(
    private val thresholdDbfs: Double,
) {
    /** The segment's loudest frame peak so far, in dBFS. */
    var peakDbfs = RecorderLevels.FLOOR_DBFS
        private set

    /** Milliseconds of frames above the activity threshold so far. */
    var activeMs = 0L
        private set

    /** Milliseconds of audio so far. */
    var durationMs = 0L
        private set

    /** Adds one measured frame to the totals. */
    fun add(frame: RecorderLevels.Frame) {
        peakDbfs = max(peakDbfs, frame.peakDbfs)
        durationMs += RecorderLevels.FRAME_MS
        if (frame.isActive(thresholdDbfs)) {
            activeMs += RecorderLevels.FRAME_MS
        }
    }
}
