<h1 align="center">leptoskit_plugin_device_motion</h1>

<p align="center">
  <strong>leptoskit_plugin_device_motion</strong> is the leptoskit plugin for fused device-motion readings: user acceleration, gravity, rotation rate, and attitude on Android device axes.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_device_motion"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_device_motion"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_device_motion"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_device_motion.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_device_motion"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_device_motion/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://img.shields.io/github/actions/workflows/ci.yml/badge.svg?branch=main"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

One fused sample carries user acceleration (m/s²), gravity (m/s²), rotation rate (rad/s), and attitude alpha/beta/gamma (radians around Z/X/Y) on Android axes everywhere: a phone lying face up reads about +9.8 m/s² on gravity's z. Apple CoreMotion samples are scaled to SI and negated to match. Browser `DeviceMotionEvent` rotation rates in degrees per second become radians (beta→x, gamma→y, alpha→z); attitude degrees become radians.

Select `device_motion` in the app's native plugin list and add this crate as a direct Cargo dependency. Automatic registration installs the Swift or Kotlin handler. iOS uses `CMMotionManager` device-motion updates. Android waits for linear acceleration, gravity, gyroscope, and rotation-vector events. Desktop hosts report unavailable. Browsers combine `DeviceMotionEvent` with `DeviceOrientationEvent`.

`guest::DeviceMotion` resolves the plugin once. Call `available().await` to query hardware. `next(Interval::new(16)?).await` waits for one fresh fused sample. One outstanding read per plugin is accepted; drop cancels acquisition. Repeated calls have gaps and are not lossless recording.

References: [Expo DeviceMotion](https://docs.expo.dev/versions/latest/sdk/devicemotion/), [W3C DeviceOrientation Event](https://www.w3.org/TR/orientation-event/), [Android motion sensors](https://developer.android.com/develop/sensors-and-location/sensors/sensors_motion).

## How it fits

<!-- {=p_device_motionSummary} -->

Add `leptoskit_plugin_device_motion` as a direct dependency and select the `device_motion` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Linux hosts register it where it has a Linux Rust implementation; generated Windows and terminal hosts currently omit runtime plugins, so use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::DeviceMotion` rather than resolving the plugin on every call. iOS uses `CoreMotion` device motion, Android fuses accelerometer, gravity, gyroscope and rotation-vector sensors, desktop has no sensor, and the browser uses `DeviceMotionEvent` with `DeviceOrientationEvent` for attitude.

<!-- {/p_device_motionSummary} -->

## Use it

```rust,ignore
use leptoskit_plugin_device_motion::guest::DeviceMotion;
use leptoskit_sensor::Interval;

let sensor = DeviceMotion::new()?;
let sample = sensor.next(Interval::new(16)?).await?;
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_device_motion)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
