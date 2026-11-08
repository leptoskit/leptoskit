package dev.leptoskit.plugins.devicemotion

import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Fixed fused reading layout shared with the Rust codec. */
internal object DeviceMotionWire {
    const val AVAILABLE = 0
    const val NEXT = 1
    const val READING_BYTES = 13 * 8
    const val MIN_INTERVAL_MS = 5
    const val MAX_INTERVAL_MS = 60_000
    private const val NANOS_PER_SECOND = 1_000_000_000.0

    fun decodeInterval(payload: ByteBuffer): Int {
        if (payload.remaining() != 4) throw PluginException(PluginException.INVALID_ARGUMENT)

        val interval = payload.order(ByteOrder.LITTLE_ENDIAN).int
        if (interval !in MIN_INTERVAL_MS..MAX_INTERVAL_MS) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        return interval
    }

    fun encodeAvailability(available: Boolean): ByteArray = byteArrayOf(if (available) 1 else 0, 0, 0, 0)

    fun encodeReading(
        timestampNanos: Long,
        accelX: Double,
        accelY: Double,
        accelZ: Double,
        gravityX: Double,
        gravityY: Double,
        gravityZ: Double,
        rotationX: Double,
        rotationY: Double,
        rotationZ: Double,
        attitudeAlpha: Double,
        attitudeBeta: Double,
        attitudeGamma: Double,
    ): ByteArray {
        val timestamp = timestampNanos.toDouble() / NANOS_PER_SECOND
        val values =
            doubleArrayOf(
                timestamp,
                accelX,
                accelY,
                accelZ,
                gravityX,
                gravityY,
                gravityZ,
                rotationX,
                rotationY,
                rotationZ,
                attitudeAlpha,
                attitudeBeta,
                attitudeGamma,
            )

        if (timestampNanos < 0 || values.any { !it.isFinite() }) {
            throw PluginException(PluginException.NATIVE)
        }

        val bytes = ByteBuffer.allocate(READING_BYTES).order(ByteOrder.LITTLE_ENDIAN)
        values.forEach { bytes.putDouble(it) }

        return bytes.array()
    }

    fun decodeReading(payload: ByteArray): DoubleArray {
        if (payload.size != READING_BYTES) throw PluginException(PluginException.INVALID_ARGUMENT)

        val bytes = ByteBuffer.wrap(payload).order(ByteOrder.LITTLE_ENDIAN)
        val values = DoubleArray(13) { bytes.double }
        if (values[0] < 0.0 || values.any { !it.isFinite() }) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        return values
    }
}
