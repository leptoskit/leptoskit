# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-13)

### Features

#### Every transport a connection uses

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #624](https://github.com/leptoskit/leptoskit/pull/624)

A network reading now lists every transport the active connection uses, as Flutter's `connectivity_plus` does, instead of naming only one. `Snapshot::transports` is a `Transports` set of `Transport` values — Wi-Fi, cellular, ethernet, Bluetooth, VPN, satellite, and other — so an Android app on a VPN over Wi-Fi sees both, and an app on satellite data sees satellite alongside cellular. `Snapshot::connection` still names the primary one, and the set always contains it.

Android reports every transport `NetworkCapabilities` lists, including Wi-Fi Aware as Wi-Fi and satellite on Android 15 and later. iOS and macOS report Wi-Fi, cellular, ethernet, and other from `NWPath`, plus satellite when the path is ultra-constrained (iOS and macOS 26 and later); they have no VPN or Bluetooth interface type, so a VPN tunnel shows as other. The browser maps `NetworkInformation.type`, and desktop Rust still infers no transport.

The wire snapshot grows from 32 to 36 bytes, and every side refuses transport bits it does not define. Rebuild the native app together with the guest; a guest built with this release can't read an older shell's snapshots, and the reverse.

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_plugin_network: network state

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_network` is the leptoskit plugin for network state and demand-driven change observation.

Add `leptoskit_plugin_network` as a direct dependency and select the `network` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Network` rather than resolving the plugin on every call. Apple uses `NWPathMonitor`, Android uses `ConnectivityManager`, desktop reports interface state, and the browser uses `navigator.onLine` plus `NetworkInformation`; an active connection never claims a server is reachable.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
