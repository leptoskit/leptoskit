# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-25)

### Features

- **Home-screen and desktop widgets.** `leptoskit_plugin_home_widgets` publishes bounded text snapshots for iOS and macOS WidgetKit and Android App Widgets, then asks the OS to reload them. Guests call `guest::HomeWidgets` for capability, set/get/clear, reload, and installed queries. Apps still own their WidgetKit extensions and App Widget providers; Apple needs an App Group for a shared suite, and Windows, Linux, and the browser remain unavailable. _Owner:_ Ifiok Jr. · _Introduced in:_ [bca6fc6](https://github.com/leptoskit/leptoskit/commit/bca6fc665b53b3f5945c25c13d6b029024cf541c)

#### Add mobile widget surfaces and OS integrations

_Owner:_ Ifiok Jr. · _Introduced in:_ [c3e229a](https://github.com/leptoskit/leptoskit/commit/c3e229abe4ab8bec104922c3719c46ae3fe76c73)

- `leptoskit_plugin_home_widgets::surfaces` adds `SurfaceKind` (Home Screen, Lock Screen circular/rectangular/inline, Control Center, Android Quick Settings tile, Android Glance), `TimelineSchedule` / `TimelineEntry` / `ReloadPolicy`, `WidgetAction`, `ControlCenterControl`, and `QuickSettingsTile` with binary bridge codecs.
- `leptoskit_plugin_live_activities::island` adds `DynamicIslandLayout`, `ActivitySchedule` (stale date, relevance score, push-to-start), and `AndroidLiveUpdate` / `ProgressSegment` for Android 16+ Live Updates.
- `leptoskit_plugin_app_shortcuts::integrations` adds `AppIntentDefinition`, `SpotlightItem` (`CoreSpotlight` / `AppSearch`), `UserActivity` (Apple Handoff), `IncomingShare`, `StatusBarConfig`, `SheetDetent`, `DynamicColorRole` (Material You), `BubbleConfig` (Android Bubbles), `MultiSceneConfig` / `PencilPreference` (iPadOS), and `MobileIntegrationHost` / `MemoryMobileIntegrationHost`.

#### Wire mobile Live Activities, widgets, Spotlight, and shares

_Owner:_ Ifiok Jr. · _Introduced in:_ [7910a8f](https://github.com/leptoskit/leptoskit/commit/7910a8fd84838cf6f59dab3347c4233cefa0575b)

- `leptoskit_plugin_live_activities`: maps `DynamicIslandLayout` slots onto `ContentState` and `LeptosKitLiveActivityAttributes`, adds `wait_push_to_start_token` (iOS 17.2+), `start_scheduled` / `update_scheduled` (`staleDate` + `relevanceScore`), and `post_android_live_update` / `end_android_live_update` (Android 16+ `Notification.ProgressStyle` with ongoing progress notification fallback).
- `leptoskit_plugin_home_widgets`: adds bridge methods for `TimelineSchedule`, iOS 18+ `ControlCenterControl` (`ControlCenter.shared.reloadControls`), and Android `QuickSettingsTile`, reads `LeptosKitAppGroup` automatically from `Info.plist` on Apple, and adds `LeptosKitWidgetStore` / `HomeWidgetsPlugin.readSnapshot` / `readQuickTile` helpers for native widget extensions and tile services.
- `leptoskit_plugin_app_shortcuts`: wires `CoreSpotlight` indexing (`index_spotlight` / `delete_spotlight`), Apple Handoff (`start_user_activity` / `end_user_activity`), and incoming share intake (`consume_incoming_share`) on both Apple and Android hosts.

### Fixes

- **Harden native-wave plugin correctness.** Live Activities on iOS cancel push-token waits without freezing the per-id busy lock, replace duplicate ActivityKit ids on start, and reject duplicate content keys. Picture-in-picture leases best-effort exit system PiP and observe user exit; `request_overlay` docs match launch-time status. Home widgets stop reporting unrelated pins for unknown kinds, commit preferences before success, and reject dangerous URL schemes. Device-policy and Live Activities stubs validate payloads before Unavailable. Wear/watchOS hosts return errors instead of panicking on viewport kind mismatch. New Android hosts declare `supportsPictureInPicture` on MainActivity. _Owner:_ Ifiok Jr. · _Introduced in:_ [879db6b](https://github.com/leptoskit/leptoskit/commit/879db6b2334d0c5bd8519f9a26f7a3eb92fca7f0)

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-07)

### Breaking changes

#### leptoskit_plugin_home_widgets: home-screen and desktop widgets

_Owner:_ [@ifiokjr](https://github.com/ifiokjr)

`leptoskit_plugin_home_widgets` is the leptoskit plugin for publishing bounded widget snapshots to iOS and macOS WidgetKit and Android App Widgets, then asking the OS to reload them.

Add `leptoskit_plugin_home_widgets` as a direct dependency and select the `home-widgets` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Guests enable the `guest` feature and reuse `guest::HomeWidgets`. The plugin does not draw widget UI or scaffold extension targets; apps still own their WidgetKit extensions and App Widget providers.
