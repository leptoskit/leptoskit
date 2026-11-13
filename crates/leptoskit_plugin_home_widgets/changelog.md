# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-13)

### Features

- **Home-screen and desktop widgets.** `leptoskit_plugin_home_widgets` publishes bounded text snapshots for iOS and macOS WidgetKit and Android App Widgets, then asks the OS to reload them. Guests call `guest::HomeWidgets` for capability, set/get/clear, reload, and installed queries. Apps still own their WidgetKit extensions and App Widget providers; Apple needs an App Group for a shared suite, and Windows, Linux, and the browser remain unavailable. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #579](https://github.com/leptoskit/leptoskit/pull/579)

### Fixes

- **Harden native-wave plugin correctness.** Live Activities on iOS cancel push-token waits without freezing the per-id busy lock, replace duplicate ActivityKit ids on start, and reject duplicate content keys. Picture-in-picture leases best-effort exit system PiP and observe user exit; `request_overlay` docs match launch-time status. Home widgets stop reporting unrelated pins for unknown kinds, commit preferences before success, and reject dangerous URL schemes. Device-policy and Live Activities stubs validate payloads before Unavailable. Wear/watchOS hosts return errors instead of panicking on viewport kind mismatch. New Android hosts declare `supportsPictureInPicture` on MainActivity. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #618](https://github.com/leptoskit/leptoskit/pull/618)

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-07)

### Breaking changes

#### leptoskit_plugin_home_widgets: home-screen and desktop widgets

_Owner:_ [@ifiokjr](https://github.com/ifiokjr)

`leptoskit_plugin_home_widgets` is the leptoskit plugin for publishing bounded widget snapshots to iOS and macOS WidgetKit and Android App Widgets, then asking the OS to reload them.

Add `leptoskit_plugin_home_widgets` as a direct dependency and select the `home-widgets` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Guests enable the `guest` feature and reuse `guest::HomeWidgets`. The plugin does not draw widget UI or scaffold extension targets; apps still own their WidgetKit extensions and App Widget providers.
