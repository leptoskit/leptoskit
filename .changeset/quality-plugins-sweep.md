---
"leptoskit_plugin_live_activities": patch
"leptoskit_plugin_picture_in_picture": patch
"leptoskit_plugin_home_widgets": patch
"leptoskit_plugin_device_policy": patch
"leptoskit_wearos": patch
"leptoskit_watchos": patch
"leptoskit_cli": patch
---

# Harden native-wave plugin correctness

Live Activities on iOS cancel push-token waits without freezing the per-id busy lock, replace duplicate ActivityKit ids on start, and reject duplicate content keys. Picture-in-picture leases best-effort exit system PiP and observe user exit; `request_overlay` docs match launch-time status. Home widgets stop reporting unrelated pins for unknown kinds, commit preferences before success, and reject dangerous URL schemes. Device-policy and Live Activities stubs validate payloads before Unavailable. Wear/watchOS hosts return errors instead of panicking on viewport kind mismatch. New Android hosts declare `supportsPictureInPicture` on MainActivity.
