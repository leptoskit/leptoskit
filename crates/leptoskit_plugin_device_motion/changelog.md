# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0)

### Features

#### leptoskit_plugin_device_motion: fused device-motion readings

`leptoskit_plugin_device_motion` queries hardware availability and returns one fused sample of user acceleration, gravity, rotation rate, and attitude on Android axes. Guests enable the `guest` feature and reuse `guest::DeviceMotion`.
