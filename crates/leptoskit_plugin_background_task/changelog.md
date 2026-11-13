# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Fixed

- Apple Auto hosts no longer claim Available/`BGTaskScheduler` without declared identifiers; STATUS is Unavailable until plist wiring lands. Dropped mistaken `apple-context` so generated iOS/macOS hosts construct the plugin.

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-13)

### Features

- **Add `leptoskit_plugin_background_task` for named OS background-task register/unregister/status/complete (`BGTaskScheduler` / `WorkManager`), distinct from `leptoskit_plugin_background` AppService keep-alive.** Guaranteed cadence and suspended Wasm execution remain later scope. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #696](https://github.com/leptoskit/leptoskit/pull/696)

### Fixes

#### Apple plugin and JNI builds on current toolchains

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #718](https://github.com/leptoskit/leptoskit/pull/718)

The Apple plugin packages now build and test against the current Xcode SDK. Wire readers no longer declare a `bytes` method that shadows their storage, wire-case helpers have distinct names, calendar validation is shared, EventKit events with no identifier fail honestly, camera photo delegates are retained behind a lock, Security keys use the CF bridge, print uses the current AppKit HTML and PDF APIs, and ActivityKit code is iOS-only. Delegate conformances are main-actor isolated, and the tests import internal wire types with `@testable` and supply a real presentation owner.

The JNI smoke test now finds the JDK through `JAVA_HOME`, invokes the macOS SDK compiler through `xcrun`, and stubs the 3D, timer, and DevTools entry points that the application bridge links.

- **P3 plugin quality: fix background_task Apple Auto construction and honesty, asset Kotlin NUL parity, age_range docs, and tighter wire tests.** _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #706](https://github.com/leptoskit/leptoskit/pull/706)

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0)

### Features

#### leptoskit_plugin_background_task: named OS task registration

`leptoskit_plugin_background_task` registers and unregisters named background tasks, reports Available/Restricted/Unavailable, and completes an in-flight invocation. Distinct from `leptoskit_plugin_background` AppService keep-alive. Guests enable `guest` and reuse `guest::BackgroundTask`.
