# Changelog

## Unreleased

### Added

- In-process diagnostic spans: status → Available; start → span id; end; event. No hosted exporter required. Apple, Android, Linux, Windows, and browser Direct share the no-op allocator; OpenTelemetry / OS logging remain later.

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-13)

### Features

- **Add `leptoskit_plugin_diagnostics` for start/end span and event recording.** Hosts return Unavailable in this MVP until a diagnostics backend is chosen. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #700](https://github.com/leptoskit/leptoskit/pull/700)

### Fixes

#### Apple plugin and JNI builds on current toolchains

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #718](https://github.com/leptoskit/leptoskit/pull/718)

The Apple plugin packages now build and test against the current Xcode SDK. Wire readers no longer declare a `bytes` method that shadows their storage, wire-case helpers have distinct names, calendar validation is shared, EventKit events with no identifier fail honestly, camera photo delegates are retained behind a lock, Security keys use the CF bridge, print uses the current AppKit HTML and PDF APIs, and ActivityKit code is iOS-only. Delegate conformances are main-actor isolated, and the tests import internal wire types with `@testable` and supply a real presentation owner.

The JNI smoke test now finds the JDK through `JAVA_HOME`, invokes the macOS SDK compiler through `xcrun`, and stubs the 3D, timer, and DevTools entry points that the application bridge links.

- **Blob and diagnostics MVPs now run in process memory / no-op allocators with STATUS probing, instead of Unavailable-only stubs.** _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #705](https://github.com/leptoskit/leptoskit/pull/705)
