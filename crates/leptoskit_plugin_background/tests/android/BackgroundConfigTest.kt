package dev.leptoskit.plugins.background

// Pure JVM tests for foreground-service type mapping, configuration encoding,
// fallback/timeout policy, and service/worker/boot restart decisions. Run with
// `kotlinc` against `BackgroundConfig.kt` with no Android runtime.

fun main() {
    maskMappingChecks()
    configRoundTripChecks()
    startCommandDecisionChecks()
    foregroundStartFallbackChecks()
    timeoutDecisionChecks()
    workerDecisionChecks()
    bootDecisionChecks()
    println("Background config, mask mapping, fallback, timeout, and restart decision checks passed")
}

private fun maskMappingChecks() {
    // Zero stays zero across every API level.
    for (sdk in listOf(26, 28, 29, 30, 33, 34, 35, 36)) {
        check(mapForegroundServiceTypes(0, sdk) == 0) { "Expected 0 at API $sdk" }
    }

    // Individual crate bits map to Android's ServiceInfo constants on API 34+.
    check(mapForegroundServiceTypes(CRATE_TYPE_DATA_SYNC, 34) == ANDROID_TYPE_DATA_SYNC)
    check(mapForegroundServiceTypes(CRATE_TYPE_HEALTH, 34) == ANDROID_TYPE_HEALTH)
    check(mapForegroundServiceTypes(CRATE_TYPE_MICROPHONE, 34) == ANDROID_TYPE_MICROPHONE)
    check(mapForegroundServiceTypes(CRATE_TYPE_LOCATION, 34) == ANDROID_TYPE_LOCATION)
    check(mapForegroundServiceTypes(CRATE_TYPE_CONNECTED_DEVICE, 34) == ANDROID_TYPE_CONNECTED_DEVICE)

    // Exact Android ServiceInfo numeric values: health=256 (not 2), microphone=128 (not 4).
    check(mapForegroundServiceTypes(CRATE_TYPE_DATA_SYNC, 36) == 1)
    check(mapForegroundServiceTypes(CRATE_TYPE_HEALTH, 36) == 256)
    check(mapForegroundServiceTypes(CRATE_TYPE_MICROPHONE, 36) == 128)
    check(mapForegroundServiceTypes(CRATE_TYPE_LOCATION, 36) == 8)
    check(mapForegroundServiceTypes(CRATE_TYPE_CONNECTED_DEVICE, 36) == 16)

    // Pre-API 29 has no foregroundServiceType constants at all.
    check(mapForegroundServiceTypes(ALL_CRATE_TYPES, 26) == 0)
    check(mapForegroundServiceTypes(ALL_CRATE_TYPES, 28) == 0)

    // API 29: dataSync, location, connectedDevice exist; microphone (API 30) and health (API 34) drop.
    check(
        mapForegroundServiceTypes(ALL_CRATE_TYPES, 29) ==
            (ANDROID_TYPE_DATA_SYNC or ANDROID_TYPE_LOCATION or ANDROID_TYPE_CONNECTED_DEVICE),
    )
    check(mapForegroundServiceTypes(CRATE_TYPE_MICROPHONE, 29) == 0)
    check(mapForegroundServiceTypes(CRATE_TYPE_HEALTH, 29) == 0)

    // API 30..33: microphone is included; health (API 34) still drops.
    for (sdk in listOf(30, 31, 33)) {
        check(mapForegroundServiceTypes(CRATE_TYPE_MICROPHONE, sdk) == ANDROID_TYPE_MICROPHONE)
        check(mapForegroundServiceTypes(CRATE_TYPE_HEALTH, sdk) == 0)
        check(
            mapForegroundServiceTypes(ALL_CRATE_TYPES, sdk) ==
                (
                    ANDROID_TYPE_DATA_SYNC or
                        ANDROID_TYPE_LOCATION or
                        ANDROID_TYPE_CONNECTED_DEVICE or
                        ANDROID_TYPE_MICROPHONE
                ),
        )
    }

    // API 34+: every declared type maps, including combinations.
    check(
        mapForegroundServiceTypes(ALL_CRATE_TYPES, 34) ==
            (
                ANDROID_TYPE_DATA_SYNC or
                    ANDROID_TYPE_HEALTH or
                    ANDROID_TYPE_MICROPHONE or
                    ANDROID_TYPE_LOCATION or
                    ANDROID_TYPE_CONNECTED_DEVICE
            ),
    )
    check(
        mapForegroundServiceTypes(CRATE_TYPE_HEALTH or CRATE_TYPE_MICROPHONE, 34) ==
            (ANDROID_TYPE_HEALTH or ANDROID_TYPE_MICROPHONE),
    )
    check(
        mapForegroundServiceTypes(CRATE_TYPE_DATA_SYNC or CRATE_TYPE_LOCATION, 35) ==
            (ANDROID_TYPE_DATA_SYNC or ANDROID_TYPE_LOCATION),
    )

    // Unknown bits are dropped instead of passed straight through to Android.
    check(mapForegroundServiceTypes(1 shl 5, 34) == 0)
    check(mapForegroundServiceTypes((1 shl 5) or CRATE_TYPE_DATA_SYNC, 34) == ANDROID_TYPE_DATA_SYNC)
    check(mapForegroundServiceTypes(Int.MIN_VALUE, 34) == 0)

    // API 34+ requires a non-zero mapped mask; earlier versions accept 0.
    check(canStartForegroundWithTypes(0, 28))
    check(canStartForegroundWithTypes(0, 29))
    check(canStartForegroundWithTypes(0, 33))
    check(!canStartForegroundWithTypes(0, 34))
    check(!canStartForegroundWithTypes(0, 36))
    check(canStartForegroundWithTypes(CRATE_TYPE_HEALTH, 33))
    check(canStartForegroundWithTypes(CRATE_TYPE_HEALTH, 34))
    check(canStartForegroundWithTypes(CRATE_TYPE_MICROPHONE, 34))
}

private fun configRoundTripChecks() {
    val unicode =
        BackgroundConfig(
            title = "Quant is running ✓",
            text = "Draining the spool",
            types = CRATE_TYPE_DATA_SYNC or CRATE_TYPE_HEALTH,
            intervalSecs = 45,
        )
    check(unicode.isValid())
    val encoded = BackgroundConfigCodec.encode(unicode)
    check(BackgroundConfigCodec.decode(encoded) == unicode)

    val emptyText =
        BackgroundConfig(
            title = "Sync",
            text = "",
            types = CRATE_TYPE_DATA_SYNC,
            intervalSecs = MIN_INTERVAL_SECS,
        )
    check(BackgroundConfigCodec.decode(BackgroundConfigCodec.encode(emptyText)) == emptyText)

    val maxBounds =
        BackgroundConfig(
            title = "t".repeat(MAX_TITLE_BYTES),
            text = "x".repeat(MAX_TEXT_BYTES),
            types = ALL_CRATE_TYPES,
            intervalSecs = MAX_INTERVAL_SECS,
        )
    check(BackgroundConfigCodec.decode(BackgroundConfigCodec.encode(maxBounds)) == maxBounds)

    // Missing or wrong-typed keys decode to null.
    check(BackgroundConfigCodec.decode(emptyMap<String, Any>()) == null)
    for (key in listOf(
        BackgroundConfigCodec.KEY_TITLE,
        BackgroundConfigCodec.KEY_TEXT,
        BackgroundConfigCodec.KEY_TYPES,
        BackgroundConfigCodec.KEY_INTERVAL,
    )) {
        check(BackgroundConfigCodec.decode(encoded - key) == null) { "Missing $key should fail" }
    }
    check(BackgroundConfigCodec.decode(encoded + (BackgroundConfigCodec.KEY_TYPES to "1")) == null)
    check(BackgroundConfigCodec.decode(encoded + (BackgroundConfigCodec.KEY_INTERVAL to 60L)) == null)

    // Out-of-domain values decode to null.
    val invalidConfigs =
        listOf(
            unicode.copy(title = "   "),
            unicode.copy(title = "a".repeat(MAX_TITLE_BYTES + 1)),
            unicode.copy(title = "bad\ntitle"),
            unicode.copy(text = "bad\u0000text"),
            unicode.copy(text = "x".repeat(MAX_TEXT_BYTES + 1)),
            unicode.copy(types = 1 shl 5),
            unicode.copy(types = -1),
            unicode.copy(intervalSecs = 0),
            unicode.copy(intervalSecs = MAX_INTERVAL_SECS + 1),
        )
    for (bad in invalidConfigs) {
        check(!bad.isValid()) { "Expected invalid config: $bad" }
        check(BackgroundConfigCodec.decode(BackgroundConfigCodec.encode(bad)) == null)
    }
}

private fun startCommandDecisionChecks() {
    val persisted =
        BackgroundConfig(
            title = "Persisted",
            text = "Working",
            types = CRATE_TYPE_HEALTH or CRATE_TYPE_LOCATION,
            intervalSecs = 30,
        )
    val explicit =
        BackgroundConfig(
            title = "Explicit",
            text = "Updated",
            types = CRATE_TYPE_MICROPHONE,
            intervalSecs = 15,
        )

    // Explicit BackgroundPlugin.start intent takes precedence over persisted config.
    check(
        decideStartCommand(StartIntent.FromPlugin(explicit), persisted, 34) ==
            StartCommandDecision.StartForeground(explicit, ANDROID_TYPE_MICROPHONE),
    )
    check(
        decideStartCommand(StartIntent.FromPlugin(explicit), null, 36) ==
            StartCommandDecision.StartForeground(explicit, ANDROID_TYPE_MICROPHONE),
    )

    // Worker/boot restart with persisted config uses the latest persisted config.
    check(
        decideStartCommand(StartIntent.FromBackground(explicit), persisted, 34) ==
            StartCommandDecision.StartForeground(
                persisted,
                ANDROID_TYPE_HEALTH or ANDROID_TYPE_LOCATION,
            ),
    )

    // Worker/boot restart racing with `stop()` (empty store): satisfy the foreground
    // promise and stop immediately without rescheduling periodic work.
    check(
        decideStartCommand(StartIntent.FromBackground(explicit), null, 34) ==
            StartCommandDecision.StopAfterForeground(explicit, ANDROID_TYPE_MICROPHONE),
    )
    check(
        decideStartCommand(StartIntent.FromBackground(explicit), null, 36) ==
            StartCommandDecision.StopAfterForeground(explicit, ANDROID_TYPE_MICROPHONE),
    )
    // On API 34+, a background start with zero types still gets a valid dataSync type
    // to satisfy the foreground promise before immediately stopping.
    val zeroType = persisted.copy(types = 0)
    check(
        decideStartCommand(StartIntent.FromBackground(zeroType), null, 34) ==
            StartCommandDecision.StopAfterForeground(zeroType, ANDROID_TYPE_DATA_SYNC),
    )
    check(
        decideStartCommand(StartIntent.FromBackground(zeroType), null, 33) ==
            StartCommandDecision.StopAfterForeground(zeroType, 0),
    )

    // Bare intent (no extras) preserves persisted configuration instead of zeroing types.
    check(
        decideStartCommand(StartIntent.Bare, persisted, 34) ==
            StartCommandDecision.StartForeground(
                persisted,
                ANDROID_TYPE_HEALTH or ANDROID_TYPE_LOCATION,
            ),
    )
    // On API 33, health drops while location stays.
    check(
        decideStartCommand(StartIntent.Bare, persisted, 33) ==
            StartCommandDecision.StartForeground(persisted, ANDROID_TYPE_LOCATION),
    )

    // Null intent (START_STICKY restart) restores persisted configuration.
    check(
        decideStartCommand(StartIntent.Null, persisted, 36) ==
            StartCommandDecision.StartForeground(
                persisted,
                ANDROID_TYPE_HEALTH or ANDROID_TYPE_LOCATION,
            ),
    )

    // Bare intent with nothing persisted stops cleanly and never calls startForeground.
    check(decideStartCommand(StartIntent.Bare, null, 34) == StartCommandDecision.Stop)
    check(decideStartCommand(StartIntent.Bare, null, 33) == StartCommandDecision.Stop)

    // Null intent with nothing persisted calls stopSelf() and returns START_NOT_STICKY.
    check(decideStartCommand(StartIntent.Null, null, 34) == StartCommandDecision.Stop)
    check(decideStartCommand(StartIntent.Null, null, 29) == StartCommandDecision.Stop)

    // Zero type mask is rejected on API 34+, permitted on API < 34.
    check(decideStartCommand(StartIntent.FromPlugin(zeroType), null, 34) == StartCommandDecision.Stop)
    check(decideStartCommand(StartIntent.Bare, zeroType, 34) == StartCommandDecision.Stop)
    check(decideStartCommand(StartIntent.Null, zeroType, 36) == StartCommandDecision.Stop)
    check(
        decideStartCommand(StartIntent.FromPlugin(zeroType), null, 33) ==
            StartCommandDecision.StartForeground(zeroType, 0),
    )
    check(
        decideStartCommand(StartIntent.FromPlugin(zeroType), null, 28) ==
            StartCommandDecision.StartForeground(zeroType, 0),
    )
}

private fun foregroundStartFallbackChecks() {
    // Non-dataSync failures retry once with dataSync alone (both crate and Android masks).
    for (failedAndroidTypes in listOf(
        ANDROID_TYPE_LOCATION,
        ANDROID_TYPE_HEALTH,
        ANDROID_TYPE_MICROPHONE,
        ANDROID_TYPE_CONNECTED_DEVICE,
        ANDROID_TYPE_DATA_SYNC or ANDROID_TYPE_LOCATION,
        ANDROID_TYPE_DATA_SYNC or ANDROID_TYPE_HEALTH,
    )) {
        check(
            decideForegroundStartFallback(failedAndroidTypes, 34) ==
                ForegroundStartFallback.RetryWithDataSync(
                    crateTypes = CRATE_TYPE_DATA_SYNC,
                    androidTypes = ANDROID_TYPE_DATA_SYNC,
                ),
        )
        check(
            decideForegroundStartFallback(failedAndroidTypes, 35) ==
                ForegroundStartFallback.RetryWithDataSync(
                    crateTypes = CRATE_TYPE_DATA_SYNC,
                    androidTypes = ANDROID_TYPE_DATA_SYNC,
                ),
        )
    }

    // When dataSync alone was already tried (e.g. API 35 6-hour limit exhausted or
    // background start forbidden), do not retry: stop and cancel periodic work.
    for (sdk in listOf(29, 30, 33, 34, 35, 36)) {
        check(
            decideForegroundStartFallback(ANDROID_TYPE_DATA_SYNC, sdk) ==
                ForegroundStartFallback.StopAndCancelWork,
        )
    }

    // On API 33, DATA_SYNC | HEALTH mapped to ANDROID_TYPE_DATA_SYNC alone, so it does not retry.
    val api33DataSyncAndHealth = mapForegroundServiceTypes(CRATE_TYPE_DATA_SYNC or CRATE_TYPE_HEALTH, 33)
    check(
        decideForegroundStartFallback(api33DataSyncAndHealth, 33) ==
            ForegroundStartFallback.StopAndCancelWork,
    )

    // Pre-API 29 has no type mask on startForeground, so there is no fallback type to retry.
    check(decideForegroundStartFallback(0, 28) == ForegroundStartFallback.StopAndCancelWork)
    check(decideForegroundStartFallback(0, 26) == ForegroundStartFallback.StopAndCancelWork)
}

private fun timeoutDecisionChecks() {
    val expected =
        TimeoutDecision(
            stopForeground = true,
            stopService = true,
            clearPersistedConfig = false,
            cancelPeriodicWork = false,
        )
    check(decideTimeout(ANDROID_TYPE_DATA_SYNC) == expected)
    check(decideTimeout(0) == expected)
}

private fun workerDecisionChecks() {
    val persisted =
        BackgroundConfig(
            title = "Sync",
            text = "Background",
            types = CRATE_TYPE_LOCATION,
            intervalSecs = 60,
        )

    // Service already running: only wake, never start the service again.
    check(
        decideWorkerAction(
            serviceRunning = true,
            persistedConfig = persisted,
            hasRequiredPermissions = true,
            sdkInt = 34,
        ) == WorkerDecision.WakeOnly,
    )
    check(
        decideWorkerAction(
            serviceRunning = true,
            persistedConfig = null,
            hasRequiredPermissions = false,
            sdkInt = 34,
        ) == WorkerDecision.WakeOnly,
    )

    // Service not running with valid persisted configuration and permissions granted: restart.
    check(
        decideWorkerAction(
            serviceRunning = false,
            persistedConfig = persisted,
            hasRequiredPermissions = true,
            sdkInt = 34,
        ) == WorkerDecision.StartService(persisted),
    )
    check(
        decideWorkerAction(
            serviceRunning = false,
            persistedConfig = persisted,
            hasRequiredPermissions = true,
            sdkInt = 36,
        ) == WorkerDecision.StartService(persisted),
    )

    // Service not running with missing runtime permissions: skip start (wake only), keep work.
    check(
        decideWorkerAction(
            serviceRunning = false,
            persistedConfig = persisted,
            hasRequiredPermissions = false,
            sdkInt = 34,
        ) == WorkerDecision.WakeOnly,
    )
    check(
        decideWorkerAction(
            serviceRunning = false,
            persistedConfig = persisted,
            hasRequiredPermissions = false,
            sdkInt = 36,
        ) == WorkerDecision.WakeOnly,
    )

    // Service not running with nothing persisted: do nothing and keep the periodic work.
    check(
        decideWorkerAction(
            serviceRunning = false,
            persistedConfig = null,
            hasRequiredPermissions = false,
            sdkInt = 34,
        ) == WorkerDecision.DoNothing,
    )
    check(
        decideWorkerAction(
            serviceRunning = false,
            persistedConfig = null,
            hasRequiredPermissions = true,
            sdkInt = 29,
        ) == WorkerDecision.DoNothing,
    )

    // Service not running with a zero-type config on API 34+: skip start (wake only), keep work.
    val zeroType = persisted.copy(types = 0)
    check(
        decideWorkerAction(
            serviceRunning = false,
            persistedConfig = zeroType,
            hasRequiredPermissions = true,
            sdkInt = 34,
        ) == WorkerDecision.WakeOnly,
    )
    check(
        decideWorkerAction(
            serviceRunning = false,
            persistedConfig = zeroType,
            hasRequiredPermissions = true,
            sdkInt = 33,
        ) == WorkerDecision.StartService(zeroType),
    )
}

private fun bootDecisionChecks() {
    val persisted =
        BackgroundConfig(
            title = "Sync",
            text = "After reboot",
            types = CRATE_TYPE_CONNECTED_DEVICE,
            intervalSecs = 60,
        )

    check(
        decideBootAction(
            bootRestartEnabled = false,
            persistedConfig = persisted,
            hasRequiredPermissions = true,
            sdkInt = 35,
        ) == BootDecision.DoNothing,
    )
    check(
        decideBootAction(
            bootRestartEnabled = true,
            persistedConfig = null,
            hasRequiredPermissions = true,
            sdkInt = 35,
        ) == BootDecision.DoNothing,
    )
    check(
        decideBootAction(
            bootRestartEnabled = true,
            persistedConfig = persisted,
            hasRequiredPermissions = false,
            sdkInt = 35,
        ) == BootDecision.DoNothing,
    )
    check(
        decideBootAction(
            bootRestartEnabled = true,
            persistedConfig = persisted,
            hasRequiredPermissions = true,
            sdkInt = 35,
        ) == BootDecision.StartService(persisted),
    )
    check(
        decideBootAction(
            bootRestartEnabled = true,
            persistedConfig = persisted.copy(types = 0),
            hasRequiredPermissions = true,
            sdkInt = 34,
        ) == BootDecision.DoNothing,
    )
    check(
        decideBootAction(
            bootRestartEnabled = true,
            persistedConfig = persisted.copy(types = 0),
            hasRequiredPermissions = true,
            sdkInt = 33,
        ) == BootDecision.StartService(persisted.copy(types = 0)),
    )
}
