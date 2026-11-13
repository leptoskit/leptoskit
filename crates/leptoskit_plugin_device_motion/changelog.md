# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-13)

### Features

- **Add `leptoskit_plugin_device_motion` for fused attitude, user acceleration, and rotation-rate readings on iOS, Android, and the browser.** Desktop hosts report unavailable. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #684](https://github.com/leptoskit/leptoskit/pull/684)

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0)

### Features

#### leptoskit_plugin_device_motion: fused device-motion readings

`leptoskit_plugin_device_motion` queries hardware availability and returns one fused sample of user acceleration, gravity, rotation rate, and attitude on Android axes. Guests enable the `guest` feature and reuse `guest::DeviceMotion`.
