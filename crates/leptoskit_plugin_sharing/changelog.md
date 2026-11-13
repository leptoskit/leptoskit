# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-13)

### Features

#### Share files, subject and title, and where it went

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #625](https://github.com/leptoskit/leptoskit/pull/625)

`leptoskit_plugin_sharing` now covers Flutter `share_plus`'s features. A share can carry up to 32 files, optionally with text: `File::app("exports/report.pdf")` names a file in your app's files directory (the root the `filesystem` plugin writes), and `File::bytes("chart.png", bytes)` shares data from memory, each with an optional MIME type and display name. `with_subject` sets the email subject, `with_title` the sheet or chooser title, `with_anchor` where the iPad popover or macOS picker points, and `excluding` leaves iOS activities out of the sheet.

`share` now returns a `ShareResult`: `Success` with the destination the user picked (the iOS activity type, the macOS service title, or the Android component), `Dismissed`, or `Unavailable` when the platform cannot tell, as in browsers. A result never confirms delivery. On Android this replaces the old behaviour, where dismissing the chooser and choosing an app looked the same.

Native hosts check that an app file stays inside the files directory, refusing symlinks that lead out of it, and copy shared files into a private cache that the next file share clears. On Android the plugin ships its own `FileProvider` and grants read access only to the chosen app, only for those copies; it adds `androidx.core:core`. Browsers share in-memory files through Web Share Level 2 and return `Unavailable`, without a download fallback, when they cannot. Windows and Linux still have no native share sheet.

Breaking: the wire protocol is versioned and changed. The previous request (a kind and one value) and its empty reply are refused, `share` returns `ShareResult` instead of `()`, `web::share` resolves to a `ShareResult` (a browser cancellation is `Dismissed`, no longer a `Cancelled` error, and an iOS or macOS dismissal is `Dismissed` rather than a `Cancelled` error), and `Content::as_str` and `Content::is_url` are replaced by `shared_text`, `shared_url`, `shared_files`, `subject` and `title`.

The CLI reads a new `android-file-providers` plugin metadata key: it copies the plugin's paths XML into the generated resources and declares an unexported `FileProvider` in the generated Android manifest.

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_plugin_sharing: native sharing

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_sharing` is the leptoskit plugin for bounded text and HTTP(S) URL sharing through the user-selected native share UI.

Add `leptoskit_plugin_sharing` as a direct dependency and select the `sharing` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and call `share`, and browser apps call `web::share` from a user action. iOS, macOS, Android, and browsers with Web Share are supported; Windows and Linux have no native share sheet yet, and files are not shared. The app supplies the content, the user picks the destination, and completion only means the platform finished its API callback.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
