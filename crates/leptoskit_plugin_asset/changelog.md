# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Added

#### leptoskit_plugin_asset: packaged-asset resolve wire

Status + resolve codecs for logical packaged-asset names. Every host returns typed Unavailable until packaged-asset ownership wiring lands; never invents a URI for a missing asset.

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-28)

### Features

- **Add `leptoskit_plugin_asset` for packaged-asset capability status and name→URI resolve.** Hosts return Unavailable in this MVP until packaged-asset ownership wiring lands. _Owner:_ Ifiok Jr. · _Introduced in:_ [97a7345](https://github.com/leptoskit/leptoskit/commit/97a734597a1813b29cf7a845d8fd4fa138a9b069)

### Fixes

#### Apple plugin and JNI builds on current toolchains

_Owner:_ Ifiok Jr. · _Introduced in:_ [fdf5868](https://github.com/leptoskit/leptoskit/commit/fdf586892a1411704c419bfd5069014d4ce5aaa2)

The Apple plugin packages now build and test against the current Xcode SDK. Wire readers no longer declare a `bytes` method that shadows their storage, wire-case helpers have distinct names, calendar validation is shared, EventKit events with no identifier fail honestly, camera photo delegates are retained behind a lock, Security keys use the CF bridge, print uses the current AppKit HTML and PDF APIs, and ActivityKit code is iOS-only. Delegate conformances are main-actor isolated, and the tests import internal wire types with `@testable` and supply a real presentation owner.

The JNI smoke test now finds the JDK through `JAVA_HOME`, invokes the macOS SDK compiler through `xcrun`, and stubs the 3D, timer, and DevTools entry points that the application bridge links.

- **P3 plugin quality: fix background_task Apple Auto construction and honesty, asset Kotlin NUL parity, age_range docs, and tighter wire tests.** _Owner:_ Ifiok Jr. · _Introduced in:_ [e4c23c8](https://github.com/leptoskit/leptoskit/commit/e4c23c81dc4c8ad7032b6b7c1e257bbbeef6da83)
