package dev.leptoskit.plugins.background

import android.app.Notification
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper

/**
 * The foreground service that keeps the app's process alive. It holds a
 * persistent low-importance notification and wakes the app's `AppService` on a
 * configurable interval. The service type mask mirrors the crate's bitfield:
 * dataSync=1, health=2, microphone=4, location=8, connectedDevice=16.
 */
class BackgroundService : Service() {
    private val handler = Handler(Looper.getMainLooper())
    private var title = "App is running"
    private var text = ""
    private var intervalMs = 60_000L
    private var types = 0
    private var started = false

    private val wakeTick =
        object : Runnable {
            override fun run() {
                if (!started) return
                BackgroundState.wake()
                handler.postDelayed(this, intervalMs)
            }
        }

    override fun onCreate() {
        super.onCreate()
        BackgroundState.running = true
    }

    override fun onStartCommand(
        intent: Intent?,
        flags: Int,
        startId: Int,
    ): Int {
        if (intent != null) {
            title = intent.getStringExtra(BackgroundState.EXTRA_TITLE) ?: title
            text = intent.getStringExtra(BackgroundState.EXTRA_TEXT) ?: ""
            types = intent.getIntExtra(BackgroundState.EXTRA_TYPES, 0)
            intervalMs = intent.getLongExtra(BackgroundState.EXTRA_INTERVAL, 60L) * 1000
        }

        startAsForeground()
        BackgroundState.serviceTypes = types

        if (!started) {
            started = true
            handler.postDelayed(wakeTick, intervalMs)
            BackgroundWorker.schedule(this)
        }

        return START_STICKY
    }

    private fun startAsForeground() {
        val notification = buildNotification()

        if (Build.VERSION.SDK_INT >= 34 && types != 0) {
            startForeground(BackgroundState.NOTIFICATION_ID, notification, types)
        } else if (Build.VERSION.SDK_INT >= 29) {
            startForeground(BackgroundState.NOTIFICATION_ID, notification, types)
        } else {
            startForeground(BackgroundState.NOTIFICATION_ID, notification)
        }
    }

    private fun buildNotification(): Notification {
        val builder =
            Notification
                .Builder(this, BackgroundState.CHANNEL_ID)
                .setSmallIcon(android.R.drawable.ic_dialog_info)
                .setContentTitle(title)
                .setContentText(text)
                .setOngoing(true)
                .setCategory(Notification.CATEGORY_SERVICE)
                .setPriority(Notification.PRIORITY_LOW)

        if (Build.VERSION.SDK_INT >= 26) {
            builder.setChannelId(BackgroundState.CHANNEL_ID)
        }

        return builder.build()
    }

    override fun onDestroy() {
        started = false
        BackgroundState.running = false
        handler.removeCallbacks(wakeTick)
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null
}
