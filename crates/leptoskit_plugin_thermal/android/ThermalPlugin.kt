package dev.leptoskit.plugins.thermal

import android.content.Context
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.PowerManager
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CancellableContinuation
import kotlinx.coroutines.suspendCancellableCoroutine
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.coroutines.resume

/** Two little-endian u32 fields: the shared state and Android's own status. */
internal const val SNAPSHOT_BYTES = 8

/** `PowerManager` thermal status first appears at this API level. */
private const val THERMAL_API = 29

/** Shared state codes, in Rust's `State` order. */
internal const val STATE_UNKNOWN = 0
internal const val STATE_NOMINAL = 1
internal const val STATE_FAIR = 2
internal const val STATE_SERIOUS = 3
internal const val STATE_CRITICAL = 4

/** The reserved maximum stands in for an Android status the device cannot report. */
internal const val NO_STATUS = -1

/** `PowerManager.THERMAL_STATUS_SHUTDOWN`, the last status. */
private const val LAST_STATUS = 6

/** Method identifiers. */
private const val SNAPSHOT = 0
private const val WAIT_FOR_CHANGE = 1

/** Thermal status from `PowerManager`, with one demand-driven change waiter. */
class ThermalPlugin(
    context: Context,
) : NativePlugin {
    private val context = context.applicationContext
    private val handler = Handler(Looper.getMainLooper())
    private var waiter: CancellableContinuation<ByteArray>? = null
    private var previous = byteArrayOf()
    private var listener: PowerManager.OnThermalStatusChangedListener? = null
    private var closed = false
    override val name = "thermal"
    override val methods = intArrayOf(SNAPSHOT, WAIT_FOR_CHANGE)

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        check(Looper.myLooper() == Looper.getMainLooper())

        if (closed) throw PluginException(PluginException.CANCELLED)

        return when (method) {
            SNAPSHOT -> currentSnapshot(payload)
            WAIT_FOR_CHANGE -> awaitChange(payload)
            else -> throw PluginException(PluginException.UNAVAILABLE)
        }
    }

    override fun close() {
        check(Looper.myLooper() == Looper.getMainLooper())
        closed = true
        val pending = waiter
        cleanup()
        pending?.cancel()
    }

    /** Answers one reading; the snapshot call takes no arguments. */
    private fun currentSnapshot(payload: ByteBuffer): ByteArray {
        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

        return encodeSnapshot(status())
    }

    /** Suspends until the OS reports a status other than the caller's last one. */
    private suspend fun awaitChange(payload: ByteBuffer): ByteArray {
        val expected = validateSnapshot(payload)

        // Before API 29 there is nothing to observe, so a wait would never end.
        if (Build.VERSION.SDK_INT < THERMAL_API) throw PluginException(PluginException.UNAVAILABLE)

        val power = powerManager() ?: throw PluginException(PluginException.UNAVAILABLE)

        if (waiter?.isActive == false) cleanup()

        if (waiter != null) throw PluginException(PluginException.BUSY)

        return suspendCancellableCoroutine { continuation ->
            waiter = continuation
            previous = expected
            register(power)
            continuation.invokeOnCancellation { release(continuation) }
            changed()
        }
    }

    /** Listens on the main executor; the OS also calls back once with the current status. */
    private fun register(power: PowerManager) {
        if (Build.VERSION.SDK_INT < THERMAL_API) return

        val observer = PowerManager.OnThermalStatusChangedListener { changed() }
        listener = observer
        power.addThermalStatusListener(context.mainExecutor, observer)
    }

    /**
     * Drops the listener for a cancelled caller. The cancellation callback can
     * run off the main thread, so it only acts when this continuation is still
     * the one waiting.
     */
    private fun release(continuation: CancellableContinuation<ByteArray>) {
        val remove =
            Runnable {
                if (waiter === continuation) cleanup()
            }

        if (Looper.myLooper() == Looper.getMainLooper()) remove.run() else handler.post(remove)
    }

    /** Resumes the waiter once, and only when the reading actually differs. */
    private fun changed() {
        val pending = waiter ?: return
        val value = encodeSnapshot(status())

        if (!value.contentEquals(previous)) {
            cleanup()
            pending.resume(value)
        }
    }

    private fun cleanup() {
        waiter = null
        previous = byteArrayOf()
        val observer = listener
        listener = null

        if (observer != null && Build.VERSION.SDK_INT >= THERMAL_API) {
            powerManager()?.removeThermalStatusListener(observer)
        }
    }

    /** The current `PowerManager` status, or unknown before API 29. */
    private fun status(): Int {
        if (Build.VERSION.SDK_INT < THERMAL_API) return NO_STATUS

        return powerManager()?.currentThermalStatus ?: NO_STATUS
    }

    private fun powerManager(): PowerManager? = context.getSystemService(Context.POWER_SERVICE) as? PowerManager
}

/** Folds Android's seven statuses into the four shared levels; the Rust crate uses the same table. */
internal fun stateFor(status: Int): Int =
    when (status) {
        PowerManager.THERMAL_STATUS_NONE -> STATE_NOMINAL

        PowerManager.THERMAL_STATUS_LIGHT,
        PowerManager.THERMAL_STATUS_MODERATE,
        -> STATE_FAIR

        PowerManager.THERMAL_STATUS_SEVERE -> STATE_SERIOUS

        PowerManager.THERMAL_STATUS_CRITICAL,
        PowerManager.THERMAL_STATUS_EMERGENCY,
        PowerManager.THERMAL_STATUS_SHUTDOWN,
        -> STATE_CRITICAL

        else -> STATE_UNKNOWN
    }

/** Encodes a status; one this host doesn't know stays unknown in both fields. */
internal fun encodeSnapshot(status: Int): ByteArray {
    val known = status in 0..LAST_STATUS

    return ByteBuffer
        .allocate(SNAPSHOT_BYTES)
        .order(ByteOrder.LITTLE_ENDIAN)
        .putInt(if (known) stateFor(status) else STATE_UNKNOWN)
        .putInt(if (known) status else NO_STATUS)
        .array()
}

/** Echoes the caller's last reading back, refusing pairs the Rust decoder would refuse. */
internal fun validateSnapshot(payload: ByteBuffer): ByteArray {
    if (payload.remaining() != SNAPSHOT_BYTES) throw PluginException(PluginException.INVALID_ARGUMENT)

    val bytes = ByteArray(SNAPSHOT_BYTES).also { payload.get(it) }
    val fields = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
    val state = fields.int
    val status = fields.int
    val consistent = if (status == NO_STATUS) state in STATE_UNKNOWN..STATE_CRITICAL else stateFor(status) == state

    if ((status != NO_STATUS && status !in 0..LAST_STATUS) || !consistent) {
        throw PluginException(PluginException.INVALID_ARGUMENT)
    }

    return bytes
}
