# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-25)

### Features

#### Add platform targeting and capability queries

_Owner:_ Ifiok Jr. · _Introduced in:_ [fee46c0](https://github.com/leptoskit/leptoskit/commit/fee46c0e80a311139a43d5b33ea3f559aae84df6)

`leptoskit::target` adds a shared runtime and compile-time platform API (`TargetPlatform`, `PlatformFamily`, `PlatformSwitch`, `PlatformCapabilities`, `Capability`, `CapabilitySupport`, `with_platform`, and `on_ios` / `on_android` / `on_mobile` / `on_macos` / `on_windows` / `on_linux` / `on_desktop` / `on_apple` / `on_web` / `on_extension` / `on_watch`) so universal apps can cleanly opt into platform-specific features and test every platform branch from any host.

`leptoskit_build::platforms()` now also recognizes `harmonyos`, `wearos`, `watchos`, and `quest` alongside the existing platform features.

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_build: one app, platform modules

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_build` is the build-script helper behind `platform_module!`: it records which platform-specific modules exist and cfg-selects them while one app keeps a single module tree.

Generated apps call it from `build.rs`. It lets a shared module be replaced by a `.macos.rs`, `.ios.rs`, or `.web.rs` file without changing the imports in the rest of the app.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
