---
"release": minor
---

# Background execution for apps

`leptoskit_plugin_background` keeps a LeptosKit app working in the background. On Android it runs a foreground `Service` with a persistent, low-importance notification (its own channel) and wakes the app's Rust `AppService` on a configurable interval through the host's existing service-wake path; a WorkManager periodic fallback restarts the service every 15 minutes, and an opt-in `BOOT_COMPLETED` receiver restarts it after a reboot. The service's `foregroundServiceType` is declared in the generated manifest from the plugin's metadata, with the matching `FOREGROUND_SERVICE_*` permissions, and a guest API asks the user to exempt the app from battery optimisation. On iOS the plugin registers `BGTaskScheduler` app-refresh and processing tasks that wake the `AppService`, with `UIBackgroundModes` and `BGTaskSchedulerPermittedIdentifiers` merged into the generated `Info.plist`. macOS, desktop, and the browser report `Unavailable`.

The guest API is `guest::Background::{start, stop, status, request_ignore_battery_optimizations}`, behind the `guest` feature. `status` reports whether background execution is running, the last wake, and the battery-optimisation state. The `AppService` now receives a wake reason through a new `set_wake_reason` hook, so a service can tell a platform background tick from an ordinary app call.
