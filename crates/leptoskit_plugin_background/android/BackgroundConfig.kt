package dev.leptoskit.plugins.background

// Pure foreground-service type mapping, configuration encoding, and restart
// decisions. Nothing here touches Android, so `kotlinc` can run
// `BackgroundConfigTest.kt` against it on a desktop JVM.

internal const val MAX_TITLE_BYTES = 128
internal const val MAX_TEXT_BYTES = 1024
internal const val MIN_INTERVAL_SECS = 1
internal const val MAX_INTERVAL_SECS = 86_400

// Mirrors `ForegroundServiceType` in `crates/leptoskit_plugin_background/src/lib.rs`.
internal const val CRATE_TYPE_DATA_SYNC = 1 shl 0
internal const val CRATE_TYPE_HEALTH = 1 shl 1
internal const val CRATE_TYPE_MICROPHONE = 1 shl 2
internal const val CRATE_TYPE_LOCATION = 1 shl 3
internal const val CRATE_TYPE_CONNECTED_DEVICE = 1 shl 4
internal const val ALL_CRATE_TYPES =
    CRATE_TYPE_DATA_SYNC or
        CRATE_TYPE_HEALTH or
        CRATE_TYPE_MICROPHONE or
        CRATE_TYPE_LOCATION or
        CRATE_TYPE_CONNECTED_DEVICE

// Matches `android.content.pm.ServiceInfo.FOREGROUND_SERVICE_TYPE_*`.
internal const val ANDROID_TYPE_DATA_SYNC = 1
internal const val ANDROID_TYPE_LOCATION = 8
internal const val ANDROID_TYPE_CONNECTED_DEVICE = 16
internal const val ANDROID_TYPE_MICROPHONE = 128
internal const val ANDROID_TYPE_HEALTH = 256

/** Foreground-service configuration shared by the plugin, service, worker, and boot receiver. */
internal data class BackgroundConfig(
    val title: String,
    val text: String,
    val types: Int,
    val intervalSecs: Int,
) {
    /** Whether all four fields satisfy the plugin's wire contract. */
    fun isValid(): Boolean =
        isValidText(title, MAX_TITLE_BYTES, required = true) &&
            isValidText(text, MAX_TEXT_BYTES, required = false) &&
            (types and ALL_CRATE_TYPES.inv()) == 0 &&
            intervalSecs in MIN_INTERVAL_SECS..MAX_INTERVAL_SECS
}

/** Pure key/value encoding for persisting [BackgroundConfig] in `SharedPreferences`. */
internal object BackgroundConfigCodec {
    const val KEY_TITLE = "title"
    const val KEY_TEXT = "text"
    const val KEY_TYPES = "types"
    const val KEY_INTERVAL = "interval"

    fun encode(config: BackgroundConfig): Map<String, Any> =
        mapOf(
            KEY_TITLE to config.title,
            KEY_TEXT to config.text,
            KEY_TYPES to config.types,
            KEY_INTERVAL to config.intervalSecs,
        )

    fun decode(entries: Map<String, *>): BackgroundConfig? {
        val title = entries[KEY_TITLE] as? String ?: return null
        val text = entries[KEY_TEXT] as? String ?: return null
        val types = entries[KEY_TYPES] as? Int ?: return null
        val interval = entries[KEY_INTERVAL] as? Int ?: return null
        val config = BackgroundConfig(title, text, types, interval)
        return if (config.isValid()) config else null
    }
}

/**
 * Converts the crate's `ForegroundServiceTypes` bitmask to Android's
 * `ServiceInfo.FOREGROUND_SERVICE_TYPE_*` bitmask for [sdkInt], dropping any
 * type whose constant did not exist on that OS version (`dataSync`, `location`,
 * and `connectedDevice` require API 29; `microphone` requires API 30; `health`
 * requires API 34).
 */
internal fun mapForegroundServiceTypes(
    crateMask: Int,
    sdkInt: Int,
): Int {
    if (sdkInt < 29) return 0

    var mapped = 0
    if (crateMask and CRATE_TYPE_DATA_SYNC != 0) mapped = mapped or ANDROID_TYPE_DATA_SYNC
    if (crateMask and CRATE_TYPE_LOCATION != 0) mapped = mapped or ANDROID_TYPE_LOCATION
    if (crateMask and CRATE_TYPE_CONNECTED_DEVICE != 0) mapped = mapped or ANDROID_TYPE_CONNECTED_DEVICE
    if (sdkInt >= 30 && crateMask and CRATE_TYPE_MICROPHONE != 0) mapped = mapped or ANDROID_TYPE_MICROPHONE
    if (sdkInt >= 34 && crateMask and CRATE_TYPE_HEALTH != 0) mapped = mapped or ANDROID_TYPE_HEALTH
    return mapped
}

/**
 * Whether `BackgroundService` can legally call `startForeground` with [crateMask]
 * on [sdkInt]. Android 14 (API 34+) prohibits starting a foreground service with
 * type `none` (0).
 */
internal fun canStartForegroundWithTypes(
    crateMask: Int,
    sdkInt: Int,
): Boolean = sdkInt < 34 || mapForegroundServiceTypes(crateMask, sdkInt) != 0

/** What `BackgroundService.onStartCommand` received from Android. */
internal sealed interface StartIntent {
    /** An explicit `BackgroundPlugin.start` intent carrying configuration extras. */
    data class FromPlugin(
        val config: BackgroundConfig,
    ) : StartIntent

    /**
     * A restart intent from `BackgroundWorker` or `BootReceiver` carrying
     * configuration extras. Treated as stale if `stop()` cleared the persisted
     * store before `onStartCommand` ran.
     */
    data class FromBackground(
        val config: BackgroundConfig,
    ) : StartIntent

    /** A non-null intent without configuration extras. */
    data object Bare : StartIntent

    /** A null intent delivered by Android's `START_STICKY` restart. */
    data object Null : StartIntent
}

/** Action `BackgroundService.onStartCommand` should take. */
internal sealed interface StartCommandDecision {
    /** Run (or update) the foreground service with [config] and mapped [androidTypes]. */
    data class StartForeground(
        val config: BackgroundConfig,
        val androidTypes: Int,
    ) : StartCommandDecision

    /**
     * Satisfy Android's `startForegroundService` promise with [config] and
     * [androidTypes], then immediately leave the foreground and stop without
     * rescheduling periodic work. Used when a background restart races with
     * `stop()`.
     */
    data class StopAfterForeground(
        val config: BackgroundConfig,
        val androidTypes: Int,
    ) : StartCommandDecision

    /** Stop the service (`stopSelf()`) and return `START_NOT_STICKY`. */
    data object Stop : StartCommandDecision
}

/**
 * Decides how `BackgroundService.onStartCommand` handles [incoming].
 *
 * - Explicit `BackgroundPlugin.start` intents (`FromPlugin`) use the intent's
 *   configuration.
 * - Background restart intents (`FromBackground`) honour `stop()`: if nothing
 *   valid is persisted, they satisfy Android's foreground promise and stop
 *   immediately without rescheduling work; otherwise they use the latest
 *   persisted configuration.
 * - Bare and null intents fall back to [persistedConfig], stopping cleanly when
 *   nothing valid is persisted.
 * - Never permits `startForeground` with a zero type mask on API 34+.
 */
internal fun decideStartCommand(
    incoming: StartIntent,
    persistedConfig: BackgroundConfig?,
    sdkInt: Int,
): StartCommandDecision =
    when (incoming) {
        is StartIntent.FromPlugin -> {
            startForegroundOrStop(incoming.config, sdkInt)
        }

        is StartIntent.FromBackground -> {
            val active = persistedConfig?.takeIf { it.isValid() }
            if (active == null) {
                StartCommandDecision.StopAfterForeground(
                    config = incoming.config,
                    androidTypes = foregroundTypesForPromise(incoming.config.types, sdkInt),
                )
            } else {
                val androidTypes = mapForegroundServiceTypes(active.types, sdkInt)
                if (sdkInt >= 34 && androidTypes == 0) {
                    StartCommandDecision.StopAfterForeground(
                        config = active,
                        androidTypes = mapForegroundServiceTypes(CRATE_TYPE_DATA_SYNC, sdkInt),
                    )
                } else {
                    StartCommandDecision.StartForeground(active, androidTypes)
                }
            }
        }

        StartIntent.Bare, StartIntent.Null -> {
            val active = persistedConfig?.takeIf { it.isValid() }
            if (active == null) {
                StartCommandDecision.Stop
            } else {
                startForegroundOrStop(active, sdkInt)
            }
        }
    }

private fun startForegroundOrStop(
    config: BackgroundConfig,
    sdkInt: Int,
): StartCommandDecision {
    if (!config.isValid()) return StartCommandDecision.Stop
    val androidTypes = mapForegroundServiceTypes(config.types, sdkInt)
    if (sdkInt >= 34 && androidTypes == 0) return StartCommandDecision.Stop
    return StartCommandDecision.StartForeground(config, androidTypes)
}

private fun foregroundTypesForPromise(
    crateMask: Int,
    sdkInt: Int,
): Int {
    val mapped = mapForegroundServiceTypes(crateMask, sdkInt)
    return if (sdkInt >= 34 && mapped == 0) {
        mapForegroundServiceTypes(CRATE_TYPE_DATA_SYNC, sdkInt)
    } else {
        mapped
    }
}

/** Action to take after `startForeground` throws `SecurityException` or `IllegalStateException`. */
internal sealed interface ForegroundStartFallback {
    /** Retry `startForeground` once with `dataSync` alone. */
    data class RetryWithDataSync(
        val crateTypes: Int,
        val androidTypes: Int,
    ) : ForegroundStartFallback

    /** Stop the service and cancel periodic work while keeping the saved configuration. */
    data object StopAndCancelWork : ForegroundStartFallback
}

/**
 * Decides whether a failed `startForeground` call should retry once with
 * `dataSync` alone or abort. `dataSync` is always declared in the plugin's
 * manifest metadata and has no runtime permission prerequisite; we only retry
 * when `dataSync` alone was not what just failed.
 */
internal fun decideForegroundStartFallback(
    attemptedAndroidTypes: Int,
    sdkInt: Int,
): ForegroundStartFallback {
    if (sdkInt < 29) return ForegroundStartFallback.StopAndCancelWork
    val dataSyncAndroidTypes = mapForegroundServiceTypes(CRATE_TYPE_DATA_SYNC, sdkInt)
    return if (attemptedAndroidTypes == dataSyncAndroidTypes) {
        ForegroundStartFallback.StopAndCancelWork
    } else {
        ForegroundStartFallback.RetryWithDataSync(
            crateTypes = CRATE_TYPE_DATA_SYNC,
            androidTypes = dataSyncAndroidTypes,
        )
    }
}

/** Action `BackgroundService.onTimeout(startId, fgsType)` should take on API 35+. */
internal data class TimeoutDecision(
    val stopForeground: Boolean,
    val stopService: Boolean,
    val clearPersistedConfig: Boolean,
    val cancelPeriodicWork: Boolean,
)

/**
 * Decides how `BackgroundService.onTimeout(startId, fgsType)` responds when
 * Android expires a time-limited foreground service type (such as `dataSync`
 * after ~6 hours on Android 15+): leave the foreground and stop cleanly while
 * keeping the saved configuration and periodic work so the app stays woken.
 */
internal fun decideTimeout(
    @Suppress("UNUSED_PARAMETER") fgsType: Int,
): TimeoutDecision =
    TimeoutDecision(
        stopForeground = true,
        stopService = true,
        clearPersistedConfig = false,
        cancelPeriodicWork = false,
    )

/** Action `BackgroundWorker.doWork` should take. */
internal sealed interface WorkerDecision {
    /** Wake the app without starting the service. */
    data object WakeOnly : WorkerDecision

    /** Start the service with [config] extras. */
    data class StartService(
        val config: BackgroundConfig,
    ) : WorkerDecision

    /** Do nothing and keep the periodic work enqueued. */
    data object DoNothing : WorkerDecision
}

/**
 * Decides what `BackgroundWorker.doWork` should do. Never cancels periodic work
 * (only `stop()` or an unrecoverable `startForeground` failure does), and never
 * calls `startForegroundService` unless a valid configuration is saved, its
 * mapped type is valid on [sdkInt], and [hasRequiredPermissions] is true.
 */
internal fun decideWorkerAction(
    serviceRunning: Boolean,
    persistedConfig: BackgroundConfig?,
    hasRequiredPermissions: Boolean,
    sdkInt: Int,
): WorkerDecision {
    if (serviceRunning) return WorkerDecision.WakeOnly
    val config = persistedConfig?.takeIf { it.isValid() } ?: return WorkerDecision.DoNothing
    if (!canStartForegroundWithTypes(config.types, sdkInt)) return WorkerDecision.WakeOnly
    if (!hasRequiredPermissions) return WorkerDecision.WakeOnly
    return WorkerDecision.StartService(config)
}

/** Action `BootReceiver.onReceive` should take on `BOOT_COMPLETED`. */
internal sealed interface BootDecision {
    /** Attempt to start the service with [config] extras. */
    data class StartService(
        val config: BackgroundConfig,
    ) : BootDecision

    /** Leave the service stopped. */
    data object DoNothing : BootDecision
}

/**
 * Decides what `BootReceiver.onReceive` should do. Never requests starting the
 * service unless boot restart is enabled, a valid configuration is persisted,
 * `startForeground` can legally run on [sdkInt], and [hasRequiredPermissions]
 * is true.
 */
internal fun decideBootAction(
    bootRestartEnabled: Boolean,
    persistedConfig: BackgroundConfig?,
    hasRequiredPermissions: Boolean,
    sdkInt: Int,
): BootDecision {
    if (!bootRestartEnabled) return BootDecision.DoNothing
    val config = persistedConfig?.takeIf { it.isValid() } ?: return BootDecision.DoNothing
    if (!canStartForegroundWithTypes(config.types, sdkInt)) return BootDecision.DoNothing
    if (!hasRequiredPermissions) return BootDecision.DoNothing
    return BootDecision.StartService(config)
}

private fun isValidText(
    value: String,
    maxBytes: Int,
    required: Boolean,
): Boolean =
    value.toByteArray(Charsets.UTF_8).size <= maxBytes &&
        (!required || value.isNotBlank()) &&
        value.none { it.isISOControl() }
