<h1 align="center">leptoskit_plugin_health</h1>

<p align="center">
  <strong>leptoskit_plugin_health</strong> is the leptoskit plugin for reading smartwatch and phone health data from Health Connect on Android and HealthKit on iOS.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_health"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_health"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_health"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_health.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_health"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_health/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

A read-only window into the health stores a phone already keeps: heart rate, resting heart rate, heart-rate variability, steps, sleep with its stages, blood oxygen, respiratory rate and exercise sessions, each with the app or device that recorded it, plus paged reads and incremental sync over a stored token.

Select Cargo crate `leptoskit_plugin_health` and runtime `health`; native registration is automatic. Android reads Health Connect (the platform store on Android 14+, the Health Connect app on older versions) and iOS reads HealthKit. macOS, other desktops and browsers report `Unsupported`.

The app owns its permission story. On iOS the CLI adds the HealthKit entitlement from this crate's metadata and the app writes its own `NSHealthShareUsageDescription` purpose string. On Android the CLI merges the `READ_*` health permissions, the Health Connect package visibility entry and the permission-rationale activities this crate declares into the app's manifest; the generated rationale entry finishes immediately, and an app that wants to explain its health-data use declares its own activity for the same action, which generation then leaves untouched.

Ask with `request_permissions`, then read. Health Connect reports grants exactly; HealthKit does not expose read authorization, so its grants return an empty mask with `exact = false`. That means unknown, not denied: request permission and read the records without using the mask to decide whether a read is allowed. Android's optional `READ_HEALTH_DATA_IN_BACKGROUND` and `READ_HEALTH_DATA_HISTORY` permissions are requested through `Extra` flags when the app also declares them in its own manifest, and `features()` says whether the store supports them. Heart-rate variability is reported as RMSSD on Android and SDNN on iOS as distinct data types; they are never converted into each other. Sleep arrives as sessions with stages on Android and as one record per stage on iOS. iOS reads page by counting already-delivered samples, because HealthKit has no page cursors; two samples with identical start and end could swap across a page boundary. Health Connect expires change tokens after 30 days and the reply says so; start over with a fresh token then.

Real watches, permission prompts and the stores themselves have not been read in this implementation; tests are synthetic codec and manifest fixtures.

References: [Health Connect](https://developer.android.com/health-and-fitness/guides/health-connect), [HealthKit](https://developer.apple.com/documentation/healthkit).

## How it fits

<!-- {=p_healthSummary} -->

Add `leptoskit_plugin_health` as a direct dependency and select the `health` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::Health` rather than resolving the plugin on every call. Android reads Health Connect and iOS reads `HealthKit`; the app supplies the Apple purpose string and any background or history manifest permissions it wants to request.

<!-- {/p_healthSummary} -->

## Use it

```rust,ignore
use leptoskit_plugin_health::guest::Health;
use leptoskit_plugin_health::{DataType, Extra};

let health = Health::new()?;
if health.availability().await? == leptoskit_plugin_health::Availability::Available {
    let grants = health
        .request_permissions(&[DataType::HeartRate, DataType::Sleep], Extra::default())
        .await?;
    let page = health.read(DataType::HeartRate, start_ms, end_ms, 200, None).await?;
}
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_health)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
