package dev.leptoskit.plugins.backgroundtask

import android.content.Context
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.Worker
import androidx.work.WorkerParameters
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.util.concurrent.TimeUnit

/**
 * Registers named periodic WorkManager jobs. Distinct from the `background`
 * plugin's foreground-service AppService keep-alive surface.
 */
class BackgroundTaskPlugin(
    private val hostContext: Context,
) : NativePlugin {
    override val name: String = "background-task"
    override val methods: IntArray = intArrayOf(0, 1, 2, 3)

    private val tasks = linkedMapOf<String, Int>()
    private var closed = false

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (closed) throw PluginException(PluginException.CANCELLED)
        return when (method) {
            0 -> {
                if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
                BackgroundTaskWire.status(Status.Available)
            }

            1 -> {
                val request = BackgroundTaskWire.register(payload)
                val work =
                    PeriodicWorkRequestBuilder<NoopWorker>(
                        request.intervalSecs.toLong(),
                        TimeUnit.SECONDS,
                    ).build()
                WorkManager
                    .getInstance(hostContext)
                    .enqueueUniquePeriodicWork(
                        request.name,
                        ExistingPeriodicWorkPolicy.UPDATE,
                        work,
                    )
                tasks[request.name] = request.intervalSecs
                ByteArray(0)
            }

            2 -> {
                val name = BackgroundTaskWire.name(payload)
                WorkManager.getInstance(hostContext).cancelUniqueWork(name)
                tasks.remove(name)
                ByteArray(0)
            }

            3 -> {
                val name = BackgroundTaskWire.name(payload)
                if (!tasks.containsKey(name)) throw PluginException(PluginException.INVALID_ARGUMENT)
                ByteArray(0)
            }

            else -> {
                throw PluginException(PluginException.UNAVAILABLE)
            }
        }
    }

    override fun close() {
        closed = true
        val manager = WorkManager.getInstance(hostContext)
        for (name in tasks.keys.toList()) {
            manager.cancelUniqueWork(name)
        }
        tasks.clear()
    }
}

/** Placeholder worker; guest Wasm execution while suspended is later scope. */
class NoopWorker(
    context: Context,
    params: WorkerParameters,
) : Worker(context, params) {
    override fun doWork(): Result = Result.success()
}
