package dev.leptoskit.plugins.wearable

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import android.os.Build
import androidx.core.content.ContextCompat
import androidx.health.services.client.HealthServices
import androidx.health.services.client.MeasureCallback
import androidx.health.services.client.PassiveListenerCallback
import androidx.health.services.client.data.Availability
import androidx.health.services.client.data.DataPointContainer
import androidx.health.services.client.data.DataType
import androidx.health.services.client.data.DeltaDataType
import androidx.health.services.client.data.PassiveListenerConfig
import androidx.health.services.client.data.SampleDataPoint
import com.google.common.util.concurrent.ListenableFuture
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.suspendCancellableCoroutine
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.ArrayDeque
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

/**
 * Direct on-watch biometric sensor plugin for Wear OS devices (Pixel Watch,
 * Samsung Galaxy Watch / One UI Watch, TicWatch, OnePlus Watch, etc.).
 *
 * Uses Android Wear Health Services (`PassiveMonitoringClient`, `MeasureClient`)
 * plus Android `SensorManager` (`TYPE_HEART_RATE`, `TYPE_STEP_COUNTER`,
 * `TYPE_LOW_LATENCY_OFFBODY_DETECT`), and detects the optional Samsung Health
 * Sensor SDK at runtime when installed on Samsung Galaxy Watch hardware.
 */
class WearablePlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "wearable"
    override val methods = intArrayOf(0, 1, 2, 3, 4, 5, 6, 7)

    private val sensorManager = context.getSystemService(Context.SENSOR_SERVICE) as? SensorManager
    private val lock = Any()
    private val buffer = ArrayDeque<ByteArray>()
    private var dropped = 0
    private var waiter: CompletableDeferred<ByteArray>? = null
    private var closed = false
    private var passiveRunning = false
    private var liveRunning = false
    private var lastStepCounter: Long? = null
    private var lastStepTimeMs: Long? = null

    private val sensorListener =
        object : SensorEventListener {
            override fun onSensorChanged(event: SensorEvent) {
                val now = System.currentTimeMillis()
                when (event.sensor.type) {
                    Sensor.TYPE_HEART_RATE -> {
                        val bpm = event.values.firstOrNull()?.toDouble() ?: return
                        if (bpm > 0.0) {
                            enqueue(encodeHeartRate(now, bpm, mapAccuracy(event.accuracy)))
                        }
                    }

                    Sensor.TYPE_LOW_LATENCY_OFFBODY_DETECT -> {
                        val worn = (event.values.firstOrNull() ?: 0f) >= 1f
                        enqueue(encodeWearState(now, worn))
                    }

                    Sensor.TYPE_STEP_COUNTER -> {
                        val total = event.values.firstOrNull()?.toLong() ?: return
                        val prev = lastStepCounter
                        val prevTime = lastStepTimeMs
                        lastStepCounter = total
                        lastStepTimeMs = now
                        if (prev != null && prevTime != null && total > prev) {
                            enqueue(encodeSteps(prevTime, now, total - prev))
                        }
                    }
                }
            }

            override fun onAccuracyChanged(sensor: Sensor?, accuracy: Int) = Unit
        }

    private val measureCallback =
        object : MeasureCallback {
            override fun onAvailabilityChanged(dataType: DeltaDataType<*, *>, availability: Availability) = Unit

            override fun onDataReceived(data: DataPointContainer) {
                val now = System.currentTimeMillis()
                for (point in data.getData(DataType.HEART_RATE_BPM)) {
                    val bpm = (point as? SampleDataPoint<Double>)?.value ?: continue
                    if (bpm > 0.0) enqueue(encodeHeartRate(now, bpm, ACCURACY_HIGH))
                }
            }
        }

    private val passiveCallback =
        object : PassiveListenerCallback {
            override fun onNewDataPointsReceived(dataPoints: DataPointContainer) {
                val now = System.currentTimeMillis()
                for (point in dataPoints.getData(DataType.HEART_RATE_BPM)) {
                    val bpm = (point as? SampleDataPoint<Double>)?.value ?: continue
                    if (bpm > 0.0) enqueue(encodeHeartRate(now, bpm, ACCURACY_HIGH))
                }
                for (point in dataPoints.getData(DataType.STEPS)) {
                    val count = point.value
                    if (count > 0L) {
                        enqueue(encodeSteps(now - 60_000L, now, count))
                    }
                }
                for (point in dataPoints.getData(DataType.CALORIES)) {
                    if (point.value > 0.0) {
                        enqueue(encodeIntervalDouble(KIND_CALORIES, now - 60_000L, now, point.value))
                    }
                }
                for (point in dataPoints.getData(DataType.DISTANCE)) {
                    if (point.value > 0.0) {
                        enqueue(encodeIntervalDouble(KIND_DISTANCE, now - 60_000L, now, point.value))
                    }
                }
                for (point in dataPoints.getData(DataType.FLOORS)) {
                    if (point.value > 0.0) {
                        enqueue(encodeIntervalDouble(KIND_FLOORS, now - 60_000L, now, point.value))
                    }
                }
            }
        }

    override suspend fun invoke(method: Int, payload: ByteBuffer): ByteArray {
        if (closed) throw PluginException(PluginException.CANCELLED)
        return when (method) {
            0 -> capabilities()
            1 -> encodeU32(permissionState())
            2 -> encodeU32(permissionState())
            3 -> startPassive(readMask(payload))
            4 -> stopPassive()
            5 -> startLive(payload)
            6 -> stopLive()
            7 -> nextBatch()
            else -> throw PluginException(PluginException.UNAVAILABLE)
        }
    }

    override fun close() {
        closed = true
        stopSensorsLocked()
        synchronized(lock) {
            waiter?.cancel()
            waiter = null
        }
    }

    private fun isWatch(): Boolean =
        context.packageManager.hasSystemFeature(PackageManager.FEATURE_WATCH)

    private fun hasSamsungHealthSensorSdk(): Boolean =
        try {
            Class.forName("com.samsung.android.service.health.tracking.HealthTrackingService")
            true
        } catch (_: ClassNotFoundException) {
            false
        }

    private fun capabilities(): ByteArray {
        val watch = isWatch()
        val samsung = hasSamsungHealthSensorSdk()
        var mask = 0
        if (sensorManager?.getDefaultSensor(Sensor.TYPE_HEART_RATE) != null || watch) {
            mask = mask or (1 shl 0)
        }
        if (sensorManager?.getDefaultSensor(Sensor.TYPE_STEP_COUNTER) != null || watch) {
            mask = mask or (1 shl 4)
        }
        if (watch) {
            mask = mask or (1 shl 5) or (1 shl 6) or (1 shl 7)
        }
        val offBody = sensorManager?.getDefaultSensor(Sensor.TYPE_LOW_LATENCY_OFFBODY_DETECT) != null
        if (offBody) mask = mask or (1 shl 8)
        if (samsung) {
            // Samsung Galaxy Watch Health Sensor SDK adds IBI/HRV, SpO2, skin temp, BIA.
            mask = mask or (1 shl 1) or (1 shl 2) or (1 shl 3) or (1 shl 10)
        }
        var flags = 0
        if (watch) flags = flags or 1 or 2
        if (samsung) flags = flags or 4
        if (offBody) flags = flags or 16
        return ByteBuffer
            .allocate(12)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(if (watch) 1 else 0)
            .putInt(mask)
            .putInt(flags)
            .array()
    }

    private fun permissionState(): Int {
        if (!isWatch()) return 4
        val body = ContextCompat.checkSelfPermission(context, Manifest.permission.BODY_SENSORS) ==
            PackageManager.PERMISSION_GRANTED
        if (!body) return 3
        val background =
            if (Build.VERSION.SDK_INT >= 33) {
                ContextCompat.checkSelfPermission(
                    context,
                    "android.permission.BODY_SENSORS_BACKGROUND",
                ) == PackageManager.PERMISSION_GRANTED
            } else {
                true
            }
        return if (background) 2 else 1
    }

    private suspend fun startPassive(mask: Int): ByteArray {
        if (!isWatch()) throw PluginException(PluginException.UNAVAILABLE)
        val types = mutableSetOf<DataType<*, *>>()
        if (mask and (1 shl 0) != 0) types += DataType.HEART_RATE_BPM
        if (mask and (1 shl 4) != 0) types += DataType.STEPS
        if (mask and (1 shl 5) != 0) types += DataType.FLOORS
        if (mask and (1 shl 6) != 0) types += DataType.CALORIES
        if (mask and (1 shl 7) != 0) types += DataType.DISTANCE
        if (types.isNotEmpty()) {
            val client = HealthServices.getClient(context).passiveMonitoringClient
            val config = PassiveListenerConfig.builder().setDataTypes(types).build()
            client.setPassiveListenerCallback(config, passiveCallback)
        }
        passiveRunning = true
        return ByteArray(0)
    }

    private suspend fun stopPassive(): ByteArray {
        if (passiveRunning) {
            runCatching {
                HealthServices
                    .getClient(context)
                    .passiveMonitoringClient
                    .clearPassiveListenerCallbackAsync()
                    .awaitResult()
            }
            passiveRunning = false
        }
        return ByteArray(0)
    }

    private suspend fun startLive(payload: ByteBuffer): ByteArray {
        val buf = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        if (buf.remaining() != 12) throw PluginException(PluginException.INVALID_ARGUMENT)
        val mask = buf.int
        val intervalMs = buf.int
        buf.int // activity hint
        if (mask == 0 || intervalMs < 200) throw PluginException(PluginException.INVALID_ARGUMENT)

        val delayUs = intervalMs * 1000
        if (mask and (1 shl 0) != 0) {
            sensorManager?.getDefaultSensor(Sensor.TYPE_HEART_RATE)?.let {
                sensorManager.registerListener(sensorListener, it, delayUs)
            }
            if (isWatch()) {
                runCatching {
                    HealthServices.getClient(context).measureClient
                        .registerMeasureCallback(DataType.HEART_RATE_BPM, measureCallback)
                }
            }
        }
        if (mask and (1 shl 4) != 0) {
            sensorManager?.getDefaultSensor(Sensor.TYPE_STEP_COUNTER)?.let {
                sensorManager.registerListener(sensorListener, it, delayUs)
            }
        }
        if (mask and (1 shl 8) != 0) {
            sensorManager?.getDefaultSensor(Sensor.TYPE_LOW_LATENCY_OFFBODY_DETECT)?.let {
                sensorManager.registerListener(sensorListener, it, SensorManager.SENSOR_DELAY_NORMAL)
            }
        }
        liveRunning = true
        return ByteArray(0)
    }

    private suspend fun stopLive(): ByteArray {
        stopSensorsLocked()
        if (liveRunning && isWatch()) {
            runCatching {
                HealthServices.getClient(context).measureClient
                    .unregisterMeasureCallbackAsync(DataType.HEART_RATE_BPM, measureCallback)
                    .awaitResult()
            }
        }
        liveRunning = false
        return ByteArray(0)
    }

    private fun stopSensorsLocked() {
        sensorManager?.unregisterListener(sensorListener)
    }

    private suspend fun nextBatch(): ByteArray {
        val immediate = drainLocked()
        if (immediate != null) return immediate
        val pending = CompletableDeferred<ByteArray>()
        synchronized(lock) {
            val again = drainLocked()
            if (again != null) return again
            waiter = pending
        }
        return pending.await()
    }

    private fun enqueue(encodedReading: ByteArray) {
        synchronized(lock) {
            if (buffer.size >= MAX_BATCH) {
                buffer.removeFirst()
                dropped += 1
            }
            buffer.addLast(encodedReading)
            val w = waiter
            if (w != null) {
                waiter = null
                val batch = drainLocked()
                if (batch != null) w.complete(batch)
            }
        }
    }

    private fun drainLocked(): ByteArray? =
        synchronized(lock) {
            if (buffer.isEmpty() && dropped == 0) return@synchronized null
            val out = ByteArrayOutputStream()
            writeU32(out, dropped)
            writeU32(out, buffer.size)
            dropped = 0
            while (buffer.isNotEmpty()) {
                out.write(buffer.removeFirst())
            }
            out.toByteArray()
        }

    private fun readMask(payload: ByteBuffer): Int {
        val buf = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        if (buf.remaining() != 4) throw PluginException(PluginException.INVALID_ARGUMENT)
        val mask = buf.int
        if (mask == 0) throw PluginException(PluginException.INVALID_ARGUMENT)
        return mask
    }

    private companion object {
        const val MAX_BATCH = 128
        const val KIND_HR = 0
        const val KIND_STEPS = 4
        const val KIND_FLOORS = 5
        const val KIND_CALORIES = 6
        const val KIND_DISTANCE = 7
        const val KIND_WEAR = 8
        const val ACCURACY_HIGH = 4

        fun mapAccuracy(accuracy: Int): Int =
            when (accuracy) {
                SensorManager.SENSOR_STATUS_NO_CONTACT -> 1
                SensorManager.SENSOR_STATUS_ACCURACY_LOW -> 2
                SensorManager.SENSOR_STATUS_ACCURACY_MEDIUM -> 3
                SensorManager.SENSOR_STATUS_ACCURACY_HIGH -> 4
                else -> 0
            }

        fun encodeU32(value: Int): ByteArray =
            ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN).putInt(value).array()

        fun writeU32(out: ByteArrayOutputStream, value: Int) {
            out.write(value and 0xFF)
            out.write((value shr 8) and 0xFF)
            out.write((value shr 16) and 0xFF)
            out.write((value shr 24) and 0xFF)
        }

        fun encodeHeartRate(timeMs: Long, bpm: Double, accuracy: Int): ByteArray =
            ByteBuffer
                .allocate(24)
                .order(ByteOrder.LITTLE_ENDIAN)
                .putInt(KIND_HR)
                .putLong(timeMs)
                .putDouble(bpm)
                .putInt(accuracy)
                .array()

        fun encodeSteps(startMs: Long, endMs: Long, count: Long): ByteArray =
            ByteBuffer
                .allocate(28)
                .order(ByteOrder.LITTLE_ENDIAN)
                .putInt(KIND_STEPS)
                .putLong(startMs)
                .putLong(endMs)
                .putLong(count)
                .array()

        fun encodeIntervalDouble(kind: Int, startMs: Long, endMs: Long, value: Double): ByteArray =
            ByteBuffer
                .allocate(28)
                .order(ByteOrder.LITTLE_ENDIAN)
                .putInt(kind)
                .putLong(startMs)
                .putLong(endMs)
                .putDouble(value)
                .array()

        fun encodeWearState(timeMs: Long, onWrist: Boolean): ByteArray =
            ByteBuffer
                .allocate(16)
                .order(ByteOrder.LITTLE_ENDIAN)
                .putInt(KIND_WEAR)
                .putLong(timeMs)
                .putInt(if (onWrist) 1 else 0)
                .array()
    }
}

private suspend fun <T> ListenableFuture<T>.awaitResult(): T =
    suspendCancellableCoroutine { cont ->
        addListener(
            {
                if (!cont.isActive) return@addListener
                try {
                    cont.resume(get())
                } catch (error: Throwable) {
                    cont.resumeWithException(error)
                }
            },
            { command -> command.run() },
        )
        cont.invokeOnCancellation { cancel(false) }
    }
