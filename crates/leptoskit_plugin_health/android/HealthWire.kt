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
import dev.leptoskit.bridge.PluginException
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * Wire framing shared with the Rust guest client; `records.rs` documents the
 * layout. Little-endian throughout, and an optional string is either a u32
 * byte length and its bytes, or the length [ABSENT_LENGTH].
 */
internal const val ABSENT_LENGTH: Int = Int.MIN_VALUE

/** Data type codes, in wire order; SDNN (3) is HealthKit only. */
internal const val TYPE_HEART_RATE = 0
internal const val TYPE_RESTING_HEART_RATE = 1
internal const val TYPE_HRV_RMSSD = 2
internal const val TYPE_STEPS = 4
internal const val TYPE_SLEEP = 5
internal const val TYPE_OXYGEN_SATURATION = 6
internal const val TYPE_RESPIRATORY_RATE = 7
internal const val TYPE_EXERCISE = 8

/** Every type Health Connect records, as a bit mask. */
internal const val ANDROID_TYPES = 0x1F3

/** Sleep stage codes, in wire order. */
internal const val STAGE_UNKNOWN = 0
internal const val STAGE_AWAKE = 1
internal const val STAGE_ASLEEP = 2
internal const val STAGE_LIGHT = 3
internal const val STAGE_DEEP = 4
internal const val STAGE_REM = 5
internal const val STAGE_IN_BED = 6

/** Common activity codes, in wire order; anything else stays [ACTIVITY_OTHER]. */
internal const val ACTIVITY_OTHER = 0
internal const val ACTIVITY_WALKING = 1
internal const val ACTIVITY_RUNNING = 2
internal const val ACTIVITY_CYCLING = 3
internal const val ACTIVITY_SWIMMING = 4
internal const val ACTIVITY_HIKING = 5
internal const val ACTIVITY_STRENGTH = 6
internal const val ACTIVITY_YOGA = 7
internal const val ACTIVITY_ROWING = 8
internal const val ACTIVITY_ELLIPTICAL = 9
internal const val ACTIVITY_STAIR_CLIMBING = 10
internal const val ACTIVITY_DANCING = 11
internal const val ACTIVITY_PILATES = 12
internal const val ACTIVITY_MARTIAL_ARTS = 13

/** The most samples one read may ask for. */
internal const val MAX_PAGE_SIZE = 1000

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

    fun raw(bytes: ByteArray): WireWriter {
        output.write(bytes)
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

    fun optional(): String? =
        if (input.remaining() >= 4) {
            val mark = input.position()
            if (input.int == ABSENT_LENGTH) {
                null
            } else {
                input.position(mark)
                string()
            }
        } else {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }

    fun finished() {
        if (input.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
    }

    fun remaining(): Int = input.remaining()

    private fun requireAtLeast(size: Int) {
        if (input.remaining() < size) throw PluginException(PluginException.INVALID_ARGUMENT)
    }
}

/** A read request: one type, a window, a page size and a page token. */
internal class ReadRequest(
    payload: ByteBuffer,
) {
    val type: Int
    val startMs: Long
    val endMs: Long
    val pageSize: Int
    val token: String?

    init {
        val reader = WireReader(payload)
        type = reader.u32()
        startMs = reader.i64()
        endMs = reader.i64()
        pageSize = reader.u32()
        token = reader.optional()
        reader.finished()
        if (type !in SUPPORTED_TYPES || startMs >= endMs || pageSize !in 1..MAX_PAGE_SIZE || token == "") {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
    }

    companion object {
        /** The type codes a read accepts; SDNN never reaches Health Connect. */
        val SUPPORTED_TYPES =
            setOf(
                TYPE_HEART_RATE,
                TYPE_RESTING_HEART_RATE,
                TYPE_HRV_RMSSD,
                TYPE_STEPS,
                TYPE_SLEEP,
                TYPE_OXYGEN_SATURATION,
                TYPE_RESPIRATORY_RATE,
                TYPE_EXERCISE,
            )
    }
}

/**
 * A permission or changes-token request: a type mask and extra flags. Types
 * Health Connect never records are dropped rather than refused, so one
 * request can name both platforms' heart-rate-variability types.
 */
internal class TypesRequest(
    payload: ByteBuffer,
) {
    val mask: Int
    val flags: Int

    init {
        val reader = WireReader(payload)
        val requested = reader.u32()
        flags = reader.u32()
        reader.finished()
        mask = requested and ANDROID_TYPES
        if (requested == 0 || mask == 0 || (flags and 0b11.inv()) != 0) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
    }
}

/** One record encoded as the wire's sample layout. */
internal fun encodeRecord(record: Record): ByteArray {
    val writer = WireWriter()
    val (start, end) = intervalOf(record)
    val metadata = record.metadata
    if (metadata.id.isEmpty()) throw PluginException(PluginException.NATIVE)
    writer.string(metadata.id)
    writer.u32(typeCodeOf(record))
    writer.i64(start)
    writer.i64(end)
    writer.optional(metadata.dataOrigin.packageName)
    writer.optional(null)
    writer.optional(deviceName(metadata))
    when (record) {
        is HeartRateRecord -> {
            writer.u32(record.samples.size)
            for (sample in record.samples) {
                writer.i64(sample.time.toEpochMilli())
                writer.f64(sample.beatsPerMinute.toDouble())
            }
        }

        is RestingHeartRateRecord -> {
            writer.f64(record.beatsPerMinute.toDouble())
        }

        is HeartRateVariabilityRmssdRecord -> {
            writer.f64(record.heartRateVariabilityMillis)
        }

        is StepsRecord -> {
            writer.i64(record.count)
        }

        is SleepSessionRecord -> {
            writer.u32(record.stages.size)
            for (stage in record.stages) {
                writer.i64(stage.startTime.toEpochMilli())
                writer.i64(stage.endTime.toEpochMilli())
                writer.u32(stageCode(stage.stage))
            }
        }

        is OxygenSaturationRecord -> {
            writer.f64(record.percentage.value)
        }

        is RespiratoryRateRecord -> {
            writer.f64(record.rate)
        }

        is ExerciseSessionRecord -> {
            writer.u32(activityCode(record.exerciseType))
            writer.u32(record.exerciseType)
            writer.optional(record.title)
        }

        else -> {
            throw PluginException(PluginException.NATIVE)
        }
    }
    return writer.bytes()
}

/** A page of records and the token the next page needs. */
internal fun encodePage(
    records: List<Record>,
    token: String?,
): ByteArray {
    val writer = WireWriter()
    writer.u32(records.size)
    for (record in records) {
        writer.raw(encodeRecord(record))
    }
    writer.optional(token)
    return writer.bytes()
}

/** Grants or features: a type mask and flag bits. */
internal fun encodeMask(
    mask: Int,
    flags: Int,
): ByteArray = WireWriter().u32(mask).u32(flags).bytes()

/** The wire code of a record's data type. */
private fun typeCodeOf(record: Record): Int =
    when (record) {
        is HeartRateRecord -> TYPE_HEART_RATE
        is RestingHeartRateRecord -> TYPE_RESTING_HEART_RATE
        is HeartRateVariabilityRmssdRecord -> TYPE_HRV_RMSSD
        is StepsRecord -> TYPE_STEPS
        is SleepSessionRecord -> TYPE_SLEEP
        is OxygenSaturationRecord -> TYPE_OXYGEN_SATURATION
        is RespiratoryRateRecord -> TYPE_RESPIRATORY_RATE
        is ExerciseSessionRecord -> TYPE_EXERCISE
        else -> throw PluginException(PluginException.NATIVE)
    }

/** A record's window, in Unix milliseconds. */
private fun intervalOf(record: Record): Pair<Long, Long> =
    when (record) {
        is HeartRateRecord -> Pair(record.startTime.toEpochMilli(), record.endTime.toEpochMilli())
        is StepsRecord -> Pair(record.startTime.toEpochMilli(), record.endTime.toEpochMilli())
        is SleepSessionRecord -> Pair(record.startTime.toEpochMilli(), record.endTime.toEpochMilli())
        is ExerciseSessionRecord -> Pair(record.startTime.toEpochMilli(), record.endTime.toEpochMilli())
        is RestingHeartRateRecord -> instantOf(record.time)
        is HeartRateVariabilityRmssdRecord -> instantOf(record.time)
        is OxygenSaturationRecord -> instantOf(record.time)
        is RespiratoryRateRecord -> instantOf(record.time)
        else -> throw PluginException(PluginException.NATIVE)
    }

private fun instantOf(time: java.time.Instant): Pair<Long, Long> = Pair(time.toEpochMilli(), time.toEpochMilli())

/** The recording device as one name, when the platform reports one. */
private fun deviceName(metadata: androidx.health.connect.client.records.metadata.Metadata): String? =
    metadata.device
        ?.let { device -> listOfNotNull(device.manufacturer, device.model).joinToString(" ") }
        ?.ifEmpty { null }

/** A sleep stage constant as its wire code. */
internal fun stageCode(stage: Int): Int =
    when (stage) {
        SleepSessionRecord.STAGE_TYPE_AWAKE,
        SleepSessionRecord.STAGE_TYPE_AWAKE_IN_BED,
        SleepSessionRecord.STAGE_TYPE_OUT_OF_BED,
        -> STAGE_AWAKE

        SleepSessionRecord.STAGE_TYPE_SLEEPING -> STAGE_ASLEEP

        SleepSessionRecord.STAGE_TYPE_LIGHT -> STAGE_LIGHT

        SleepSessionRecord.STAGE_TYPE_DEEP -> STAGE_DEEP

        SleepSessionRecord.STAGE_TYPE_REM -> STAGE_REM

        else -> STAGE_UNKNOWN
    }

/** An exercise type constant as its common-activity code. */
internal fun activityCode(type: Int): Int =
    when (type) {
        ExerciseSessionRecord.EXERCISE_TYPE_WALKING -> ACTIVITY_WALKING

        ExerciseSessionRecord.EXERCISE_TYPE_RUNNING,
        ExerciseSessionRecord.EXERCISE_TYPE_RUNNING_TREADMILL,
        -> ACTIVITY_RUNNING

        ExerciseSessionRecord.EXERCISE_TYPE_BIKING,
        ExerciseSessionRecord.EXERCISE_TYPE_BIKING_STATIONARY,
        -> ACTIVITY_CYCLING

        ExerciseSessionRecord.EXERCISE_TYPE_SWIMMING_POOL -> ACTIVITY_SWIMMING

        ExerciseSessionRecord.EXERCISE_TYPE_HIKING -> ACTIVITY_HIKING

        ExerciseSessionRecord.EXERCISE_TYPE_STRENGTH_TRAINING,
        ExerciseSessionRecord.EXERCISE_TYPE_WEIGHTLIFTING,
        -> ACTIVITY_STRENGTH

        ExerciseSessionRecord.EXERCISE_TYPE_YOGA -> ACTIVITY_YOGA

        ExerciseSessionRecord.EXERCISE_TYPE_ROWING,
        ExerciseSessionRecord.EXERCISE_TYPE_ROWING_MACHINE,
        -> ACTIVITY_ROWING

        ExerciseSessionRecord.EXERCISE_TYPE_ELLIPTICAL -> ACTIVITY_ELLIPTICAL

        ExerciseSessionRecord.EXERCISE_TYPE_STAIR_CLIMBING -> ACTIVITY_STAIR_CLIMBING

        ExerciseSessionRecord.EXERCISE_TYPE_DANCING -> ACTIVITY_DANCING

        ExerciseSessionRecord.EXERCISE_TYPE_PILATES -> ACTIVITY_PILATES

        ExerciseSessionRecord.EXERCISE_TYPE_MARTIAL_ARTS -> ACTIVITY_MARTIAL_ARTS

        else -> ACTIVITY_OTHER
    }
