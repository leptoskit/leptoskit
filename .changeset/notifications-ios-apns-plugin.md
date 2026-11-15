---
"leptoskit_plugin_notifications": minor
---

iOS APNs device-token acquisition in the notifications plugin. The generated app delegate forwards `didRegisterForRemoteNotificationsWithDeviceToken` and `didFailToRegisterForRemoteNotificationsWithError` into a process-wide broker that answers before the plugin or its guest exists and survives plugin close without retaining a closed guest; the token is held in memory only, forwarded as exact lower-case hex of whatever length the OS issued, never logged, and never invented on failure. Two new methods — `REMOTE_TOKEN` reads the revisioned latest answer without side effects, and `REMOTE_REGISTER` asks the OS to register for remote notifications and waits a bounded time for the answer, so a silent OS resolves instead of hanging, every waiter sees the same answer, and a cancelled wait keeps the late callback readable. Every other platform answers `Unavailable` for both; methods 0–6 are unchanged.
