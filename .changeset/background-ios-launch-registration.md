---
"leptoskit_plugin_background": patch
"leptoskit_cli": patch
---

# The background plugin no longer crashes iOS apps

iOS ends an app that registers a `BGTaskScheduler` handler after launch finishes, or registers the same one twice, and the `background` plugin registered its handlers in `start`, which the guest calls once the app is already running. The handlers are now registered by `BackgroundPlugin.registerLaunchHandlers()`, which the generated iOS app delegate calls from `application(_:willFinishLaunchingWithOptions:)`; repeated calls are ignored, and a task whose identifier or background mode is missing from the app's `Info.plist` is skipped instead of registered. `start` now only schedules the next app refresh. An app whose `hosts/ios/App.swift` was generated before this keeps launching, and its `start` answers `Unavailable` on iOS until it adds the call.
