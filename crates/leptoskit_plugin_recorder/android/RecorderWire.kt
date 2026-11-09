package dev.leptoskit.plugins.recorder

import dev.leptoskit.bridge.PluginException
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * Wire framing shared with the Rust guest client; `wire.rs` documents the
 * layout. Little-endian throughout, and an optional string is either a u32
 * byte length and its bytes, or the length [ABSENT_LENGTH].
 */
internal const val ABSENT_LENGTH: Int = Int.MIN_VALUE

/** Permission codes, in wire order. */
internal const val PERMISSION_UNKNOWN = 0
internal const val PERMISSION_GRANTED = 1
internal const val PERMISSION_DENIED = 2

/** State codes, in wire order. */
internal const val STATE_IDLE = 0
internal const val STATE_RECORDING = 1
internal const val STATE_INTERRUPTED = 2
internal const val STATE_SILENCED = 3
internal const val STATE_FAILED = 4

/** Why a segment ended, in wire order. */
internal const val ENDED_ROTATION = 0
internal const val ENDED_STOP = 1
internal const val ENDED_INTERRUPTION = 2
internal const val ENDED_RECOVERED = 3

/** The bounds the start request enforces, mirroring `types.rs`. */
internal const val MIN_SEGMENT_SECS = 5
internal const val MAX_SEGMENT_SECS = 3600
internal const val MIN_SAMPLE_RATE = 8000
internal const val MAX_SAMPLE_RATE = 48000
internal const val MIN_BITRATE = 8000
internal const val MAX_BITRATE = 320000
internal const val MIN_THRESHOLD_DBFS = -100.0
internal const val MAX_THRESHOLD_DBFS = 0.0

/** The longest directory path, reason and segment path the wire carries. */
internal const val MAX_DIRECTORY_BYTES = 1024
internal const val MAX_REASON_BYTES = 512
internal const val MAX_PATH_BYTES = 4096

/** The longest string, in bytes, the wire carries. */
private const val MAX_STRING_BYTES = 64 * 1024

/** A typed little-endian reply under construction. */
internal class WireWriter {
    private val output = ByteArrayOutputStream()

    fun u32(value: Int): WireWriter {
        output.write(value and 0xFF)
        output.write(value shr 8 and 0xFF)
        output.write(value shr 16 and 0xFF)
        output.write(value shr 24 and 0xFF)
        return this
    }

    fun i64(value: Long): WireWriter {
        for (shift in 0 until 64 step 8) {
            output.write((value shr shift and 0xFF).toInt())
        }
        return this
    }

    fun f64(value: Double): WireWriter {
        if (!value.isFinite()) throw PluginException(PluginException.NATIVE)
        i64(value.toRawBits())
        return this
    }

    fun string(value: String): WireWriter {
        val bytes = value.toByteArray(Charsets.UTF_8)
        if (bytes.size > MAX_STRING_BYTES) throw PluginException(PluginException.NATIVE)
        u32(bytes.size)
        output.write(bytes)
        return this
    }

    fun optional(value: String?): WireWriter {
        if (value == null) {
            u32(ABSENT_LENGTH)
        } else {
            string(value)
        }
        return this
    }

    fun bytes(): ByteArray = output.toByteArray()
}

/** Reads a typed request the Rust guest client wrote. */
internal class WireReader(
    payload: ByteBuffer,
) {
    private val input = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)

    fun u32(): Int {
        requireAtLeast(4)
        return input.int
    }

    fun i64(): Long {
        requireAtLeast(8)
        return input.long
    }

    fun f64(): Double {
        requireAtLeast(8)
        return Double.fromBits(input.long)
    }

    fun string(): String {
        val length = u32()
        if (length < 0 || length > input.remaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
        val bytes = ByteArray(length)
        input.get(bytes)
        val text = String(bytes, Charsets.UTF_8)
        if (text.toByteArray(Charsets.UTF_8).size != length) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
        return text
    }

    fun optional(): String? {
        requireAtLeast(4)
        val mark = input.position()
        if (input.int == ABSENT_LENGTH) return null
        input.position(mark)
        return string()
    }

    fun finish() {
        if (input.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
    }

    private fun requireAtLeast(size: Int) {
        if (input.remaining() < size) throw PluginException(PluginException.INVALID_ARGUMENT)
    }
}

/** The start request: a directory and four tuned bounds. */
internal class StartRequest(
    payload: ByteBuffer,
) {
    val directory: String
    val segmentSecs: Int
    val sampleRate: Int
    val bitrate: Int
    val thresholdDbfs: Double

    init {
        if (payload.remaining() > MAX_DIRECTORY_BYTES + 32) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        val reader = WireReader(payload)
        val requestedDirectory = reader.string()
        val requestedSegmentSecs = reader.u32()
        val requestedSampleRate = reader.u32()
        val requestedBitrate = reader.u32()
        val requestedThreshold = reader.f64()
        reader.finish()

        if (requestedDirectory.isEmpty() ||
            requestedDirectory.toByteArray(Charsets.UTF_8).size > MAX_DIRECTORY_BYTES ||
            !requestedDirectory.startsWith("/") ||
            requestedDirectory.split("/").contains("..") ||
            requestedDirectory.any { it.code < 32 || it.code == 127 } ||
            requestedSampleRate !in MIN_SAMPLE_RATE..MAX_SAMPLE_RATE ||
            requestedBitrate !in MIN_BITRATE..MAX_BITRATE ||
            requestedThreshold !in MIN_THRESHOLD_DBFS..MAX_THRESHOLD_DBFS
        ) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        directory = requestedDirectory
        segmentSecs = requestedSegmentSecs.coerceIn(MIN_SEGMENT_SECS, MAX_SEGMENT_SECS)
        sampleRate = requestedSampleRate
        bitrate = requestedBitrate
        thresholdDbfs = requestedThreshold
    }
}

/** Encodes the permission reply. */
internal fun encodePermission(code: Int): ByteArray = WireWriter().u32(code).bytes()

/** Encodes the recovery report: files re-queued and files deleted. */
internal fun encodeRecovery(
    recovered: Int,
    discarded: Int,
): ByteArray = WireWriter().u32(recovered).u32(discarded).bytes()

/** One finished segment, before encoding. */
internal class SegmentReply(
    val path: String,
    val startedAtMs: Long,
    val durationMs: Long,
    val bytes: Long,
    val peakDbfs: Double,
    val activeMs: Long,
    val endedBy: Int,
) {
    /** Encodes the next-segment reply carrying this segment. */
    fun encode(): ByteArray {
        if (path.isEmpty() || path.toByteArray(Charsets.UTF_8).size > MAX_PATH_BYTES ||
            startedAtMs <= 0 || durationMs < 0 || bytes <= 0 ||
            activeMs < 0 || activeMs > durationMs
        ) {
            throw PluginException(PluginException.NATIVE)
        }

        return WireWriter()
            .u32(1)
            .string(path)
            .i64(startedAtMs)
            .i64(durationMs)
            .i64(bytes)
            .f64(peakDbfs)
            .i64(activeMs)
            .u32(endedBy)
            .bytes()
    }
}

/** Encodes the empty next-segment reply: nothing waits and nothing records. */
internal fun encodeAbsentSegment(): ByteArray = WireWriter().u32(0).bytes()

/** Encodes a status reply: only failure carries a reason, only an open segment a start. */
internal fun encodeStatus(
    state: Int,
    segmentStartedAtMs: Long,
    reason: String?,
): ByteArray {
    if (reason != null && (reason.isEmpty() || reason.toByteArray(Charsets.UTF_8).size > MAX_REASON_BYTES)) {
        throw PluginException(PluginException.NATIVE)
    }

    return WireWriter()
        .u32(state)
        .i64(segmentStartedAtMs)
        .optional(reason)
        .bytes()
}
