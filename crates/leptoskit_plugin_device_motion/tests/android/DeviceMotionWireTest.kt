package dev.leptoskit.plugins.devicemotion

import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Exercises fused reading framing without opening SensorManager. */
fun main() {
    val encoded =
        DeviceMotionWire.encodeReading(
            timestampNanos = 1_500_000_000L,
            accelX = 0.1,
            accelY = -0.2,
            accelZ = 0.3,
            gravityX = 0.0,
            gravityY = 0.0,
            gravityZ = 9.8,
            rotationX = 0.01,
            rotationY = -0.02,
            rotationZ = 0.03,
            attitudeAlpha = 1.0,
            attitudeBeta = 0.5,
            attitudeGamma = -0.25,
        )
    check(encoded.size == DeviceMotionWire.READING_BYTES)
    val decoded = DeviceMotionWire.decodeReading(encoded)
    check(decoded[0] == 1.5)
    check(decoded[1] == 0.1)
    check(decoded[12] == -0.25)

    try {
        DeviceMotionWire.decodeReading(encoded.copyOf(encoded.size - 1))
        error("Expected truncated rejection")
    } catch (error: PluginException) {
        check(error.code == PluginException.INVALID_ARGUMENT)
    }

    try {
        DeviceMotionWire.encodeReading(
            timestampNanos = -1L,
            accelX = 0.0,
            accelY = 0.0,
            accelZ = 0.0,
            gravityX = 0.0,
            gravityY = 0.0,
            gravityZ = 0.0,
            rotationX = 0.0,
            rotationY = 0.0,
            rotationZ = 0.0,
            attitudeAlpha = 0.0,
            attitudeBeta = 0.0,
            attitudeGamma = 0.0,
        )
        error("Expected negative timestamp rejection")
    } catch (error: PluginException) {
        check(error.code == PluginException.NATIVE)
    }

    val interval = ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN).putInt(100)
    interval.flip()
    check(DeviceMotionWire.decodeInterval(interval) == 100)

    try {
        val bad = ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN).putInt(1)
        bad.flip()
        DeviceMotionWire.decodeInterval(bad)
        error("Expected interval rejection")
    } catch (error: PluginException) {
        check(error.code == PluginException.INVALID_ARGUMENT)
    }

    check(DeviceMotionWire.encodeAvailability(true).contentEquals(byteArrayOf(1, 0, 0, 0)))
    check(DeviceMotionWire.encodeAvailability(false).contentEquals(byteArrayOf(0, 0, 0, 0)))

    println("DeviceMotion synthetic wire and bounds checks passed")
}
