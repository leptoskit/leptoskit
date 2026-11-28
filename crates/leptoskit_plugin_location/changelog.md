# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-28)

### Features

#### Location: continuous background updates

_Owner:_ Ifiok Jr. · _Introduced in:_ [a51c332](https://github.com/leptoskit/leptoskit/commit/a51c332f201e582ab58bcd8fb966cef5405442b9)

`leptoskit_plugin_location` grows from foreground reads into a continuous background stream, the shape a life-logging app needs for a constant record of the owner's location.

`request_background_permission` upgrades the foreground grant exactly as each platform requires: iOS asks for "Always" on top of "When In Use", and Android 10+ requests `ACCESS_BACKGROUND_LOCATION` as its own separate step after the fine-location grant; earlier Android versions grant background access implicitly and the call only reports it. The reply is the background scope on its own, so a denied background with a granted foreground reads as denied without wiping the foreground's accuracy report.

`start_updates(UpdateOptions)` starts continuous updates: an accuracy request (best, navigation or balanced), a distance filter in metres (0–10,000), an interval hint in milliseconds (1–3,600,000), a background flag, and iOS's automatic-pause and background-indicator settings. `stop_updates` stops every source without discarding buffered fixes. `next_updates` is a cancellable wait that returns one batch: the fixes buffered since the last call, oldest first, with a `dropped` count for the fixes the bounded native buffer (64) discarded when it was full. An empty batch means no source is running and nothing is buffered: the stream has ended.

iOS also gains `start_visits`, which buffers each visit as a fix at its coordinate timestamped at arrival and, once the visit ends, at departure, and `start_significant_changes`, the low-power source that relaunches a killed app. Both are `Unavailable` on Android, macOS and the browser. After a relaunch the app must start its sources again at startup; iOS then delivers the event that woke it to the new listener.

Android background updates require the `background` plugin's location-typed foreground service (bit 3 of its service-type mask); `start_updates(background: true)` fails with a clear error when it is not running. iOS background updates require the "Always" grant and the `location` background mode. The plugin's registry metadata now requires the app's `NSLocationWhenInUseUsageDescription` and `NSLocationAlwaysAndWhenInUseUsageDescription` purpose strings, adds `location` to `UIBackgroundModes`, and declares `ACCESS_BACKGROUND_LOCATION` and `FOREGROUND_SERVICE_LOCATION` in the Android manifest.

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_plugin_location: foreground location

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_location` is the leptoskit plugin for foreground location permission and cancellable bounded fixes through CoreLocation, Android LocationManager, or browser Geolocation.

Add `leptoskit_plugin_location` as a direct dependency and select the `location` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Location` rather than resolving the plugin on every call. Native reads never prompt; request permission as a separate, intentional action, and the app supplies its usage description and manifest permissions. iOS, macOS, Android, and the browser are supported; Windows and Linux have no location backend yet, and there is no background tracking or geofencing.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
