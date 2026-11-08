---
"leptoskit_plugin_android_alarm_manager": minor
"leptoskit_cli": minor
"leptoskit_bridge_ffi": minor
---

# Android alarms that run app code

`leptoskit_plugin_android_alarm_manager` ports Flutter's `android_alarm_manager_plus`. Guests schedule one-shot and repeating Android `AlarmManager` alarms by integer ID, with exact, wakeup, allow-while-idle, alarm-clock and reschedule-on-reboot options, check and request the Android 12+ exact-alarm permission (or fall back to inexact timing), cancel and list alarms, and receive fired alarms through a bounded on-device queue that survives process death: `next()` waits for one, `drain()` takes the ones that fired while the app wasn't running. An alarm marked `headless` starts the app's guest without a window when no guest is running, so `handle_launch` can run app code for it within an eight-second budget. iOS, macOS, Windows, Linux, HarmonyOS and the browser return `Unavailable`.

The plugin declares its alarm and boot receivers with the `android-receivers` runtime metadata, which the host-project docs now describe. The Android host gains `NativeHeadlessGuest`, which runs the bundled guest with no window for a receiver or job and discards what it draws.
