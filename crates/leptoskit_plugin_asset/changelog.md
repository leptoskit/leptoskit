# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Added

#### leptoskit_plugin_asset: packaged-asset resolve wire

Status + resolve codecs for logical packaged-asset names. Every host returns typed Unavailable until packaged-asset ownership wiring lands; never invents a URI for a missing asset.

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-13)

### Features

- **Add `leptoskit_plugin_asset` for packaged-asset capability status and name→URI resolve.** Hosts return Unavailable in this MVP until packaged-asset ownership wiring lands. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #699](https://github.com/leptoskit/leptoskit/pull/699)

### Fixes

#### Apple plugin and JNI builds on current toolchains

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #718](https://github.com/leptoskit/leptoskit/pull/718)

The Apple plugin packages now build and test against the current Xcode SDK. Wire readers no longer declare a `bytes` method that shadows their storage, wire-case helpers have distinct names, calendar validation is shared, EventKit events with no identifier fail honestly, camera photo delegates are retained behind a lock, Security keys use the CF bridge, print uses the current AppKit HTML and PDF APIs, and ActivityKit code is iOS-only. Delegate conformances are main-actor isolated, and the tests import internal wire types with `@testable` and supply a real presentation owner.

The JNI smoke test now finds the JDK through `JAVA_HOME`, invokes the macOS SDK compiler through `xcrun`, and stubs the 3D, timer, and DevTools entry points that the application bridge links.

- **P3 plugin quality: fix background_task Apple Auto construction and honesty, asset Kotlin NUL parity, age_range docs, and tighter wire tests.** _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #706](https://github.com/leptoskit/leptoskit/pull/706)
