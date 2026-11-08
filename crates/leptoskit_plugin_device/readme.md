<h1 align="center">leptoskit_plugin_device</h1>

<p align="center">
  <strong>leptoskit_plugin_device</strong> is the leptoskit plugin for device and operating system information, with persistent identifiers and user-assigned names kept behind a separate call.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_device"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_device"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_device"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_device.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_device"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_device/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Device information for native LeptosKit guests, Rust desktop hosts, and browsers: hardware, operating system, memory, storage, and the details only one platform reports. It covers what Flutter's [`device_info_plus`](https://pub.dev/packages/device_info_plus) reports, split into two calls so that reading the device never reads who it belongs to:

- `snapshot()` (method 0) returns a `Snapshot`. It never contains a persistent identifier or a name a person gave the device.
- `identifiers()` (method 1) returns `Identifiers`: the user-assigned device name, the iOS vendor identifier, and the desktop machine identifier. Call it only when you need one of them and disclose why. Each field is `None` where the platform withholds it.

Add this crate as a direct app dependency and `"device"` to `native.plugins` to register the UIKit, AppKit, or Android implementation automatically. Rust desktop hosts register `desktop::Device`; browser apps call `web::snapshot()` directly.

### What the snapshot reports

Every platform fills the common fields it can: manufacturer, brand, model, model identifier, OS name and version, architecture, kernel release, total and available memory, the size of and free space on the volume holding the app's data, logical CPU count, form factor, and simulator status. A `Platform` section then carries one platform's own details.

| Platform              | Common fields                                                                                                                                                                      | Platform section                                                                                                                                                                                                                                                                                                         |
| --------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| iOS                   | `UIDevice` model and system name/version, `iPhone15,2` model identifier (the simulated model in the simulator), Darwin kernel release, free memory pages, data volume, active CPUs | `Apple`: localized model, OS build, iOS-app-on-Mac, Mac Catalyst, iOS-app-on-Vision (iOS 26.1 and later)                                                                                                                                                                                                                 |
| macOS (Swift or Rust) | `macOS` and its version, `hw.model` model identifier such as `Mac14,2`, kernel release, memory, data volume, CPUs                                                                  | `Apple`: OS build, nominal CPU frequency where the Mac reports one (Intel Macs; Apple silicon reports none)                                                                                                                                                                                                              |
| Android               | `Build` manufacturer/brand/model, release, preferred ABI, kernel release, `ActivityManager` total and available memory, data volume, CPUs, recognized emulator signature           | `Android`: SDK and preview SDK level, security patch, codename, incremental, release, base OS, board, bootloader, device, display, fingerprint, hardware, host, `Build.ID`, product, tags, type, build time, all three ABI lists, system features (sorted, at most 1024), low-RAM device class, current low-memory state |
| Windows (Rust)        | OS name/version, architecture, build number as kernel version, memory, data volume, CPUs                                                                                           | `Windows`: major/minor version, build number, update build revision, product name (corrected to Windows 11 on build 22000 and later), display version, edition, release ID, build lab strings                                                                                                                            |
| Linux (Rust)          | OS name/version, architecture, kernel release, memory, data volume, CPUs                                                                                                           | `Linux`: every `os-release` field `device_info_plus` reads (`NAME`, `VERSION`, `ID`, `ID_LIKE`, `VERSION_CODENAME`, `VERSION_ID`, `PRETTY_NAME`, `BUILD_ID`, `VARIANT`, `VARIANT_ID`), falling back to `lsb-release`                                                                                                     |
| Browser               | Low-entropy User-Agent Client Hints platform as OS name, `hardwareConcurrency` as CPU count                                                                                        | `Web`: brands, mobile flag, maximum touch points                                                                                                                                                                                                                                                                         |

Browsers send the detailed client hints — model, architecture, bitness, platform version, full browser versions, form factors, WoW64 — only when asked. `web::high_entropy_values()` asks; `web::snapshot()` never does. Browsers without client hints (Firefox, Safari) leave those fields unknown, and a `Permissions-Policy` refusal returns `PermissionDenied`. The user-agent string is never parsed, and the coarse, privacy-reduced `navigator.deviceMemory` is not presented as total RAM.

Android phone/tablet classification follows the configured smallest screen width (600 dp threshold), so resizing and desktop modes can change it; a desk dock leaves a phone a phone. Android reports `simulator: Some(true)` for the `goldfish`/`ranchu` emulator hardware and `None` otherwise; this is informational, not attestation. iOS simulator detection is a compile-time signal. Desktop virtualization is not inferred.

Memory means bytes exposed to the operating system, not the app's budget. Available memory changes constantly; iOS counts only free pages. Free disk is what this app can still write, so it excludes space reserved for the system. Desktop reads refresh RAM and the CPU list only and do not enumerate processes or sample CPUs.

### Identifiers

| Field         | iOS                                                                                                                                         | Android                                          | macOS                                                     | Windows               | Linux             |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------ | --------------------------------------------------------- | --------------------- | ----------------- |
| `device_name` | `UIDevice.name`, only with the `com.apple.developer.device-information.user-assigned-device-name` entitlement from iOS 16; otherwise `None` | `Settings.Global.DEVICE_NAME`                    | Computer name (Swift); host name (Rust)                   | Host name             | Host name         |
| `vendor_id`   | `identifierForVendor`                                                                                                                       | `None`                                           | `None`                                                    | `None`                | `None`            |
| `machine_id`  | `None`                                                                                                                                      | `None`: the serial needs a privileged permission | `IOPlatformUUID`; `None` from Rust inside the App Sandbox | `SQMClient\MachineId` | `/etc/machine-id` |

Apple grants the user-assigned-device-name entitlement on request; add it to the app's entitlements yourself. iOS without it returns the model name, which the plugin reports as `None` rather than as a name. `device_info_plus` also exposes the Windows user name, registered owner and product key; this crate does not read them.

### Wire format

Snapshot version 2 and identifiers version 1 are documented on the crate root. Every reply is strictly validated: known enum and tri-state codes, canonical optional numbers, UTF-8, bounded strings (4096 bytes; identifiers 256) and lists (16 ABIs, `ID_LIKE` entries or brands; 1024 system features), and no trailing bytes. Version 1 snapshots from older native hosts still decode, with newer fields unknown. A guest older than version 2 cannot read a newer host's reply, so rebuild guests with the host.

### Tests

`cargo test -p leptoskit_plugin_device` covers every codec edge, the `os-release` parser, and the real desktop reads on the machine running it. `swift test` in this directory runs `apple-tests/` against fake facts and the real Mac. `tests/android/` holds instrumented tests that the generated Android plugin suite runs on an emulator, and `tests/browser.rs` runs in Chrome with controlled client hints. The Rust, Swift and Kotlin tests assert the same golden bytes.

## How it fits

<!-- {=p_deviceSummary} -->

Add `leptoskit_plugin_device` as a direct dependency and select the `device` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Linux hosts register it where it has a Linux Rust implementation; generated Windows and terminal hosts currently omit runtime plugins, so use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::Device` rather than resolving the plugin on every call. Native guests, Rust desktop hosts, and browsers each return a validated binary snapshot with the fields that platform can actually provide. The snapshot never holds persistent identifiers or names a person gave the device; those come only from the separate `identifiers` call.

<!-- {/p_deviceSummary} -->

## Use it

```rust,ignore
let device = leptoskit_plugin_device::guest::Device::new()?;
let snapshot = device.snapshot().await?;

if let leptoskit_plugin_device::Platform::Android(android) = &snapshot.platform {
	let sdk = android.sdk_int;
}

// Only when you need it, and say so in your privacy disclosures.
let identifiers = device.identifiers().await?;
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_device)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
