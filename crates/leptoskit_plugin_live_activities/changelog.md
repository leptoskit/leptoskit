# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-21)

### Features

#### iOS Live Activities

_Owner:_ Ifiok Jr. · _Introduced in:_ [4c0123c](https://github.com/leptoskit/leptoskit/commit/4c0123ca69be52687c94cfb7b3b0204048ca5bb7)

Apps can start, update, and end iOS Live Activities (Lock Screen and Dynamic Island) through the new `leptoskit_plugin_live_activities` crate and the `live-activities` runtime plugin. Guests call `are_enabled`, `start`, `update`, `end`, `wait_push_token`, and `list`. Content state is a bounded string map that a Widget Extension renders via `LeptosKitLiveActivityAttributes`.

Only iOS implements ActivityKit. Android, macOS, the browser, and the unsupported host stub return `Unavailable` after validating payloads. Apps must set `NSSupportsLiveActivities` in Info.plist, ship a Widget Extension, and enable Push Notifications when requesting push tokens. Custom attribute types, push-to-start tokens, and remote payload helpers are still out of scope.

#### Add mobile widget surfaces and OS integrations

_Owner:_ Ifiok Jr. · _Introduced in:_ [c3e229a](https://github.com/leptoskit/leptoskit/commit/c3e229abe4ab8bec104922c3719c46ae3fe76c73)

- `leptoskit_plugin_home_widgets::surfaces` adds `SurfaceKind` (Home Screen, Lock Screen circular/rectangular/inline, Control Center, Android Quick Settings tile, Android Glance), `TimelineSchedule` / `TimelineEntry` / `ReloadPolicy`, `WidgetAction`, `ControlCenterControl`, and `QuickSettingsTile` with binary bridge codecs.
- `leptoskit_plugin_live_activities::island` adds `DynamicIslandLayout`, `ActivitySchedule` (stale date, relevance score, push-to-start), and `AndroidLiveUpdate` / `ProgressSegment` for Android 16+ Live Updates.
- `leptoskit_plugin_app_shortcuts::integrations` adds `AppIntentDefinition`, `SpotlightItem` (`CoreSpotlight` / `AppSearch`), `UserActivity` (Apple Handoff), `IncomingShare`, `StatusBarConfig`, `SheetDetent`, `DynamicColorRole` (Material You), `BubbleConfig` (Android Bubbles), `MultiSceneConfig` / `PencilPreference` (iPadOS), and `MobileIntegrationHost` / `MemoryMobileIntegrationHost`.

### Fixes

#### Apple plugin and JNI builds on current toolchains

_Owner:_ Ifiok Jr. · _Introduced in:_ [fdf5868](https://github.com/leptoskit/leptoskit/commit/fdf586892a1411704c419bfd5069014d4ce5aaa2)

The Apple plugin packages now build and test against the current Xcode SDK. Wire readers no longer declare a `bytes` method that shadows their storage, wire-case helpers have distinct names, calendar validation is shared, EventKit events with no identifier fail honestly, camera photo delegates are retained behind a lock, Security keys use the CF bridge, print uses the current AppKit HTML and PDF APIs, and ActivityKit code is iOS-only. Delegate conformances are main-actor isolated, and the tests import internal wire types with `@testable` and supply a real presentation owner.

The JNI smoke test now finds the JDK through `JAVA_HOME`, invokes the macOS SDK compiler through `xcrun`, and stubs the 3D, timer, and DevTools entry points that the application bridge links.

- **Harden native-wave plugin correctness.** Live Activities on iOS cancel push-token waits without freezing the per-id busy lock, replace duplicate ActivityKit ids on start, and reject duplicate content keys. Picture-in-picture leases best-effort exit system PiP and observe user exit; `request_overlay` docs match launch-time status. Home widgets stop reporting unrelated pins for unknown kinds, commit preferences before success, and reject dangerous URL schemes. Device-policy and Live Activities stubs validate payloads before Unavailable. Wear/watchOS hosts return errors instead of panicking on viewport kind mismatch. New Android hosts declare `supportsPictureInPicture` on MainActivity. _Owner:_ Ifiok Jr. · _Introduced in:_ [879db6b](https://github.com/leptoskit/leptoskit/commit/879db6b2334d0c5bd8519f9a26f7a3eb92fca7f0)

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0)

### Features

#### leptoskit_plugin_live_activities: Live Activities

`leptoskit_plugin_live_activities` is the leptoskit plugin for starting, updating, and ending iOS Live Activities (including Dynamic Island presentations) with optional ActivityKit push-token hooks.

Add `leptoskit_plugin_live_activities` as a direct dependency and select the `live-activities` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::LiveActivities` rather than resolving the plugin on every call. Only iOS implements ActivityKit; other platforms return `Unavailable`. Starting an activity does not prove Lock Screen or Dynamic Island UI appeared: the app must ship a Widget Extension that renders `LeptosKitLiveActivityAttributes`.
