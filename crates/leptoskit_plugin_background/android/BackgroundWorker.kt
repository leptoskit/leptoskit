package dev.leptoskit.plugins.background

import android.content.Context
import android.content.Intent
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.Worker
import androidx.work.WorkerParameters
import java.util.concurrent.TimeUnit

/**
 * WorkManager periodic fallback: every 15 minutes it restarts the foreground
 * service or wakes the app if the system killed the service. WorkManager is the
 * only fallback that survives Doze; it does not guarantee an exact cadence.
 */
class BackgroundWorker(
    context: Context,
    params: WorkerParameters,
) : Worker(context, params) {
    override fun doWork(): Result {
        val service = Intent(applicationContext, BackgroundService::class.java)

        try {
            if (android.os.Build.VERSION.SDK_INT >= 26) {
                applicationContext.startForegroundService(service)
            } else {
                applicationContext.startService(service)
            }
        } catch (_: Exception) {
            // The app may be under a background-start restriction; wake what is
            // already running instead of failing the work.
            BackgroundState.wake()
        }

        return Result.success()
    }

    companion object {
        fun schedule(context: Context) {
            val request =
                PeriodicWorkRequestBuilder<BackgroundWorker>(15, TimeUnit.MINUTES).build()

            WorkManager
                .getInstance(context)
                .enqueueUniquePeriodicWork(
                    BackgroundState.WORK_NAME,
                    ExistingPeriodicWorkPolicy.KEEP,
                    request,
                )
        }
    }
}
