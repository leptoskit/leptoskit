<h1 align="center">leptoskit_plugin_app_usage</h1>

<p align="center">
  <strong>leptoskit_plugin_app_usage</strong> is the leptoskit plugin for app usage: Usage Access status, typed usage events, foreground sessions and screen-time summaries through Android UsageStatsManager, and lock and unlock events on iOS and macOS.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_app_usage"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_app_usage"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_app_usage"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_app_usage.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_app_usage"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_app_usage/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

How has this device been used, and when? Android answers through `UsageStatsManager`, gated by Usage Access, a special grant the device's owner switches on in the system's own Usage Access screen. Nothing grants it at install time, and Device Admin does not grant it either. The plugin declares `android.permission.PACKAGE_USAGE_STATS`, never prompts, and gives the app two calls to work with that screen: `usage_access_status()` reports the AppOps mode, and `open_usage_access_settings()` opens the screen. Reads without the grant return `PermissionDenied`.

With the grant, three queries run over a closed `UsageRange` of Unix milliseconds:

- `events(range, max)` returns typed `UsageEvent`s oldest first, at most `max` (1–1,024). The kinds are an activity resuming or pausing (each naming its package), the screen becoming interactive or not, the keyguard showing or hiding, and device startup and shutdown. Other system event types are skipped. When the range holds more events than `max`, the reply stops there; narrow the range or page from the last timestamp.
- `sessions(range)` returns per-app foreground `UsageSession`s with the launcher's label, clipped to the range. An app already in the foreground when the range starts begins at the range's start, and one still open at its end closes at the earlier of the end and now. More than 4,096 sessions return `TooLarge`.
- `summary(range)` returns the range's screen-on milliseconds, unlock count (keyguard dismissals) and per-app foreground totals sorted by package, built from the same events.

iOS and macOS give third-party apps no per-app usage. Screen Time data reaches only DeviceActivity report extensions, which render it and cannot send it back to the app, so `sessions` and `summary` return `Unavailable` and `open_usage_access_settings` has no screen to open. `usage_access_status` answers `Granted`, because the events Apple platforms do support need no grant. There `events` is a cancellable wait: it returns the `Locked` and `Unlocked` transitions buffered while the plugin is alive (protected-data availability on iOS, the screen-lock notifications on macOS), waiting for the next one when the buffer is empty. The buffer holds 64 transitions and drops the oldest first, and the range is checked but ignored. Browsers have no equivalent and return `Unavailable`.

The wire uses methods 0 status, 1 open settings, 2 events, 3 sessions and 4 summary. A range is two little-endian `i64`s; events append a `u32` cap. Strings are a `u32` byte length and UTF-8 bytes, and an absent package is the length `u32::MAX`. Rust rejects truncated, oversized, unsorted and trailing payloads.

Tests use synthetic events and codec fixtures; they never open Usage Access or read the device's usage log. The Android wire and session logic is covered by `android-tests/AppUsagePluginTest.kt`, and the Apple buffer and framing by `apple-tests`. Real usage logs, the Usage Access screen and lock transitions need separate device testing.

References: [Android UsageStatsManager](https://developer.android.com/reference/android/app/usage/UsageStatsManager), [Android UsageEvents.Event](https://developer.android.com/reference/android/app/usage/UsageEvents.Event), [Apple DeviceActivity](https://developer.apple.com/documentation/deviceactivity), [Apple protected data notifications](https://developer.apple.com/documentation/uikit/uiapplication/protecteddatadidbecomeavailablenotification).

## How it fits

<!-- {=p_appUsageSummary} -->

Add `leptoskit_plugin_app_usage` as a direct dependency and select the `app-usage` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Windows, Linux, and terminal hosts currently omit runtime plugins, so use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::AppUsage` rather than resolving the plugin on every call. Android reports typed usage events, foreground sessions and screen-time summaries from Usage Access, which the app's owner grants on the system's own screen; iOS and macOS give third-party apps no per-app usage, so sessions and summaries are unavailable and events are the lock and unlock transitions buffered while the app is alive.

<!-- {/p_appUsageSummary} -->

## Use it

```toml
# In a native host, select `app-usage` in `native.plugins`.
[dependencies]
leptoskit_plugin_app_usage = { version = "0.3", features = ["guest"] }
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_app_usage)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
