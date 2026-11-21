---
"leptoskit_plugin_ble": patch
"leptoskit_plugin_health": patch
"leptoskit_plugin_watch_sync": patch
"leptoskit_plugin_wearable": patch
---

# Fix wearable plugin SwiftPM manifests and Android await

- Added missing `Package.swift` manifests to `leptoskit_plugin_ble`, `leptoskit_plugin_watch_sync`, and `leptoskit_plugin_wearable` so iOS, macOS, and watchOS hosts resolve their Swift packages.
- Fixed Swift 6 strict concurrency (`MainActor.assumeIsolated`), exclusivity in `WireReader`, and HealthKit `appleSleepingWristTemperature` switch expressions.
- Replaced external `kotlinx-coroutines-play-services` and `kotlinx-coroutines-guava` `.await()` calls in `WatchSyncPlugin.kt` and `WearablePlugin.kt` with self-contained `suspendCancellableCoroutine` helpers and moved `deferred.await()` outside `synchronized`.
