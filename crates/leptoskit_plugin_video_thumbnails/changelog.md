# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-21)

### Features

- **Add `leptoskit_plugin_video_thumbnails` to extract one JPEG frame from an app-owned video URI at a time offset (`AVAssetImageGenerator` / `MediaMetadataRetriever`).** Multi-frame extraction remains later scope. _Owner:_ Ifiok Jr. · _Introduced in:_ [88c385c](https://github.com/leptoskit/leptoskit/commit/88c385c35553dfab40a5e0270b848bd2805f2367)

### Fixes

#### Apple plugin and JNI builds on current toolchains

_Owner:_ Ifiok Jr. · _Introduced in:_ [fdf5868](https://github.com/leptoskit/leptoskit/commit/fdf586892a1411704c419bfd5069014d4ce5aaa2)

The Apple plugin packages now build and test against the current Xcode SDK. Wire readers no longer declare a `bytes` method that shadows their storage, wire-case helpers have distinct names, calendar validation is shared, EventKit events with no identifier fail honestly, camera photo delegates are retained behind a lock, Security keys use the CF bridge, print uses the current AppKit HTML and PDF APIs, and ActivityKit code is iOS-only. Delegate conformances are main-actor isolated, and the tests import internal wire types with `@testable` and supply a real presentation owner.

The JNI smoke test now finds the JDK through `JAVA_HOME`, invokes the macOS SDK compiler through `xcrun`, and stubs the 3D, timer, and DevTools entry points that the application bridge links.

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0)

### Features

#### leptoskit_plugin_video_thumbnails: one-frame JPEG extraction

`leptoskit_plugin_video_thumbnails` extracts one JPEG frame from an app-owned video URI at a time offset with quality 1–100. Multi-frame extraction remains later scope. Guests enable `guest` and reuse `guest::VideoThumbnails`.
