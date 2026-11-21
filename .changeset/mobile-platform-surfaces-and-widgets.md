---
"leptoskit_plugin_app_shortcuts": minor
"leptoskit_plugin_home_widgets": minor
"leptoskit_plugin_live_activities": minor
---

# Add mobile widget surfaces and OS integrations

- `leptoskit_plugin_home_widgets::surfaces` adds `SurfaceKind` (Home Screen, Lock Screen circular/rectangular/inline, Control Center, Android Quick Settings tile, Android Glance), `TimelineSchedule` / `TimelineEntry` / `ReloadPolicy`, `WidgetAction`, `ControlCenterControl`, and `QuickSettingsTile` with binary bridge codecs.
- `leptoskit_plugin_live_activities::island` adds `DynamicIslandLayout`, `ActivitySchedule` (stale date, relevance score, push-to-start), and `AndroidLiveUpdate` / `ProgressSegment` for Android 16+ Live Updates.
- `leptoskit_plugin_app_shortcuts::integrations` adds `AppIntentDefinition`, `SpotlightItem` (`CoreSpotlight` / `AppSearch`), `UserActivity` (Apple Handoff), `IncomingShare`, `StatusBarConfig`, `SheetDetent`, `DynamicColorRole` (Material You), `BubbleConfig` (Android Bubbles), `MultiSceneConfig` / `PencilPreference` (iPadOS), and `MobileIntegrationHost` / `MemoryMobileIntegrationHost`.
