---
"leptoskit_bridge_ffi": patch
"leptoskit_plugin_age_range": patch
"leptoskit_plugin_app_integrity": patch
"leptoskit_plugin_asset": patch
"leptoskit_plugin_background_task": patch
"leptoskit_plugin_blob": patch
"leptoskit_plugin_calendar": patch
"leptoskit_plugin_camera": patch
"leptoskit_plugin_contacts": patch
"leptoskit_plugin_device_keys": patch
"leptoskit_plugin_diagnostics": patch
"leptoskit_plugin_live_activities": patch
"leptoskit_plugin_live_photo": patch
"leptoskit_plugin_mail_composer": patch
"leptoskit_plugin_maps": patch
"leptoskit_plugin_media_library": patch
"leptoskit_plugin_nfc": patch
"leptoskit_plugin_picture_in_picture": patch
"leptoskit_plugin_print": patch
"leptoskit_plugin_sms": patch
"leptoskit_plugin_video_thumbnails": patch
"leptoskit_plugin_web_browser": patch
---

# Apple plugin and JNI builds on current toolchains

The Apple plugin packages now build and test against the current Xcode SDK. Wire readers no longer declare a `bytes` method that shadows their storage, wire-case helpers have distinct names, calendar validation is shared, EventKit events with no identifier fail honestly, camera photo delegates are retained behind a lock, Security keys use the CF bridge, print uses the current AppKit HTML and PDF APIs, and ActivityKit code is iOS-only. Delegate conformances are main-actor isolated, and the tests import internal wire types with `@testable` and supply a real presentation owner.

The JNI smoke test now finds the JDK through `JAVA_HOME`, invokes the macOS SDK compiler through `xcrun`, and stubs the 3D, timer, and DevTools entry points that the application bridge links.
