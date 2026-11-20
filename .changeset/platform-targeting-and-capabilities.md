---
"leptoskit": minor
"leptoskit_build": minor
"leptoskit_native": minor
---

# Add platform targeting and capability queries

`leptoskit::target` adds a shared runtime and compile-time platform API (`TargetPlatform`, `PlatformFamily`, `PlatformSwitch`, `PlatformCapabilities`, `Capability`, `CapabilitySupport`, `with_platform`, and `on_ios` / `on_android` / `on_mobile` / `on_macos` / `on_windows` / `on_linux` / `on_desktop` / `on_apple` / `on_web` / `on_extension` / `on_watch`) so universal apps can cleanly opt into platform-specific features and test every platform branch from any host.

`leptoskit_build::platforms()` now also recognizes `harmonyos`, `wearos`, `watchos`, and `quest` alongside the existing platform features.
