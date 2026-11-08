<h1 align="center">leptoskit_plugin_cellular</h1>

<p align="center">
  <strong>leptoskit_plugin_cellular</strong> is the leptoskit plugin for carrier name, MCC/MNC, radio generation, and VoIP capability snapshots with explicit Unknown / Unavailable / Restricted fields.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_cellular"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_cellular"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_cellular"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_cellular.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_cellular"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_cellular/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Reads the permission-light cellular identity the platform exposes: carrier display name, mobile country / network codes, radio generation (2G–5G), and VoIP allowance when CoreTelephony reports it. Fields that the OS cannot answer stay `Unknown`, hosts without a radio answer `Unavailable`, and SecurityException / policy denial becomes `Restricted`. Values are never invented from Wi‑Fi or browser NetworkInformation.

Add the crate to the native host and select `"cellular"` in `native.plugins`. The generator discovers `CellularPlugin` from Cargo package metadata. This MVP declares **no** Android `READ_PHONE_STATE` / location permissions; when those are required for a field, the adapter reports Restricted instead of prompting.

Guest applications enable the `guest` feature.

| Platform              | Snapshot source                                      | Notes                                                                                           |
| --------------------- | ---------------------------------------------------- | ----------------------------------------------------------------------------------------------- |
| iOS                   | CoreTelephony `CTTelephonyNetworkInfo` / `CTCarrier` | Carrier, MCC/MNC, radio access technology, `allowsVOIP`; empty SIM → Unknown                    |
| Android               | `TelephonyManager` without phone-state permission    | Operator name / MCC-MNC / data network type; SecurityException → Restricted; VoIP stays Unknown |
| macOS Swift host      | Unavailable                                          | No cellular radio                                                                               |
| macOS, Windows, Linux | `desktop::Cellular`                                  | All fields Unavailable                                                                          |
| Browser               | `web::snapshot()`                                    | All fields Unavailable; `effectiveType` belongs to the network plugin                           |

Change observation, subscription lists, and permission-gated deep telephony APIs remain out of scope.

API reference: [Expo Cellular](https://docs.expo.dev/versions/latest/sdk/cellular/), [CoreTelephony](https://developer.apple.com/documentation/coretelephony), [TelephonyManager](https://developer.android.com/reference/android/telephony/TelephonyManager).

## How it fits

<!-- {=p_cellularSummary} -->

Add `leptoskit_plugin_cellular` as a direct dependency and select the `cellular` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Linux hosts register it where it has a Linux Rust implementation; generated Windows and terminal hosts currently omit runtime plugins, so use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::Cellular` rather than resolving the plugin on every call. Carrier name, MCC/MNC, radio generation, and `VoIP` allowance stay Unknown, Unavailable, or Restricted when the platform does not expose them — values are never invented.

<!-- {/p_cellularSummary} -->

## Use it

```rust,ignore
let cellular = leptoskit_plugin_cellular::guest::Cellular::new()?;
let snapshot = cellular.snapshot().await?;
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_cellular)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
