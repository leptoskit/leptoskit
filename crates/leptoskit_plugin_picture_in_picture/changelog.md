# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-13)

### Features

- **Mobile picture-in-picture and system overlays.** Apps can keep a call or video-like surface visible while another app is open. The new `picture-in-picture` runtime plugin reports capabilities and the active session, enters Android activity picture-in-picture as a cancellable lease, and — after the user grants `SYSTEM_ALERT_WINDOW` in system settings — holds an Android system overlay window. Overlay status and `request_overlay` open the grant screen without inventing a permission. iOS reports whether system PiP is supported but does not yet enter it without an attached video player, and has no draw-over-apps overlay. Desktop and the browser return `Unavailable`. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #576](https://github.com/leptoskit/leptoskit/pull/576)

### Fixes

- **Android plugins compile against the public SDK again.** Picture-in-picture listens through AndroidX, which is the type the platform stubs actually ship, and still notices a plain activity leaving PiP when it resumes. The camera plugin passes the failure itself to `PluginException`. Device keys read the SPKI bytes from `PublicKey` instead of `ECKey`, which has none. Calendar, contacts, print, SMS, mail, the web browser, and the media library set a string slice's byte order on the `ByteBuffer` `slice()` returns, because Android's `limit` returns `Buffer`. A calendar event reserves room for both text lengths as well as the timestamps, and decoding a contact advances the caller's buffer so the next record in a list can be read. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #738](https://github.com/leptoskit/leptoskit/pull/738)

#### Apple plugin and JNI builds on current toolchains

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #718](https://github.com/leptoskit/leptoskit/pull/718)

The Apple plugin packages now build and test against the current Xcode SDK. Wire readers no longer declare a `bytes` method that shadows their storage, wire-case helpers have distinct names, calendar validation is shared, EventKit events with no identifier fail honestly, camera photo delegates are retained behind a lock, Security keys use the CF bridge, print uses the current AppKit HTML and PDF APIs, and ActivityKit code is iOS-only. Delegate conformances are main-actor isolated, and the tests import internal wire types with `@testable` and supply a real presentation owner.

The JNI smoke test now finds the JDK through `JAVA_HOME`, invokes the macOS SDK compiler through `xcrun`, and stubs the 3D, timer, and DevTools entry points that the application bridge links.

- **Android picture-in-picture builds against the public SDK.** Android PiP no longer calls `ActivityInfo.supportsPictureInPicture()`, which is `@hide` and missing from the public SDK stubs (so the generated app failed to compile). Capability checks use the system PiP feature plus an activity context; hosts that omit `android:supportsPictureInPicture` still fail closed when entering PiP. The PiP-mode listener imports `android.app.PictureInPictureModeChangedInfo` instead of the non-existent `android.util` type. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #710](https://github.com/leptoskit/leptoskit/pull/710)
- **Harden native-wave plugin correctness.** Live Activities on iOS cancel push-token waits without freezing the per-id busy lock, replace duplicate ActivityKit ids on start, and reject duplicate content keys. Picture-in-picture leases best-effort exit system PiP and observe user exit; `request_overlay` docs match launch-time status. Home widgets stop reporting unrelated pins for unknown kinds, commit preferences before success, and reject dangerous URL schemes. Device-policy and Live Activities stubs validate payloads before Unavailable. Wear/watchOS hosts return errors instead of panicking on viewport kind mismatch. New Android hosts declare `supportsPictureInPicture` on MainActivity. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #618](https://github.com/leptoskit/leptoskit/pull/618)

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-07)

### Features

#### leptoskit_plugin_picture_in_picture: mobile picture-in-picture and overlays

`leptoskit_plugin_picture_in_picture` is the leptoskit plugin for cross-mobile picture-in-picture and Android system overlay windows.

Add `leptoskit_plugin_picture_in_picture` as a direct dependency and select the `picture-in-picture` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::PictureInPicture` rather than resolving the plugin on every call. Android can enter activity picture-in-picture and, with `SYSTEM_ALERT_WINDOW`, hold a system overlay window. iOS reports whether system picture-in-picture is supported but does not yet enter it without an attached video player, and has no draw-over-apps overlay. Desktop and the browser return `Unavailable`.
