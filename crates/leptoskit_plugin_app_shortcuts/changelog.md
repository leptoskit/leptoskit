# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Features

#### Generated hosts deliver shortcut launches

Android dynamic shortcuts carry a unique intent per id. Generated iOS, Android, and macOS hosts call `noteLaunch` for quick actions, launcher shortcuts, and dock-menu picks. `DockShortcutStore.plugin` and `AppShortcutsPlugin.shared` expose the registered instance for custom hosts.

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-28)

### Features

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

- **Native capability parity: badges and shortcut hosts.** Browser pages can set and clear app-icon badges through the Badging API (`web::set` / `web::clear`), and `web::get` returns the last count this adapter published. Generated iOS, Android, and macOS hosts now call `noteLaunch` when the OS activates an app shortcut: Android dynamic shortcuts carry a unique intent per id, the macOS host builds the dock menu from `DockShortcutStore`, and iOS notes home-screen quick actions. Docs match the real backends for menus, badges, and shortcuts. _Owner:_ Ifiok Jr. · _Introduced in:_ [0c66c58](https://github.com/leptoskit/leptoskit/commit/0c66c58142338c3399198783a442bc777f04476c)

#### App shortcuts, icon badges, and NFC reads

_Owner:_ Ifiok Jr. · _Introduced in:_ [888ff04](https://github.com/leptoskit/leptoskit/commit/888ff04cf9d89bc604a176efa427dd3c3bf4251e)

Three new runtime plugins cover high-value native capabilities that were still missing:

- **`app-shortcuts`** publishes up to eight static launcher shortcuts (iOS quick actions, Android `ShortcutManager`, macOS dock-menu store) and returns the id that launched the app.
- **`badge`** sets, reads, and clears the app icon badge count on Apple and Android, with a Linux Unity/KDE D-Bus client for custom hosts.
- **`nfc`** queries hardware readiness and reads one bounded NDEF text or URI record on iOS and Android.

Each plugin ships guest clients, wire codecs, Apple/Android backends (with stubs where a platform has no hardware), docs, and tests. Windows jump-list publishing, full Siri App Intents, background NFC, and badge Badging API support remain follow-ups.

### Fixes

- **Harden keyboard shortcuts and menu accelerators.** Global shortcuts and menu items now share the same muda `CmdOrCtrl+…` accelerator bounds and conflict rules: chords are capped at 64 printable ASCII bytes, menus reject duplicate accelerators (including equivalent spellings after parse), and dropping a global-shortcut owner retries native unregister so failed RAII cleanup does not leave OS bindings behind. Closing the Apple or Android app-shortcuts plugin clears published launcher entries so they cannot outlive the bridge. _Owner:_ Ifiok Jr. · _Introduced in:_ [89b4fea](https://github.com/leptoskit/leptoskit/commit/89b4feab1dcdc4767bd4589186f22b7c4be384a7)

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0)

### Features

#### leptoskit_plugin_app_shortcuts: launcher shortcuts

`leptoskit_plugin_app_shortcuts` publishes a bounded static shortcut list to iOS home-screen quick actions, Android `ShortcutManager`, and a macOS dock-menu store. Guests enable the `guest` feature and reuse `guest::AppShortcuts`.
