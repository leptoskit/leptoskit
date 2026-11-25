---
"leptoskit_plugin_background": patch
---

# Keep the Android foreground service type across restarts

Android 14 and later throw `InvalidForegroundServiceTypeException` when a foreground service calls `startForeground` with type `0` or with an undeclared type bit. `leptoskit_plugin_background` now maps its `ForegroundServiceTypes` bitfield to Android's `ServiceInfo.FOREGROUND_SERVICE_TYPE_*` constants per SDK level (keeping `BackgroundState.serviceTypes` in crate numbering for the location and recorder plugins), persists the active configuration in app-private `SharedPreferences` so `WorkManager`, boot, and sticky restarts preserve the notification and service types instead of resetting them to zero, avoids restarting an already-running service from `BackgroundWorker`, catches boot-time foreground-service start restrictions in `BootReceiver`, and clears the saved configuration while cancelling the `WorkManager` periodic work in `stop`.
