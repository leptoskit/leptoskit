# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Added

#### leptoskit_plugin_age_range: declared age-range request wire

Queries whether the host can ask, then requests a band around a threshold. Responses are Declined, Unavailable, or Shared with inclusive lower/upper bounds (`upper == 0` means open-ended). Every host returns typed Unavailable until Declared Age Range / Play Age Signals wiring lands. Guests enable `guest` and reuse `guest::AgeRange`.

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-13)

### Features

- **Add `leptoskit_plugin_age_range` for declared age-range capability status and threshold request (Declined / Unavailable / Shared bounds).** Android, desktop, and browser return Unavailable in this MVP; live OS prompts remain later scope. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #697](https://github.com/leptoskit/leptoskit/pull/697)

### Fixes

#### Apple plugin and JNI builds on current toolchains

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #718](https://github.com/leptoskit/leptoskit/pull/718)

The Apple plugin packages now build and test against the current Xcode SDK. Wire readers no longer declare a `bytes` method that shadows their storage, wire-case helpers have distinct names, calendar validation is shared, EventKit events with no identifier fail honestly, camera photo delegates are retained behind a lock, Security keys use the CF bridge, print uses the current AppKit HTML and PDF APIs, and ActivityKit code is iOS-only. Delegate conformances are main-actor isolated, and the tests import internal wire types with `@testable` and supply a real presentation owner.

The JNI smoke test now finds the JDK through `JAVA_HOME`, invokes the macOS SDK compiler through `xcrun`, and stubs the 3D, timer, and DevTools entry points that the application bridge links.

- **P3 plugin quality: fix background_task Apple Auto construction and honesty, asset Kotlin NUL parity, age_range docs, and tighter wire tests.** _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #706](https://github.com/leptoskit/leptoskit/pull/706)
