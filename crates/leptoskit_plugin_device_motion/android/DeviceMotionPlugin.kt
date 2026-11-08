package dev.leptoskit.plugins.devicemotion

import android.app.Activity
import android.app.Application
import android.content.Context
import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CancellableContinuation
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

/**
 * One fused device-motion sample per `NEXT` call from linear acceleration,
 * gravity, gyroscope and rotation-vector sensors. Cancellation, close and
 * backgrounding unregister every listener.
 */
class DeviceMotionPlugin(
    context: Context,
) : NativePlugin {
    override val name = "device_motion"
    override val methods = intArrayOf(DeviceMotionWire.AVAILABLE, DeviceMotionWire.NEXT)

    private val activity = context as? Activity
    private val manager = context.getSystemService(Context.SENSOR_SERVICE) as? SensorManager
    private val handler = Handler(Looper.getMainLooper())
    private val rotationMatrix = FloatArray(9)
    private val orientation = FloatArray(3)

    private var waiting: CancellableContinuation<ByteArray>? = null
    private var listener: SensorEventListener? = null
    private var lifecycle: Application.ActivityLifecycleCallbacks? = null
    private var closed = false

    @Volatile private var accel: SensorEvent? = null

    @Volatile private var gravity: SensorEvent? = null

    @Volatile private var gyro: SensorEvent? = null

    @Volatile private var rotation: SensorEvent? = null

    override suspend fun invoke(
        method: Int,
        payload: java.nio.ByteBuffer,
    ): ByteArray {
        if (closed) throw PluginException(PluginException.CANCELLED)

        if (method !in methods) throw PluginException(PluginException.UNAVAILABLE)

        val sensorManager = manager
        val sensors = requiredSensors(sensorManager)

        if (method == DeviceMotionWire.AVAILABLE) {
            if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

            return DeviceMotionWire.encodeAvailability(sensors != null)
        }

        val intervalMs = DeviceMotionWire.decodeInterval(payload)
        if (sensorManager == null || sensors == null) throw PluginException(PluginException.UNAVAILABLE)

        return awaitFused(intervalMs, sensorManager, sensors)
    }

    private fun requiredSensors(manager: SensorManager?): List<Sensor>? {
        if (manager == null) return null

        val types =
            intArrayOf(
                Sensor.TYPE_LINEAR_ACCELERATION,
                Sensor.TYPE_GRAVITY,
                Sensor.TYPE_GYROSCOPE,
                Sensor.TYPE_ROTATION_VECTOR,
            )
        val sensors = types.map { manager.getDefaultSensor(it) ?: return null }

        return sensors
    }

    private suspend fun awaitFused(
        intervalMs: Int,
        sensorManager: SensorManager,
        sensors: List<Sensor>,
    ): ByteArray {
        if (waiting != null) throw PluginException(PluginException.BUSY)

        return suspendCancellableCoroutine { continuation ->
            startWait(continuation, intervalMs, sensorManager, sensors)
        }
    }

    private fun startWait(
        continuation: CancellableContinuation<ByteArray>,
        intervalMs: Int,
        sensorManager: SensorManager,
        sensors: List<Sensor>,
    ) {
        waiting = continuation
        accel = null
        gravity = null
        gyro = null
        rotation = null
        val started = SystemClock.elapsedRealtimeNanos()
        val observer =
            object : SensorEventListener {
                override fun onAccuracyChanged(
                    sensor: Sensor?,
                    accuracy: Int,
                ) = Unit

                override fun onSensorChanged(event: SensorEvent) {
                    if (waiting !== continuation || !continuation.isActive || event.timestamp < started) return

                    when (event.sensor.type) {
                        Sensor.TYPE_LINEAR_ACCELERATION -> accel = event
                        Sensor.TYPE_GRAVITY -> gravity = event
                        Sensor.TYPE_GYROSCOPE -> gyro = event
                        Sensor.TYPE_ROTATION_VECTOR -> rotation = event
                    }
                    maybeComplete(continuation)
                }
            }

        listener = observer
        continuation.invokeOnCancellation {
            handler.post { if (waiting === continuation) cleanup() }
        }

        if (!continuation.isActive) {
            cleanup()

            return
        }

        try {
            val period = intervalMs * 1000
            for (sensor in sensors) {
                if (!sensorManager.registerListener(observer, sensor, period, handler)) {
                    fail(continuation, PluginException(PluginException.UNAVAILABLE))

                    return
                }
            }
            watchLifecycle(continuation)
        } catch (error: SecurityException) {
            fail(continuation, PluginException(PluginException.PERMISSION_DENIED, error))
        } catch (error: RuntimeException) {
            fail(continuation, PluginException(PluginException.NATIVE, error))
        }
    }

    private fun maybeComplete(continuation: CancellableContinuation<ByteArray>) {
        val accelEvent = accel ?: return
        val gravityEvent = gravity ?: return
        val gyroEvent = gyro ?: return
        val rotationEvent = rotation ?: return

        if (waiting !== continuation || !continuation.isActive) return

        val bytes =
            try {
                encode(accelEvent, gravityEvent, gyroEvent, rotationEvent)
            } catch (error: PluginException) {
                fail(continuation, error)

                return
            }

        cleanup()
        continuation.resume(bytes)
    }

    private fun encode(
        accelEvent: SensorEvent,
        gravityEvent: SensorEvent,
        gyroEvent: SensorEvent,
        rotationEvent: SensorEvent,
    ): ByteArray {
        requireAxes(accelEvent)
        requireAxes(gravityEvent)
        requireAxes(gyroEvent)

        SensorManager.getRotationMatrixFromVector(rotationMatrix, rotationEvent.values)
        SensorManager.getOrientation(rotationMatrix, orientation)

        // Match Expo DeviceMotion attitude signs on Android.
        val alpha = -orientation[0].toDouble()
        val beta = -orientation[1].toDouble()
        val gamma = orientation[2].toDouble()
        if (!alpha.isFinite() || !beta.isFinite() || !gamma.isFinite()) {
            throw PluginException(PluginException.NATIVE)
        }

        return DeviceMotionWire.encodeReading(
            timestampNanos = maxOf(accelEvent.timestamp, gravityEvent.timestamp, gyroEvent.timestamp, rotationEvent.timestamp),
            accelX = accelEvent.values[0].toDouble(),
            accelY = accelEvent.values[1].toDouble(),
            accelZ = accelEvent.values[2].toDouble(),
            gravityX = gravityEvent.values[0].toDouble(),
            gravityY = gravityEvent.values[1].toDouble(),
            gravityZ = gravityEvent.values[2].toDouble(),
            rotationX = gyroEvent.values[0].toDouble(),
            rotationY = gyroEvent.values[1].toDouble(),
            rotationZ = gyroEvent.values[2].toDouble(),
            attitudeAlpha = alpha,
            attitudeBeta = beta,
            attitudeGamma = gamma,
        )
    }

    private fun requireAxes(event: SensorEvent) {
        if (event.values.size < 3 || (0 until 3).any { !event.values[it].isFinite() }) {
            throw PluginException(PluginException.NATIVE)
        }
    }

    private fun watchLifecycle(continuation: CancellableContinuation<ByteArray>) {
        val target = activity ?: return
        val callbacks =
            object : Application.ActivityLifecycleCallbacks {
                override fun onActivityPaused(current: Activity) {
                    if (current !== target || waiting !== continuation) return

                    cleanup()
                    continuation.cancel()
                }

                override fun onActivityCreated(
                    current: Activity,
                    state: Bundle?,
                ) = Unit

                override fun onActivityStarted(current: Activity) = Unit

                override fun onActivityResumed(current: Activity) = Unit

                override fun onActivityStopped(current: Activity) = Unit

                override fun onActivitySaveInstanceState(
                    current: Activity,
                    state: Bundle,
                ) = Unit

                override fun onActivityDestroyed(current: Activity) = onActivityPaused(current)
            }

        lifecycle = callbacks
        target.application.registerActivityLifecycleCallbacks(callbacks)
    }

    override fun close() {
        closed = true
        val continuation = waiting
        cleanup()
        continuation?.cancel()
    }

    private fun fail(
        continuation: CancellableContinuation<ByteArray>,
        error: PluginException,
    ) {
        if (waiting !== continuation) return

        cleanup()
        if (continuation.isActive) continuation.resumeWithException(error)
    }

    private fun cleanup() {
        waiting = null
        listener?.let { manager?.unregisterListener(it) }
        listener = null
        lifecycle?.let { activity?.application?.unregisterActivityLifecycleCallbacks(it) }
        lifecycle = null
        accel = null
        gravity = null
        gyro = null
        rotation = null
    }
}
