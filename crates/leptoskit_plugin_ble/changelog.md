# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-25)

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

- **Fix Swift 6 delegate isolation in BlePlugin.swift.** Adopted `@preconcurrency CBCentralManagerDelegate` and `@preconcurrency CBPeripheralDelegate` on `@MainActor BlePlugin` (whose central manager runs on `.main`) to resolve Swift 6 `sending` data-race errors on `CBPeripheral`. _Owner:_ Ifiok Jr. · _Introduced in:_ [73b6c7e](https://github.com/leptoskit/leptoskit/commit/73b6c7ec3c8dd3f823034658eb34f4342a2dd580)

#### Fix wearable plugin SwiftPM manifests and Android await

_Owner:_ Ifiok Jr. · _Introduced in:_ [69d0cb2](https://github.com/leptoskit/leptoskit/commit/69d0cb211dfc8d7ae4ee2be28cc8f476d491555b)

- Added missing `Package.swift` manifests to `leptoskit_plugin_ble`, `leptoskit_plugin_watch_sync`, and `leptoskit_plugin_wearable` so iOS, macOS, and watchOS hosts resolve their Swift packages.
- Fixed Swift 6 strict concurrency (`MainActor.assumeIsolated`), exclusivity in `WireReader`, and HealthKit `appleSleepingWristTemperature` switch expressions.
- Replaced external `kotlinx-coroutines-play-services` and `kotlinx-coroutines-guava` `.await()` calls in `WatchSyncPlugin.kt` and `WearablePlugin.kt` with self-contained `suspendCancellableCoroutine` helpers and moved `deferred.await()` outside `synchronized`.

- **Fix wearable wire codecs, watch plugins, and host builds.** Fixes cross-platform wire mismatches in Health (`ABSENT_LENGTH`, `ANDROID_TYPES` / `HARMONY_TYPES` bitmask preserving `HRV_RMSSD`, and exercise `Activity` codes 8..14), HarmonyOS Pedometer (`bounded_reply` 36-byte framing) and Battery charge state mapping, and BLE 16-bit/128-bit UUID normalization. Fixes Apple/Android runtime bugs across `ble`, `watch-sync`, `wearable`, and watchOS `haptics`, adds `Reading::Ecg` for `SensorMask::ECG`, fixes `RotaryState` wrap-around detent math, adds watchOS 10 support to `LeptosKitBridge` and wearable SwiftPM manifests, enables native plugin resolution and unified output paths for `wearos`, `watchos`, and `harmonyos` in `leptoskit_cli`, and adds the wearable crates to CI unit tests. _Owner:_ Ifiok Jr. · _Introduced in:_ [21a5f55](https://github.com/leptoskit/leptoskit/commit/21a5f55fea25e9c1f043da51aae77bcc6296aeb3)
