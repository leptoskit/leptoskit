# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-21)

### Features

#### Android intents plugin

_Owner:_ Ifiok Jr. · _Introduced in:_ [5261e0a](https://github.com/leptoskit/leptoskit/commit/5261e0a1a578c6ba300de59223516633170f944b)

`leptoskit_plugin_android_intent` ports Flutter's `android_intent_plus`: guests describe an intent with `Intent::new().action(…).data(…).extra(…)` and launch it, open it through a titled chooser, send it as a broadcast, start a service, check whether an activity resolves it, read the package, class, and label it resolves to, or launch an `intent:` URI. Extras cover booleans, ints, longs, doubles, strings, nested bundles, byte arrays, integer and string lists, and boolean, int, long, double, and string arrays.

Only Android registers the plugin; everywhere else `guest::AndroidIntent::new` returns `Unavailable`. Requests travel as a bounded little-endian wire that the Kotlin plugin validates again, because guest code can change over the air: URI-permission grant flags and `file:` URIs are refused with `PermissionDenied`, extras can't carry parcelables, and `intent:` URIs can't name a component or selector and only reach browsable activities. A missing activity or service is `Unavailable` and an Android `SecurityException` is `PermissionDenied`. Resolution on Android 11 and later only sees apps the app's own `<queries>` declares.

### Fixes

- **A drawing-scaled vector stroke follows the layout frame, not the view after it has been snapped out to whole pixels.** At 420 dpi a 10dp edge becomes 27px, and the stroke was a little wider than the layout asked for. `parse_and_launch` no longer opens an `intent:` URI that has no scheme. API 35's Settings activity is browsable, so an action-only Settings URI used to start it. The activity it does open still has to declare `CATEGORY_BROWSABLE` on the filter that matched. _Owner:_ Ifiok Jr. · _Introduced in:_ [8128505](https://github.com/leptoskit/leptoskit/commit/81285051e628dada5bf9819978e4d608c53b1fb5)
