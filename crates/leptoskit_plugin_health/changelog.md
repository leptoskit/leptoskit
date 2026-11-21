# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-21)

### Features

#### Wearable and smartwatch support across watch OSes and BLE

_Owner:_ Ifiok Jr. · _Introduced in:_ [5b1f3ba](https://github.com/leptoskit/leptoskit/commit/5b1f3ba88e456a3bd5b427603bda36022781444d)

Adds sensor-first wearable application APIs, watch host packaging, and wearable biometric/companion/BLE plugins:

- **`leptoskit_watch`**: adds `WatchKind::HarmonyOs`, Samsung Galaxy Watch and Huawei Watch viewport presets, and the sensor-first `WearableApp` API (`WearableStream`, `SamplingPolicy`, `DeliveryMode`, `WearableStatus`, `WearableSurfaceMode::AutoStatus` / `Custom`, and per-OS `required_permissions()`).
- **`leptoskit_plugin_health`**: adds `SkinTemperature`, `ActiveCalories`, `Distance`, and `FloorsClimbed`; enables `HealthKit` on watchOS alongside iOS; adds HarmonyOS `HealthPlugin.ts` (`@kit.HealthServiceKit`).
- **`leptoskit_plugin_wearable`**: new direct on-watch live & passive biometric sensor plugin covering Wear OS Health Services (`PassiveMonitoringClient`, `MeasureClient`, `SensorManager`, plus optional Samsung Health Sensor SDK runtime hook), watchOS (`HKWorkoutSession`, `HKLiveWorkoutBuilder`, `HKObserverQuery`, `CMPedometer`), and HarmonyOS Wearable (`@kit.SensorServiceKit`).
- **`leptoskit_plugin_watch_sync`**: new bidirectional phone ↔ watch companion sync plugin over Apple `WatchConnectivity`, Android/Wear OS Wearable Data & Message Layer, and Huawei Wear Engine.
- **`leptoskit_plugin_ble`**: new BLE GATT central plugin (`CoreBluetooth`, Android/Wear OS `BluetoothGatt`, HarmonyOS `@kit.ConnectivityKit`) with pure-Rust Bluetooth SIG decoders for Heart Rate + RR-intervals/RMSSD, Pulse Oximeter (`SpO2`), Health Thermometer, Battery, Running Speed & Cadence, and Cycling Power.
- **HarmonyOS plugins**: adds ArkTS implementations for `battery`, `thermal`, and `pedometer`.
- **`leptoskit_config` & `leptoskit_cli`**: adds `[native.wearos]`, `[native.watchos]`, and `[native.harmonyos]` configuration tables; upgrades `wearos` to build Android APKs with Wear OS manifest transformation (watch feature, standalone metadata, tiles/complications); upgrades `watchos` to generate watchOS Xcode build files and invoke `xcodebuild`; updates HarmonyOS host generation to emit configured `deviceTypes` (including `"wearable"`) and `requestPermissions`.

### Fixes

#### Fix wearable plugin SwiftPM manifests and Android await

_Owner:_ Ifiok Jr. · _Introduced in:_ [69d0cb2](https://github.com/leptoskit/leptoskit/commit/69d0cb211dfc8d7ae4ee2be28cc8f476d491555b)

- Added missing `Package.swift` manifests to `leptoskit_plugin_ble`, `leptoskit_plugin_watch_sync`, and `leptoskit_plugin_wearable` so iOS, macOS, and watchOS hosts resolve their Swift packages.
- Fixed Swift 6 strict concurrency (`MainActor.assumeIsolated`), exclusivity in `WireReader`, and HealthKit `appleSleepingWristTemperature` switch expressions.
- Replaced external `kotlinx-coroutines-play-services` and `kotlinx-coroutines-guava` `.await()` calls in `WatchSyncPlugin.kt` and `WearablePlugin.kt` with self-contained `suspendCancellableCoroutine` helpers and moved `deferred.await()` outside `synchronized`.

- **Report HealthKit read permissions as unknown rather than deriving an exact read mask from write authorization.** An empty inexact mask must not prevent an app from reading records the owner has authorized. _Owner:_ Ifiok Jr. · _Introduced in:_ [ee9473c](https://github.com/leptoskit/leptoskit/commit/ee9473c2325dd58283fb6215310f114cd336bd1f)
