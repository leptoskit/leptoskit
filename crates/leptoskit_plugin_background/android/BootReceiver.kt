package dev.leptoskit.plugins.background

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.os.Build

/**
 * Opt-in: restarts the foreground service after a reboot. The manifest declares
 * the receiver with a `BOOT_COMPLETED` intent filter; the app leaves it out of
 * `native.plugins` when it does not want to survive a reboot.
 */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(
        context: Context,
        intent: Intent,
    ) {
        if (intent.action != Intent.ACTION_BOOT_COMPLETED) return

        val persisted = BackgroundConfigStore.load(context)
        val hasPermissions =
            persisted != null &&
                BackgroundService.hasRequiredPermissions(
                    context,
                    persisted.types,
                    Build.VERSION.SDK_INT,
                )
        val decision =
            decideBootAction(
                bootRestartEnabled = BackgroundState.bootRestart,
                persistedConfig = persisted,
                hasRequiredPermissions = hasPermissions,
                sdkInt = Build.VERSION.SDK_INT,
            )

        val config =
            when (decision) {
                is BootDecision.StartService -> decision.config
                BootDecision.DoNothing -> return
            }

        val service = BackgroundService.startIntent(context, config, fromBackground = true)

        try {
            if (Build.VERSION.SDK_INT >= 26) {
                context.startForegroundService(service)
            } else {
                context.startService(service)
            }
        } catch (_: Exception) {
            // Android 14+ and 15+ restrict starting several foreground service
            // types from BOOT_COMPLETED; schedule the periodic worker instead
            // of crashing the app at boot.
            try {
                BackgroundWorker.schedule(context)
            } catch (_: Exception) {
                // Leave the app running without crashing if WorkManager cannot be scheduled.
            }
        }
    }
}
