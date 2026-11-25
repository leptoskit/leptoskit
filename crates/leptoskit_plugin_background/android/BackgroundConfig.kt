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
    /** An intent explicitly carrying configuration extras. */
    data class WithConfig(
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

    /** Stop the service (`stopSelf()`) and return `START_NOT_STICKY`. */
    data object Stop : StartCommandDecision
}

/**
 * Decides how `BackgroundService.onStartCommand` handles [incoming].
 *
 * Uses the intent's configuration only when it explicitly carries one;
 * otherwise falls back to [persistedConfig]. Never permits `startForeground`
 * with a zero type mask on API 34+, and stops cleanly when nothing valid is
 * configured.
 */
internal fun decideStartCommand(
    incoming: StartIntent,
    persistedConfig: BackgroundConfig?,
    sdkInt: Int,
): StartCommandDecision {
    val config =
        when (incoming) {
            is StartIntent.WithConfig -> incoming.config
            StartIntent.Bare, StartIntent.Null -> persistedConfig
        } ?: return StartCommandDecision.Stop

    if (!config.isValid()) return StartCommandDecision.Stop

    val androidTypes = mapForegroundServiceTypes(config.types, sdkInt)
    if (sdkInt >= 34 && androidTypes == 0) return StartCommandDecision.Stop

    return StartCommandDecision.StartForeground(config, androidTypes)
}

/** Action `BackgroundWorker.doWork` should take. */
internal sealed interface WorkerDecision {
    /** The service is already running; wake the app without starting the service again. */
    data object WakeOnly : WorkerDecision

    /** Start the service with [config] extras. */
    data class StartService(
        val config: BackgroundConfig,
    ) : WorkerDecision

    /** Nothing valid is persisted (or it cannot start on [sdkInt]); cancel the periodic work. */
    data object CancelWork : WorkerDecision
}

/**
 * Decides what `BackgroundWorker.doWork` should do. Never requests starting the
 * service unless it will be able to call `startForeground` with a valid type.
 */
internal fun decideWorkerAction(
    serviceRunning: Boolean,
    persistedConfig: BackgroundConfig?,
    sdkInt: Int,
): WorkerDecision {
    if (serviceRunning) return WorkerDecision.WakeOnly
    val config = persistedConfig?.takeIf { it.isValid() } ?: return WorkerDecision.CancelWork
    if (!canStartForegroundWithTypes(config.types, sdkInt)) return WorkerDecision.CancelWork
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
 * and `startForeground` can legally run on [sdkInt].
 */
internal fun decideBootAction(
    bootRestartEnabled: Boolean,
    persistedConfig: BackgroundConfig?,
    sdkInt: Int,
): BootDecision {
    if (!bootRestartEnabled) return BootDecision.DoNothing
    val config = persistedConfig?.takeIf { it.isValid() } ?: return BootDecision.DoNothing
    if (!canStartForegroundWithTypes(config.types, sdkInt)) return BootDecision.DoNothing
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
