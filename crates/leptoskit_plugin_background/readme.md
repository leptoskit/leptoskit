<h1 align="center">leptoskit_plugin_background</h1>

<p align="center">
  <strong>leptoskit_plugin_background</strong> keeps a LeptosKit app working in the background: an Android foreground service with a persistent notification and a scheduled wake of the app's Rust <code>AppService</code>, plus iOS background app-refresh and processing tasks.
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_background"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_background"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_background.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_background"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_background/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Background execution for a LeptosKit app. On Android it runs a foreground `Service` with a persistent, low-importance notification and wakes the app's `AppService` on a configurable schedule, with a WorkManager fallback and an opt-in boot receiver. On iOS it registers `BGTaskScheduler` app-refresh and processing tasks that wake the `AppService`. macOS, desktop, and the browser report `Unavailable`: those platforms already own their process lifetime, so there is nothing to keep alive.

| Platform        | Implementation                            | Scope                                                                                |
| --------------- | ----------------------------------------- | ------------------------------------------------------------------------------------ |
| Android         | Kotlin foreground `Service` + WorkManager | Persistent notification, periodic `AppService` wake, boot restart, battery exemption |
| iOS             | Swift `BGTaskScheduler`                   | App-refresh and processing tasks that wake the `AppService`                          |
| macOS           | Unavailable                               | Desktop apps already run                                                             |
| Windows / Linux | Unavailable                               | Desktop apps already run                                                             |
| Browser         | Unavailable                               | The page's lifetime is its background lifetime                                       |

iOS gives no guarantee of continuous execution: the system schedules background tasks at its own discretion, so an iOS app cannot promise a fixed wake interval. Android 14 (API 34) restricts which foreground service types may start from the background, and the system may still kill a service it judges abusive.

## Native registration and use

Enable the `guest` feature in Wasm guest code, add `leptoskit_plugin_background` as a direct dependency of each native host, and select `background` in `native.plugins`. The generated host discovers the crate from Cargo metadata. The guest client caches its numeric plugin ID and reuses `guest::Background`.

Android requests `POST_NOTIFICATIONS`, `FOREGROUND_SERVICE` and the matching `FOREGROUND_SERVICE_*` permissions, plus `RECEIVE_BOOT_COMPLETED` and `REQUEST_IGNORE_BATTERY_OPTIMIZATIONS`, in the generated manifest, and declares the service's `foregroundServiceType`. Declaring a permission does not grant it; the app must still request `POST_NOTIFICATIONS` at run time through the notifications plugin or its own flow. The boot receiver is opt-in: an app that does not need to survive a reboot leaves it out of its own manifest.

## Binary protocol

All integers are little-endian `u32`. Strings are `u32` byte lengths followed by strict UTF-8. Every decoder rejects trailing bytes, truncated fields, invalid UTF-8 and out-of-domain values before calling a platform API.

| ID | Method                    | Request                                                      | Response                                            |
| -- | ------------------------- | ------------------------------------------------------------ | --------------------------------------------------- |
| 0  | Start                     | Title, text strings, service-type mask `u32`, interval `u32` | Empty                                               |
| 1  | Stop                      | Empty                                                        | Empty                                               |
| 2  | Status                    | Empty                                                        | Running `u32`, last-wake `u32`, battery state `u32` |
| 3  | Request battery exemption | Empty                                                        | Battery state `u32`                                 |

Titles have at most 128 UTF-8 bytes, text 1024, and the wake interval is 1–86400 seconds (default 60). The service-type mask is a bitfield of `dataSync=1`, `health=2`, `microphone=4`, `location=8` and `connectedDevice=16`. Battery state is Unknown=0, Exempted=1 or NotExempted=2.

## How it fits

<!-- {=p_backgroundSummary} -->

Add `leptoskit_plugin_background` as a direct dependency and select the `background` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Android runs a foreground service with a persistent notification and wakes the app's `AppService` on a schedule, with a `WorkManager` fallback and an opt-in boot receiver; iOS registers `BGTaskScheduler` tasks that wake the `AppService`. iOS gives no guarantee of continuous execution, and macOS, desktop, and the browser report `Unavailable`.

<!-- {/p_backgroundSummary} -->

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_background)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
