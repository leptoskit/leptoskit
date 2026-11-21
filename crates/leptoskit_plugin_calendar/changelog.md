# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-21)

### Features

- **Add `leptoskit_plugin_calendar` for calendar permission, listing calendars (max 64), and creating or reading bounded events on iOS and Android.** Desktop and browser hosts report unavailable. _Owner:_ Ifiok Jr. · _Introduced in:_ [7b07d8f](https://github.com/leptoskit/leptoskit/commit/7b07d8f5999a34ede92c671fbe1c524dd49b7313)

### Fixes

- **Android plugins compile against the public SDK again.** Picture-in-picture listens through AndroidX, which is the type the platform stubs actually ship, and still notices a plain activity leaving PiP when it resumes. The camera plugin passes the failure itself to `PluginException`. Device keys read the SPKI bytes from `PublicKey` instead of `ECKey`, which has none. Calendar, contacts, print, SMS, mail, the web browser, and the media library set a string slice's byte order on the `ByteBuffer` `slice()` returns, because Android's `limit` returns `Buffer`. A calendar event reserves room for both text lengths as well as the timestamps, and decoding a contact advances the caller's buffer so the next record in a list can be read. _Owner:_ Ifiok Jr. · _Introduced in:_ [f33c376](https://github.com/leptoskit/leptoskit/commit/f33c3764e2a3836560d9e22f54b6be6595d8ba65)

#### Apple plugin and JNI builds on current toolchains

_Owner:_ Ifiok Jr. · _Introduced in:_ [fdf5868](https://github.com/leptoskit/leptoskit/commit/fdf586892a1411704c419bfd5069014d4ce5aaa2)

The Apple plugin packages now build and test against the current Xcode SDK. Wire readers no longer declare a `bytes` method that shadows their storage, wire-case helpers have distinct names, calendar validation is shared, EventKit events with no identifier fail honestly, camera photo delegates are retained behind a lock, Security keys use the CF bridge, print uses the current AppKit HTML and PDF APIs, and ActivityKit code is iOS-only. Delegate conformances are main-actor isolated, and the tests import internal wire types with `@testable` and supply a real presentation owner.

The JNI smoke test now finds the JDK through `JAVA_HOME`, invokes the macOS SDK compiler through `xcrun`, and stubs the 3D, timer, and DevTools entry points that the application bridge links.

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0)

### Features

#### leptoskit_plugin_calendar: permission, calendars, and bounded events

`leptoskit_plugin_calendar` queries calendar permission, lists at most 64 calendars, creates events, and reads events by id or in a bounded time window (max 64). Guests enable the `guest` feature and reuse `guest::Calendar`.
