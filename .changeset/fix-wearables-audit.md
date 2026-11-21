---
"leptoskit_watch": patch
"leptoskit_bridge_ffi": patch
"leptoskit_plugin_health": patch
"leptoskit_plugin_pedometer": patch
"leptoskit_plugin_battery": patch
"leptoskit_plugin_ble": patch
"leptoskit_plugin_watch_sync": patch
"leptoskit_plugin_wearable": patch
"leptoskit_plugin_haptics": patch
"leptoskit_cli": patch
---

# Fix wearable wire codecs, watch plugins, and host builds

Fixes cross-platform wire mismatches in Health (`ABSENT_LENGTH`, `ANDROID_TYPES` / `HARMONY_TYPES` bitmask preserving `HRV_RMSSD`, and exercise `Activity` codes 8..14), HarmonyOS Pedometer (`bounded_reply` 36-byte framing) and Battery charge state mapping, and BLE 16-bit/128-bit UUID normalization. Fixes Apple/Android runtime bugs across `ble`, `watch-sync`, `wearable`, and watchOS `haptics`, adds `Reading::Ecg` for `SensorMask::ECG`, fixes `RotaryState` wrap-around detent math, adds watchOS 10 support to `LeptosKitBridge` and wearable SwiftPM manifests, enables native plugin resolution and unified output paths for `wearos`, `watchos`, and `harmonyos` in `leptoskit_cli`, and adds the wearable crates to CI unit tests.
