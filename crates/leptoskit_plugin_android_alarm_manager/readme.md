<h1 align="center">leptoskit_plugin_android_alarm_manager</h1>

<p align="center">
  <strong>leptoskit_plugin_android_alarm_manager</strong> is the leptoskit plugin for Android <code>AlarmManager</code> alarms that fire even when the app isn't running.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_android_alarm_manager"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_android_alarm_manager"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_android_alarm_manager"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_android_alarm_manager.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_android_alarm_manager"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_android_alarm_manager/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

An app often needs to do something at a set time: sync every hour, remind someone at 7:30, refresh a token before it expires. A timer in the app only runs while the app does. Android's `AlarmManager` fires at the requested time even after the app has been closed, and can wake the device to do it.

This plugin is a port of Flutter's [`android_alarm_manager_plus`](https://github.com/fluttercommunity/plus_plugins/tree/main/packages/android_alarm_manager_plus). It schedules one-shot and repeating alarms by integer ID, with the same options: `exact`, `wakeup`, `allow_while_idle`, `alarm_clock` and `reschedule_on_reboot`. When an alarm fires the plugin records it in a store on the device, so the guest receives it whether or not it was running at the time. An alarm marked `headless` goes further: if no guest is running, the plugin starts the app's guest without any window so it can handle the alarm, then stops it.

| Platform                           | What happens                                                                                                                                         |
| ---------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| Android                            | Real `AlarmManager` alarms, a durable store of scheduled and fired alarms, rescheduling after a reboot, and headless launches for `headless` alarms. |
| iOS, macOS                         | `Unavailable`. Apple platforms don't let an app run its own code at a time it chooses (see below).                                                   |
| Windows, Linux, HarmonyOS, browser | `Unavailable`.                                                                                                                                       |

### Android details

- **Exact alarms.** On Android 12 and later, `exact` and `alarm_clock` alarms need the exact-alarm permission. The plugin declares `SCHEDULE_EXACT_ALARM`, which Android 14 and later deny by default; `status()` reports whether it's allowed, and `request_exact_alarm_permission()` opens the "Alarms & reminders" setting. Without it, scheduling fails with `PermissionDenied`, unless the alarm sets `inexact_fallback`, in which case it's scheduled inexactly and the reply says so. Apps whose main job is an alarm clock or calendar may declare `USE_EXACT_ALARM` in their own manifest instead; Google Play restricts that permission.
- **Repeating alarms** are re-armed each time they fire, so `exact` and `allow_while_idle` apply to every firing. Android runs them at most once a minute. A device that slept through several periods fires once and moves on to the next period.
- **Reboots.** Android forgets every alarm when the device restarts. On `BOOT_COMPLETED` the plugin arms the alarms marked `reschedule_on_reboot` again and forgets the rest; an alarm whose time passed while the device was off fires straight away. When the exact-alarm permission is granted, every stored alarm is armed again. Force-stopping the app also clears its alarms until the next reboot or permission grant.
- **The fired-alarm queue** keeps up to 64 alarms; beyond that the oldest are dropped and counted. Each fired alarm carries the ID, the due and delivery times, and the params it was scheduled with. It stays on the device until the guest acknowledges it, which happens on the guest's next `next()`, `drain()` or `acknowledge()` call, so an app that is killed while handling an alarm sees it again.
- **Headless launches** start the guest that shipped in the app, with no window, for at most 8 seconds, while the alarm's broadcast holds the device awake. The guest's startup code runs as usual, so call `handle_launch` early. Plugins that need an Activity, such as permission prompts, return `Unavailable` there. Only one headless guest runs at a time, none starts while the app's own guest is running (that guest receives the alarm instead), and none starts in an app that takes over-the-air updates: the headless guest must be the same code the app runs, so those alarms wait in the queue for the next launch.

### Why there is nothing for iOS

iOS doesn't let an app run its own code at a time it picks. A scheduled local notification can show a message at a set time, but the app's code doesn't run until someone taps it; leptoskit's [notifications plugin](https://docs.rs/leptoskit_plugin_notifications) posts notifications today, and scheduled delivery is planned there rather than here. Background tasks (`BGTaskScheduler`) run when iOS decides, not at a requested time. Pretending otherwise would make alarms silently late, so this plugin reports `Unavailable` on Apple platforms.

## How it fits

<!-- {=p_android_alarm_managerSummary} -->

Add `leptoskit_plugin_android_alarm_manager` as a direct dependency and select the `android_alarm_manager` plugin in `native.plugins`; the generated Android host discovers it from Cargo metadata and declares its alarm and boot receivers and its `SCHEDULE_EXACT_ALARM` and `RECEIVE_BOOT_COMPLETED` permissions. Guests enable the `guest` feature and keep one `guest::AndroidAlarmManager`. It schedules one-shot and repeating Android `AlarmManager` alarms by integer ID, with exact, wakeup, allow-while-idle, alarm-clock and reschedule-on-reboot options, and keeps scheduled and fired alarms in an on-device store that survives process death. Guests wait for or drain fired alarms, and an alarm marked `headless` starts the guest without UI when the app isn't running. iOS, macOS, Windows, Linux, HarmonyOS and the browser return `Unavailable`: they have no equivalent that runs app code at a set time.

<!-- {/p_android_alarm_managerSummary} -->

## Use it

Add the crate to the Android host and the guest, and select it in `leptoskit.toml`:

```toml
[native]
plugins = ["android_alarm_manager"]
```

Schedule an hourly sync that survives restarts and runs even when the app is closed, then handle it in both places it can arrive. This runs on Android; elsewhere `AndroidAlarmManager::new()` returns `Unavailable`.

```rust,ignore
use std::time::Duration;

use leptoskit_plugin_android_alarm_manager::Options;
use leptoskit_plugin_android_alarm_manager::guest::AndroidAlarmManager;

const SYNC: i32 = 1;

async fn start() -> Result<(), leptoskit_bridge::Error> {
	let alarms = AndroidAlarmManager::new()?;

	// Started without a window for an alarm? Handle it and stop.
	if alarms.handle_launch(|alarm| async move { sync(alarm.id).await }).await? {
		return Ok(());
	}

	alarms
		.periodic(SYNC, Duration::from_secs(60 * 60), Options {
			wakeup: true,
			reschedule_on_reboot: true,
			headless: true,
			..Options::default()
		})
		.await?;

	// Alarms that fired while the app wasn't running, then each new one.
	for alarm in alarms.drain().await? {
		sync(alarm.id).await;
	}

	loop {
		let alarm = alarms.next().await?;
		sync(alarm.id).await;
	}
}
```

`cancel(id)` removes an alarm, `scheduled()` lists the ones the device holds, and `Request` builds a one-shot or periodic alarm with params of up to 1 KiB that come back in every `FiredAlarm`.

## Wire format

All integers are little-endian. Times are `i64` milliseconds since the Unix epoch, from 0 to 9999-12-31; durations are `i64` milliseconds. Byte strings are a `u32` length and the bytes. Every decoder, in Rust and Kotlin, rejects truncated or trailing bytes, unknown codes and values outside these ranges; both test suites check the same golden bytes.

| ID | Method                     | Request                                                                | Reply                                                                     |
| -- | -------------------------- | ---------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| 0  | `STATUS`                   | Empty                                                                  | `u32` SDK, `u8` exact allowed, `u8` headless, `u32` queued, `u64` dropped |
| 1  | `SCHEDULE`                 | `i32` id, `u8` kind, `u32` options, `i64` time, `i64` interval, params | `u8` timing, `i64` first firing                                           |
| 2  | `CANCEL`                   | `i32` id                                                               | `u8` was scheduled                                                        |
| 3  | `LIST`                     | Empty                                                                  | `u32` count, then each alarm                                              |
| 4  | `REQUEST_EXACT_PERMISSION` | Empty                                                                  | `u8` allowed                                                              |
| 5  | `NEXT`                     | `u64` cursor                                                           | One fired alarm, waiting for it if needed                                 |
| 6  | `DRAIN`                    | `u64` cursor, `u32` up to 64                                           | `u32` count, then each fired alarm                                        |
| 7  | `LAUNCH`                   | Empty                                                                  | `u8` 0, or 1 and the fired alarm that started the guest                   |
| 8  | `FINISH`                   | `u64` sequence                                                         | Empty                                                                     |

Kinds: 0 one-shot after a delay (0 to 3,660 days), 1 one-shot at a time, 2 periodic (interval from one minute to 366 days; time is the first firing, or -1 for one interval from now). Options: exact=1, wakeup=2, allow-while-idle=4, alarm-clock=8, reschedule-on-reboot=16, headless=32, inexact-fallback=64 (only with exact or alarm-clock). Timing: 0 exact, 1 inexact, 2 alarm clock. A scheduled alarm is `i32` id, `i64` next firing, `i64` interval (0 for one-shot), `u32` options, `u8` timing and params. A fired alarm is `u64` sequence (from 1, never reused), `i32` id, `i64` interval, `i64` due time, `i64` delivery time and params. `NEXT` and `DRAIN` first remove the fired alarms up to the cursor. At most 128 alarms are scheduled at once; one more fails with `Busy`.

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_android_alarm_manager)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
