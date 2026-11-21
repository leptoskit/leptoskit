# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-21)

### Features

#### Add menu roles, iOS menu bar, and signed updater feeds

_Owner:_ Ifiok Jr. · _Introduced in:_ [b53298d](https://github.com/leptoskit/leptoskit/commit/b53298d6db24da853e8c2a3d53fa2f6df303b489)

- `leptoskit_plugin_menu` adds standard OS `MenuRole` predefined items (`Role(MenuRole::Copy)`, `About`, `Quit`, `Undo`, `Redo`, …), mutually-exclusive `Radio` items, `ContextMenu()` / `ContextMenuHost` / `MemoryContextMenuHost`, and `leptoskit_plugin_menu::ios` for iOS & iPadOS `UIMenuBuilder` menu bars (`IosMenuBar`, `IosMenuSection`, `IosMenuPlacement`) and `UIKeyCommand` hardware shortcuts.
- `leptoskit_plugin_updater::feed` adds signed release-manifest desktop auto-updating (`ReleaseManifest`, `PlatformArtifact`, `TargetPlatformId`, `UpdateChannel`, `UpdatePolicy`, `SemVer`, `IntegrityVerifier` with built-in FIPS 180-4 SHA-256 verification, `FeedTransport`, and `FeedBackend`).

- **Electron desktop parity, first wave.** Desktop hosts get Electron-shaped lifecycle APIs without embedding Chromium: `power_monitor` for suspend/resume/lock/AC events and Windows idle queries, `deep_link` for inbound URL/file activations and association descriptors, `updater` hooks with pluggable backends (separate from code push), and second-launch argument handoff on `single_instance`. Docs map Electron concepts to leptoskit plugins and name the remaining gaps. _Owner:_ Ifiok Jr. · _Introduced in:_ [e4009fd](https://github.com/leptoskit/leptoskit/commit/e4009fd59c9cbe0bc1ebb4391484882e74d0374e)

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-07)

### Features

#### leptoskit_plugin_updater: native updater hooks

_Owner:_ [@ifiokjr](https://github.com/ifiokjr)

Installed-application updater hooks (check, download, install-and-restart, status) with pluggable backends. Distinct from `leptoskit_codepush` guest/Wasm updates. Ships `UnavailableBackend` and `MemoryBackend`; production feeds remain host-owned.
