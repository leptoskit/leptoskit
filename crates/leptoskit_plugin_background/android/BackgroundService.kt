package dev.leptoskit.plugins.background

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
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

        return when (decision) {
            is StartCommandDecision.StartForeground -> {
                handleStartForeground(decision)
            }

            is StartCommandDecision.StopAfterForeground -> {
                handleStopAfterForeground(decision)
            }

            StartCommandDecision.Stop -> {
                stopCleanly(leaveForeground = started, cancelWork = false)
                START_NOT_STICKY
            }
        }
    }

    override fun onTimeout(
        startId: Int,
        fgsType: Int,
    ) {
        if (Build.VERSION.SDK_INT >= 35) {
            super.onTimeout(startId, fgsType)
        }

        val decision = decideTimeout(fgsType)
        if (decision.clearPersistedConfig) {
            BackgroundConfigStore.clear(this)
        }

        stopCleanly(
            leaveForeground = decision.stopForeground,
            cancelWork = decision.cancelPeriodicWork,
            stopService = decision.stopService,
        )
    }

    private fun handleStartForeground(decision: StartCommandDecision.StartForeground): Int {
        val previousIntervalMs = intervalMs
        title = decision.config.title
        text = decision.config.text
        intervalMs = decision.config.intervalSecs.toLong() * 1_000L

        ensureNotificationChannel(this, title)
        val activeCrateTypes = startAsForegroundOrFallback(decision.config.types, decision.androidTypes)
        if (activeCrateTypes == null) {
            // Cancel periodic work so a persistent permission/quota failure is
            // not replayed every 15 minutes, while keeping the saved config so
            // the app's next foreground `start()` still works.
            stopCleanly(leaveForeground = started, cancelWork = true)
            return START_NOT_STICKY
        }

        BackgroundState.running = true
        BackgroundState.serviceTypes = activeCrateTypes

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

    private fun handleStopAfterForeground(decision: StartCommandDecision.StopAfterForeground): Int {
        if (decision.config.isValid()) {
            title = decision.config.title
            text = decision.config.text
        }

        ensureNotificationChannel(this, title)
        val enteredForeground = startAsForegroundOrFallback(decision.config.types, decision.androidTypes) != null
        stopCleanly(leaveForeground = enteredForeground || started, cancelWork = false)
        return START_NOT_STICKY
    }

    private fun stopCleanly(
        leaveForeground: Boolean,
        cancelWork: Boolean,
        stopService: Boolean = true,
    ) {
        started = false
        BackgroundState.running = false
        BackgroundState.serviceTypes = 0
        handler.removeCallbacks(wakeTick)

        if (cancelWork) {
            try {
                BackgroundWorker.cancel(this)
            } catch (_: Exception) {
                // Keep cleanup non-fatal if WorkManager is unavailable.
            }
        }

        if (leaveForeground) {
            stopForeground(STOP_FOREGROUND_REMOVE)
        }

        if (stopService) {
            stopSelf()
        }
    }

    /**
     * Calls `startForeground` with [androidTypes], retrying once with `dataSync`
     * alone on `SecurityException` or `IllegalStateException` when that was not
     * already attempted. Returns the crate bitmask actually running, or `null`
     * if nothing could be started.
     */
    private fun startAsForegroundOrFallback(
        requestedCrateTypes: Int,
        androidTypes: Int,
    ): Int? {
        val notification = buildNotification()
        if (tryStartForeground(notification, androidTypes)) {
            return requestedCrateTypes
        }

        return when (val fallback = decideForegroundStartFallback(androidTypes, Build.VERSION.SDK_INT)) {
            is ForegroundStartFallback.RetryWithDataSync -> {
                if (tryStartForeground(notification, fallback.androidTypes)) {
                    fallback.crateTypes
                } else {
                    null
                }
            }

            ForegroundStartFallback.StopAndCancelWork -> {
                null
            }
        }
    }

    private fun tryStartForeground(
        notification: Notification,
        androidTypes: Int,
    ): Boolean =
        try {
            if (Build.VERSION.SDK_INT >= 29) {
                startForeground(BackgroundState.NOTIFICATION_ID, notification, androidTypes)
            } else {
                startForeground(BackgroundState.NOTIFICATION_ID, notification)
            }
            true
        } catch (_: SecurityException) {
            false
        } catch (_: IllegalStateException) {
            false
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
        // Granular health permissions introduced in Android 16 (API 36) under
        // `android.health.connect.HealthPermissions`; kept as literals so older
        // OS versions never reference a class absent from their framework jar.
        private const val HEALTH_PERMISSION_HEART_RATE = "android.permission.health.READ_HEART_RATE"
        private const val HEALTH_PERMISSION_SKIN_TEMPERATURE = "android.permission.health.READ_SKIN_TEMPERATURE"
        private const val HEALTH_PERMISSION_OXYGEN_SATURATION = "android.permission.health.READ_OXYGEN_SATURATION"
        private const val PERMISSION_BODY_SENSORS = "android.permission.BODY_SENSORS"

        /** Builds an explicit intent carrying all four configuration extras and origin flag. */
        internal fun startIntent(
            context: Context,
            config: BackgroundConfig,
            fromBackground: Boolean = false,
        ): Intent =
            Intent(context, BackgroundService::class.java).apply {
                putExtra(BackgroundState.EXTRA_TITLE, config.title)
                putExtra(BackgroundState.EXTRA_TEXT, config.text)
                putExtra(BackgroundState.EXTRA_TYPES, config.types)
                putExtra(BackgroundState.EXTRA_INTERVAL, config.intervalSecs)
                putExtra(BackgroundState.EXTRA_FROM_BACKGROUND, fromBackground)
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
            val fromBackground = intent.getBooleanExtra(BackgroundState.EXTRA_FROM_BACKGROUND, false)
            return if (fromBackground) StartIntent.FromBackground(config) else StartIntent.FromPlugin(config)
        }

        /**
         * Checks via `checkSelfPermission` whether every requested foreground
         * service type in [crateMask] satisfies its documented runtime
         * permission prerequisites on [sdkInt].
         */
        internal fun hasRequiredPermissions(
            context: Context,
            crateMask: Int,
            sdkInt: Int = Build.VERSION.SDK_INT,
        ): Boolean {
            if (sdkInt < 34) return true

            if (crateMask and CRATE_TYPE_MICROPHONE != 0) {
                if (!hasPermission(context, Manifest.permission.RECORD_AUDIO)) return false
            }

            if (crateMask and CRATE_TYPE_LOCATION != 0) {
                val hasLocation =
                    hasPermission(context, Manifest.permission.ACCESS_FINE_LOCATION) ||
                        hasPermission(context, Manifest.permission.ACCESS_COARSE_LOCATION)
                if (!hasLocation) return false
            }

            if (crateMask and CRATE_TYPE_HEALTH != 0) {
                val hasHealth =
                    hasPermission(context, Manifest.permission.HIGH_SAMPLING_RATE_SENSORS) ||
                        hasPermission(context, Manifest.permission.ACTIVITY_RECOGNITION) ||
                        (sdkInt <= 35 && hasPermission(context, PERMISSION_BODY_SENSORS)) ||
                        hasPermission(context, HEALTH_PERMISSION_HEART_RATE) ||
                        hasPermission(context, HEALTH_PERMISSION_SKIN_TEMPERATURE) ||
                        hasPermission(context, HEALTH_PERMISSION_OXYGEN_SATURATION)
                if (!hasHealth) return false
            }

            if (crateMask and CRATE_TYPE_CONNECTED_DEVICE != 0) {
                val hasConnectedDevice =
                    hasPermission(context, Manifest.permission.BLUETOOTH_CONNECT) ||
                        hasPermission(context, Manifest.permission.BLUETOOTH_ADVERTISE) ||
                        hasPermission(context, Manifest.permission.BLUETOOTH_SCAN) ||
                        hasPermission(context, Manifest.permission.UWB_RANGING) ||
                        hasPermission(context, Manifest.permission.CHANGE_NETWORK_STATE) ||
                        hasPermission(context, Manifest.permission.CHANGE_WIFI_STATE) ||
                        hasPermission(context, Manifest.permission.CHANGE_WIFI_MULTICAST_STATE) ||
                        hasPermission(context, Manifest.permission.NFC) ||
                        hasPermission(context, Manifest.permission.TRANSMIT_IR)
                if (!hasConnectedDevice) return false
            }

            return true
        }

        private fun hasPermission(
            context: Context,
            permission: String,
        ): Boolean = context.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED

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
