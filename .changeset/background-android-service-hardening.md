---
"leptoskit_plugin_background": patch
---

# Guard Android foreground starts, timeouts, and stop races

`BackgroundService` now catches `SecurityException` and `IllegalStateException` around `startForeground`, retries once with `dataSync` alone (updating `BackgroundState.serviceTypes` to reflect only the running type) when another type fails, and stops while cancelling periodic work (leaving saved configuration intact) if nothing can be started. It also overrides `onTimeout(int, int)` on Android 15+ to leave the foreground and stop cleanly when a `dataSync` time limit expires while keeping the saved configuration and periodic work, checks runtime permissions before `BackgroundWorker` or `BootReceiver` call `startForegroundService`, keeps periodic work enqueued when nothing is persisted yet, and distinguishes worker/boot restarts from `BackgroundPlugin.start` so an in-flight worker cannot revive the service after `stop()`.
