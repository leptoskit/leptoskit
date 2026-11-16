---
"leptoskit_plugin_notifications": minor
---

macOS joins iOS on the remote-registration wire: the Swift plugin's `REMOTE_TOKEN` and `REMOTE_REGISTER` are advertised on macOS hosts too, where registration asks `NSApplication.registerForRemoteNotifications` instead of `UIApplication`'s. Everything else is the same contract the iOS slice shipped — the process-wide broker the app delegate feeds, one OS request at a time with later callers joining the wait, `Unconfirmed` on silence, the stable refusal codes, lower-case hex tokens of no assumed length, and no token in any log, `Debug`, or description. No new wire states.
