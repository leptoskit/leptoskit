---
"release": minor
---

# Thermal state for apps

`leptoskit_plugin_thermal` tells an app how hot the device is, so it can put heavy work off while the system is throttling. `guest::Thermal::snapshot` reads the state on Apple's four levels (nominal, fair, serious, critical, or unknown), and `wait_for_change` waits for the next different state and cancels when dropped. iOS and macOS read `ProcessInfo.thermalState` and its change notification; Android 10 and later read `PowerManager`'s thermal status and listener, folding its seven levels into the four and keeping the original in `Snapshot::android_status`. Older Android reports unknown, and desktop and browser hosts don't register the plugin. Battery charge, charging state and Low Power Mode stay in `leptoskit_plugin_battery`.
