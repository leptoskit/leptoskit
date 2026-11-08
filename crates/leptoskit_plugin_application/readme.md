<h1 align="center">leptoskit_plugin_application</h1>

<p align="center">
  <strong>leptoskit_plugin_application</strong> is the leptoskit plugin for installed host application identity: ID, display name, version, build number, signing certificate, install source, and install and update times.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_application"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_application"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_application"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_application.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_application"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_application/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

The installed host application's identity and install record: ID, localized display name, release version, build number, build signature, installer store, and install and update times. Apple implementations read `Bundle.main`; Android reads the running application's `PackageManager` record. This is the host's identity even when the interpreted guest is updated independently. It covers what Flutter's [`package_info_plus`](https://pub.dev/packages/package_info_plus) reports.

Add this crate as a direct dependency and `"application"` to `native.plugins` for automatic Apple/Android registration. On Rust desktop hosts, construct `Application::new(&Snapshot { ... })` with the host's packaging metadata. This validates and caches the encoded reply. The library does not use its own Cargo package name or version as the app's identity. `executable_times()` gives a Rust host the running executable's creation and modification times to use as install and update times, as `package_info_plus` does on Windows and Linux. Apple desktop bundles use the same Swift implementation as iOS.

| Field                            | iOS                                                                                                                                    | macOS                                      | Android                                                                                      | Rust host and browser |
| -------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------ | -------------------------------------------------------------------------------------------- | --------------------- |
| `id`, `name`, `version`, `build` | Bundle ID, `CFBundleDisplayName` or `CFBundleName`, `CFBundleShortVersionString`, `CFBundleVersion`                                    | Same as iOS                                | Package name, application label, `versionName`, version code                                 | Supplied by the app   |
| `build_signature`                | `None`                                                                                                                                 | `None`                                     | SHA-256 of the certificate that signed the APK, 64 uppercase hex digits                      | Supplied by the app   |
| `installer_store`                | Heuristic: `com.apple` (App Store), `com.apple.testflight`, `com.apple.development` (Xcode, ad hoc, enterprise), `com.apple.simulator` | `None`                                     | Package that initiated the install, such as `com.android.vending`; `None` for `adb` installs | Supplied by the app   |
| `install_time`                   | Creation of the app's Documents directory                                                                                              | Same, for sandboxed apps; `None` otherwise | `firstInstallTime`                                                                           | Supplied by the app   |
| `update_time`                    | Modification of the app bundle                                                                                                         | Modification of the app bundle             | `lastUpdateTime`                                                                             | Supplied by the app   |

The iOS installer store reads the App Store receipt's file name (`receipt` or `sandboxReceipt`) and whether the bundle carries `embedded.mobileprovision`, which App Store and TestFlight builds do not. Apple does not document either signal, so treat the value as a hint, not proof of purchase. `package_info_plus` reports every sandbox receipt as TestFlight; this crate separates development installs.

The Documents directory is created on first launch after install, survives updates, and is reset by deleting and reinstalling the app, so it dates the current installation. Unsandboxed Mac apps have no container to date; `package_info_plus` falls back to the bundle's modification time there, which this crate does not present as an install time.

On Android, after a signing-key rotation the build signature is the certificate the APK is currently signed with. `package_info_plus` reports the oldest certificate in the rotation history instead.

Browsers expose no equivalent installed-app identity. `package_info_plus` fetches a `version.json` that Flutter's build writes; a leptoskit web app has no such generated file, so browser applications construct their own `Snapshot` from build-time values such as `env!("CARGO_PKG_VERSION")` in the app crate. Document titles, origins, and this library's version are not substituted for native package metadata. Missing fields are `None`. No device IDs, vendor IDs, install referrers or release channel guesses are included.

### Wire format

Snapshot version 2 is documented on the crate root: four identity strings, the build signature, the installer store, then the install and update times as optional milliseconds since 1970. The decoder rejects unknown versions, malformed signatures, non-canonical optional times, invalid UTF-8, oversized strings (4096 bytes; 256 for the installer) and trailing bytes. Version 1 replies from older native hosts still decode, with the new fields unknown; a guest older than version 2 cannot read a newer host's reply, so rebuild guests with the host.

### Tests

`cargo test -p leptoskit_plugin_application` covers the codec and the desktop helper. `swift test` in this directory runs `apple-tests/` with fake facts, the installer heuristic, and the real test-runner bundle. `tests/android/ApplicationPluginTest.kt` runs in the generated Android plugin suite and checks the real package record and signing certificate. The Rust, Swift and Kotlin tests assert the same golden bytes.

## How it fits

<!-- {=p_applicationSummary} -->

Add `leptoskit_plugin_application` as a direct dependency and select the `application` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Linux hosts register it where it has a Linux Rust implementation; generated Windows and terminal hosts currently omit runtime plugins, so use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::Application` rather than resolving the plugin on every call. Apple reads `Bundle.main`, Android reads the running package record, and a Rust desktop host supplies its own packaging metadata; browsers have no equivalent installed-app identity. Native snapshots also carry the install source, install and update times, and on Android the signing certificate's SHA-256 digest.

<!-- {/p_applicationSummary} -->

## Use it

```rust,ignore
let app = leptoskit_plugin_application::guest::Application::new()?;
let identity = app.snapshot().await?;
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_application)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
