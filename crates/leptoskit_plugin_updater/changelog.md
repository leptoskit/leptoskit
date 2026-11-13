# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-13)

### Features

- **Electron desktop parity, first wave.** Desktop hosts get Electron-shaped lifecycle APIs without embedding Chromium: `power_monitor` for suspend/resume/lock/AC events and Windows idle queries, `deep_link` for inbound URL/file activations and association descriptors, `updater` hooks with pluggable backends (separate from code push), and second-launch argument handoff on `single_instance`. Docs map Electron concepts to leptoskit plugins and name the remaining gaps. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #580](https://github.com/leptoskit/leptoskit/pull/580)

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-07)

### Features

#### leptoskit_plugin_updater: native updater hooks

_Owner:_ [@ifiokjr](https://github.com/ifiokjr)

Installed-application updater hooks (check, download, install-and-restart, status) with pluggable backends. Distinct from `leptoskit_codepush` guest/Wasm updates. Ships `UnavailableBackend` and `MemoryBackend`; production feeds remain host-owned.
