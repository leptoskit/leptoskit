---
"leptoskit_plugin_app_shortcuts": minor
"leptoskit_plugin_home_widgets": minor
"leptoskit_plugin_live_activities": minor
---

# Wire mobile Live Activities, widgets, Spotlight, and shares

- `leptoskit_plugin_live_activities`: maps `DynamicIslandLayout` slots onto `ContentState` and `LeptosKitLiveActivityAttributes`, adds `wait_push_to_start_token` (iOS 17.2+), `start_scheduled` / `update_scheduled` (`staleDate` + `relevanceScore`), and `post_android_live_update` / `end_android_live_update` (Android 16+ `Notification.ProgressStyle` with ongoing progress notification fallback).
- `leptoskit_plugin_home_widgets`: adds bridge methods for `TimelineSchedule`, iOS 18+ `ControlCenterControl` (`ControlCenter.shared.reloadControls`), and Android `QuickSettingsTile`, reads `LeptosKitAppGroup` automatically from `Info.plist` on Apple, and adds `LeptosKitWidgetStore` / `HomeWidgetsPlugin.readSnapshot` / `readQuickTile` helpers for native widget extensions and tile services.
- `leptoskit_plugin_app_shortcuts`: wires `CoreSpotlight` indexing (`index_spotlight` / `delete_spotlight`), Apple Handoff (`start_user_activity` / `end_user_activity`), and incoming share intake (`consume_incoming_share`) on both Apple and Android hosts.
