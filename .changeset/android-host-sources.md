---
"leptoskit_cli": patch
---

# Android and Apple hosts ship every view they draw

Generated Android projects now compile: the platform inventories include `NativeCamera.kt`, `NativeMap.kt`, `NativeLivePhoto.kt`, and `NativeServiceWakeBroker.kt`, so `NativeWidgetRenderer` and `LeptosKitView` resolve the views they draw and the background wake they install. The macOS and iOS inventories likewise include `NativeCamera.swift`, `NativeMap.swift`, and `NativeLivePhoto.swift`, which their widget renderers already used.

A test now fails when a shipped Kotlin or Swift source refers to a type declared only in a file the platform inventory leaves out, so this class of missing source is caught before Gradle or Xcode sees it.
