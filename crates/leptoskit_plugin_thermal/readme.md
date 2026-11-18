<h1 align="center">leptoskit_plugin_thermal</h1>

<p align="center">
  <strong>leptoskit_plugin_thermal</strong> is the leptoskit plugin for the device's thermal state, with cancellable change observation.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_thermal"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_thermal"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_thermal"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_thermal.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_thermal"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_thermal/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

A phone that is running hot slows itself down, and work an app starts then (a sync, an upload, a model run) makes it hotter. This plugin tells the app how hot the device is, so it can put heavy work off until the device has cooled.

The state uses the four levels Apple defines: nominal, fair, serious and critical, or unknown where the platform cannot say. Android reports seven throttling levels; they are folded into those four, and the original status is kept in `Snapshot::android_status` so nothing is lost. `State::is_throttled` is true from serious upwards, where the system is already reducing performance.

Battery charge, charging state and Low Power Mode are in [`leptoskit_plugin_battery`](../leptoskit_plugin_battery/readme.md); use the two together for a full power picture.

Add the crate to the native host and select `"thermal"` in `native.plugins`. The generator discovers `ThermalPlugin` from Cargo package metadata. No permission prompt is needed.

Dropping `wait_for_change` cancels the native request and removes its observer. Only one waiter may be active per native plugin; another returns `Busy`. Each wait installs its observer before sampling, so a change between calls is detected. The result is the latest state: intermediate transitions can be coalesced, and a slow consumer does not accumulate an event backlog. Closing the host cancels an outstanding wait.

| Platform                | Snapshot                                     | Change observation                              |
| ----------------------- | -------------------------------------------- | ----------------------------------------------- |
| iOS, macOS Swift host   | `ProcessInfo.thermalState`                   | `ProcessInfo.thermalStateDidChangeNotification` |
| Android 10+ (API 29)    | `PowerManager.getCurrentThermalStatus`       | `PowerManager.addThermalStatusListener`         |
| Android 8–9             | Unknown                                      | `Unavailable`                                   |
| Linux, Windows, browser | Not registered; the guest gets `Unavailable` | Not registered                                  |

The iOS simulator reports the host Mac's process state, usually nominal; it is not a device measurement.

Snapshot payloads are exactly 8 bytes: two little-endian `u32` fields for the shared state (`0` unknown, `1` nominal, `2` fair, `3` serious, `4` critical) and Android's `THERMAL_STATUS_*` value (`0` to `6`, `u32::MAX` where there is none). Decoders reject invalid values, an Android status that contradicts the state, and trailing bytes. Method `0` reads a snapshot; method `1` waits for a snapshot different from the supplied 8-byte previous value.

API reference: [Apple `ProcessInfo.ThermalState`](https://developer.apple.com/documentation/foundation/processinfo/thermalstate) and [Android `PowerManager` thermal status](https://developer.android.com/reference/android/os/PowerManager#getCurrentThermalStatus()).

## How it fits

<!-- {=p_thermalSummary} -->

Add `leptoskit_plugin_thermal` as a direct dependency and select the `thermal` plugin in `native.plugins`; generated iOS, Android, and macOS hosts discover it from Cargo metadata. Guests enable the `guest` feature and reuse `guest::Thermal` rather than resolving the plugin on every call. A platform that cannot report its thermal state answers `Unknown` instead of pretending the device is cool, and Android's finer throttling status is kept alongside the shared four-level state.

<!-- {/p_thermalSummary} -->

## Use it

```rust,ignore
let thermal = leptoskit_plugin_thermal::guest::Thermal::new()?;
let mut current = thermal.snapshot().await?;

loop {
    if current.state.is_throttled() {
        pause_heavy_work();
    }

    current = thermal.wait_for_change(current).await?;
}
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_thermal)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
