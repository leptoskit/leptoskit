package dev.leptoskit.plugins.background

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

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
        if (!BackgroundState.bootRestart) return

        val service = Intent(context, BackgroundService::class.java)

        if (android.os.Build.VERSION.SDK_INT >= 26) {
            context.startForegroundService(service)
        } else {
            context.startService(service)
        }
    }
}
