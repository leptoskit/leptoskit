package dev.leptoskit.plugins.background

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper

/**
 * The foreground service that keeps the app's process alive. It holds a
 * persistent low-importance notification and wakes the app's `AppService` on a
 * configurable interval.
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

    override fun onStartCommand(
        intent: Intent?,
        flags: Int,
        startId: Int,
    ): Int {
        val decision =
            decideStartCommand(
                incoming = classifyIntent(intent),
                persistedConfig = BackgroundConfigStore.load(this),
                sdkInt = Build.VERSION.SDK_INT,
            )

        val start =
            when (decision) {
                is StartCommandDecision.StartForeground -> {
                    decision
                }

                StartCommandDecision.Stop -> {
                    started = false
                    BackgroundState.running = false
                    BackgroundState.serviceTypes = 0
                    handler.removeCallbacks(wakeTick)
                    stopSelf()
                    return START_NOT_STICKY
                }
            }

        val previousIntervalMs = intervalMs
        title = start.config.title
        text = start.config.text
        types = start.config.types
        intervalMs = start.config.intervalSecs.toLong() * 1_000L

        ensureNotificationChannel(this, title)
        startAsForeground(start.androidTypes)
        BackgroundState.running = true
        BackgroundState.serviceTypes = types

        if (!started) {
            started = true
            handler.postDelayed(wakeTick, intervalMs)
            BackgroundWorker.schedule(this)
        } else if (previousIntervalMs != intervalMs) {
            handler.removeCallbacks(wakeTick)
            handler.postDelayed(wakeTick, intervalMs)
        }

        return START_STICKY
    }

    private fun startAsForeground(androidTypes: Int) {
        val notification = buildNotification()

        if (Build.VERSION.SDK_INT >= 29) {
            startForeground(BackgroundState.NOTIFICATION_ID, notification, androidTypes)
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
        BackgroundState.serviceTypes = 0
        handler.removeCallbacks(wakeTick)
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    companion object {
        /** Builds an explicit intent carrying all four configuration extras. */
        internal fun startIntent(
            context: Context,
            config: BackgroundConfig,
        ): Intent =
            Intent(context, BackgroundService::class.java).apply {
                putExtra(BackgroundState.EXTRA_TITLE, config.title)
                putExtra(BackgroundState.EXTRA_TEXT, config.text)
                putExtra(BackgroundState.EXTRA_TYPES, config.types)
                putExtra(BackgroundState.EXTRA_INTERVAL, config.intervalSecs)
            }

        /**
         * Extracts configuration from [intent] only when it explicitly carries
         * the configuration extras; returns `null` for a null or bare intent.
         */
        internal fun configFromIntent(intent: Intent?): BackgroundConfig? {
            if (intent == null) return null
            if (
                !intent.hasExtra(BackgroundState.EXTRA_TITLE) ||
                !intent.hasExtra(BackgroundState.EXTRA_TYPES) ||
                !intent.hasExtra(BackgroundState.EXTRA_INTERVAL)
            ) {
                return null
            }

            val title = intent.getStringExtra(BackgroundState.EXTRA_TITLE) ?: return null
            val text = intent.getStringExtra(BackgroundState.EXTRA_TEXT) ?: ""
            val types = intent.getIntExtra(BackgroundState.EXTRA_TYPES, -1)
            val interval = intent.getIntExtra(BackgroundState.EXTRA_INTERVAL, 0)
            val config = BackgroundConfig(title, text, types, interval)
            return if (config.isValid()) config else null
        }

        /** Classifies an incoming `onStartCommand` intent without defaulting missing extras to 0. */
        internal fun classifyIntent(intent: Intent?): StartIntent {
            if (intent == null) return StartIntent.Null
            val config = configFromIntent(intent) ?: return StartIntent.Bare
            return StartIntent.WithConfig(config)
        }

        internal fun ensureNotificationChannel(
            context: Context,
            title: String,
        ) {
            val manager = context.getSystemService(NotificationManager::class.java) ?: return

            if (Build.VERSION.SDK_INT >= 26) {
                val channel =
                    NotificationChannel(
                        BackgroundState.CHANNEL_ID,
                        title,
                        NotificationManager.IMPORTANCE_LOW,
                    ).apply {
                        description = "Background work"
                        setShowBadge(false)
                    }
                manager.createNotificationChannel(channel)
            }
        }
    }
}
