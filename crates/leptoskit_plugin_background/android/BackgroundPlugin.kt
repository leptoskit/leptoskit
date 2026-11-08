package dev.leptoskit.plugins.background

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Looper
import android.os.PowerManager
import android.provider.Settings
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import dev.leptoskit.mobile.NativeServiceWakeBroker
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction

/** Shared, process-wide state for the foreground service and its plugin. */
internal object BackgroundState {
    const val CHANNEL_ID = "leptoskit.background"
    const val NOTIFICATION_ID = 1
    const val WORK_NAME = "leptoskit.background.work"

    const val EXTRA_TITLE = "title"
    const val EXTRA_TEXT = "text"
    const val EXTRA_TYPES = "types"
    const val EXTRA_INTERVAL = "interval"

    @Volatile
    var running = false

    @Volatile
    var lastWakeSecs = 0L

    @Volatile
    var bootRestart = false

    /** Posts a background tick to the running app and records the wake time. */
    fun wake() {
        lastWakeSecs = System.currentTimeMillis() / 1000
        NativeServiceWakeBroker.wakeBackground()
    }
}

/**
 * Keeps the app's process alive with a foreground service and wakes its Rust
 * `AppService` on a configurable schedule. Android 14 restricts which service
 * types may start from the background; see the crate README.
 */
class BackgroundPlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "background"
    override val methods = intArrayOf(START, STOP, STATUS, REQUEST_BATTERY)

    companion object {
        const val START = 0
        const val STOP = 1
        const val STATUS = 2
        const val REQUEST_BATTERY = 3

        const val MAX_TITLE = 128
        const val MAX_TEXT = 1024
    }

    private val manager =
        context.getSystemService(android.app.NotificationManager::class.java)
    private var closed = false

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        check(Looper.myLooper() == Looper.getMainLooper())
        currentCoroutineContext().ensureActive()

        if (closed) throw PluginException(PluginException.CANCELLED)

        return when (method) {
            START -> start(payload)
            STOP -> stop(payload)
            STATUS -> status(payload)
            REQUEST_BATTERY -> requestBattery(payload)
            else -> throw PluginException(PluginException.UNAVAILABLE)
        }
    }

    private fun start(payload: ByteBuffer): ByteArray {
        val input = Reader(payload, MAX_TITLE + MAX_TEXT + 16)
        val title = input.text(MAX_TITLE, true)
        val text = input.text(MAX_TEXT, false)
        val types = input.number()
        val interval = input.number()
        input.finish()

        if (interval !in 1..86_400) throw PluginException(PluginException.INVALID_ARGUMENT)
        configureChannel(title)

        val intent =
            Intent(context, BackgroundService::class.java).apply {
                putExtra(BackgroundState.EXTRA_TITLE, title)
                putExtra(BackgroundState.EXTRA_TEXT, text)
                putExtra(BackgroundState.EXTRA_TYPES, types)
                putExtra(BackgroundState.EXTRA_INTERVAL, interval)
            }

        if (Build.VERSION.SDK_INT >= 26) {
            context.startForegroundService(intent)
        } else {
            context.startService(intent)
        }

        return byteArrayOf()
    }

    private fun stop(payload: ByteBuffer): ByteArray {
        empty(payload)
        context.stopService(Intent(context, BackgroundService::class.java))

        return byteArrayOf()
    }

    private fun status(payload: ByteBuffer): ByteArray {
        empty(payload)

        return ByteBuffer
            .allocate(12)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(if (BackgroundState.running) 1 else 0)
            .putInt(BackgroundState.lastWakeSecs.toInt())
            .putInt(battery())
            .array()
    }

    private fun requestBattery(payload: ByteBuffer): ByteArray {
        empty(payload)

        if (battery() == 1) {
            return ByteBuffer
                .allocate(4)
                .order(ByteOrder.LITTLE_ENDIAN)
                .putInt(1)
                .array()
        }

        try {
            val intent =
                Intent(
                    Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS,
                    Uri.parse("package:${context.packageName}"),
                ).apply {
                    if (context !is Activity) addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                }
            context.startActivity(intent)
        } catch (_: Exception) {
            throw PluginException(PluginException.UNAVAILABLE)
        }

        return ByteBuffer
            .allocate(4)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(battery())
            .array()
    }

    override fun close() {
        closed = true
    }

    private fun configureChannel(title: String) {
        val manager = manager ?: return

        if (Build.VERSION.SDK_INT >= 26) {
            val channel =
                android.app
                    .NotificationChannel(
                        BackgroundState.CHANNEL_ID,
                        title,
                        android.app.NotificationManager.IMPORTANCE_LOW,
                    ).apply {
                        description = "Background work"
                        setShowBadge(false)
                    }
            manager.createNotificationChannel(channel)
        }
    }

    private fun battery(): Int {
        val manager = context.getSystemService(PowerManager::class.java) ?: return 0

        return if (
            Build.VERSION.SDK_INT >= 23 &&
            manager.isIgnoringBatteryOptimizations(context.packageName)
        ) {
            1
        } else {
            2
        }
    }

    private fun empty(payload: ByteBuffer) {
        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
    }
}

/** Strict little-endian reader shared by the plugin and its service. */
internal class Reader(
    payload: ByteBuffer,
    maximum: Int,
) {
    private val data = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)

    init {
        if (data.remaining() > maximum) throw PluginException(PluginException.INVALID_ARGUMENT)
    }

    fun number(): Int {
        if (data.remaining() < 4) throw PluginException(PluginException.INVALID_ARGUMENT)

        return data.int
    }

    fun text(
        maximum: Int,
        required: Boolean,
    ): String {
        val length = number()

        if (length !in 0..maximum || length > data.remaining()) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        val field = data.slice().apply { limit(length) }
        data.position(data.position() + length)
        val value =
            try {
                Charsets.UTF_8
                    .newDecoder()
                    .onMalformedInput(CodingErrorAction.REPORT)
                    .onUnmappableCharacter(CodingErrorAction.REPORT)
                    .decode(field)
                    .toString()
            } catch (_: CharacterCodingException) {
                throw PluginException(PluginException.INVALID_ARGUMENT)
            }

        if ((required && value.isBlank()) || value.any { it.isISOControl() }) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        return value
    }

    fun finish() {
        if (data.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
    }
}
