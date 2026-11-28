# Changelog

## Unreleased

### Added

- Process-memory owned blobs: status → Available; create (≤64KiB) → id; read bounded slices (empty = EOF); release. Apple, Android, Linux, Windows, and browser Direct share the same in-memory contract; durable shared-memory / object-URL backends remain later scope.

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-28)

### Features

- **Add `leptoskit_plugin_blob` for bounded create/read/release of owned byte blobs.** Hosts return Unavailable in this MVP until a durable resource protocol lands. _Owner:_ Ifiok Jr. · _Introduced in:_ [9e84b3c](https://github.com/leptoskit/leptoskit/commit/9e84b3cd3294a5bfdb0110230b9bd74b269754c2)

### Fixes

#### Apple plugin and JNI builds on current toolchains

_Owner:_ Ifiok Jr. · _Introduced in:_ [fdf5868](https://github.com/leptoskit/leptoskit/commit/fdf586892a1411704c419bfd5069014d4ce5aaa2)

The Apple plugin packages now build and test against the current Xcode SDK. Wire readers no longer declare a `bytes` method that shadows their storage, wire-case helpers have distinct names, calendar validation is shared, EventKit events with no identifier fail honestly, camera photo delegates are retained behind a lock, Security keys use the CF bridge, print uses the current AppKit HTML and PDF APIs, and ActivityKit code is iOS-only. Delegate conformances are main-actor isolated, and the tests import internal wire types with `@testable` and supply a real presentation owner.

The JNI smoke test now finds the JDK through `JAVA_HOME`, invokes the macOS SDK compiler through `xcrun`, and stubs the 3D, timer, and DevTools entry points that the application bridge links.

- **Blob and diagnostics MVPs now run in process memory / no-op allocators with STATUS probing, instead of Unavailable-only stubs.** _Owner:_ Ifiok Jr. · _Introduced in:_ [8108def](https://github.com/leptoskit/leptoskit/commit/8108def0b96f9241954831697a3b1267808784c7)
