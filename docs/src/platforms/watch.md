# Watch UIs

<Callout kind="warning" title="Experimental">

Wear OS and watchOS support is early. The shared API, host stubs, and validation budgets are in place; packaging, simulator/device install, and live tile or complication updates are not. Use this to design watch surfaces, not to ship.

</Callout>

A phone screen is the wrong starting point for a watch. Wear OS tiles, watchOS glances, and face complications are tiny ambient surfaces with their own lifecycles. leptoskit gives them a dedicated API under `leptoskit::watch` that still reuses Tailwind styling and the shared primitives (`View`, `Text`, `Button`, …) inside a hard layout budget.

## The problem it solves

If you mount a phone file route on a watch, you get unreadably small chrome, widgets watches do not have (password fields, long scrollers), and no place for tiles or complications. `WatchApp` describes the surfaces a watch actually has:

| Surface             | Role                                | Typical budget        |
| ------------------- | ----------------------------------- | --------------------- |
| `WatchPage`         | Interactive watch app page          | ~32 nodes, short text |
| `WatchTile`         | Wear OS tile / watchOS accessory    | ~12 nodes             |
| `WatchGlance`       | Smart Stack / glance preview + body | ~8 nodes              |
| `WatchComplication` | Face slot                           | ~4 nodes              |

Navigation is crown- and swipe-sized: `Single`, `Stack`, `Pager`, or `ListDetail` — not the phone file router.

## Example

```rust
use leptoskit::prelude::*;
use leptoskit::watch::*;

fn steps_wear() -> WatchApp {
	WatchApp::new("steps", WatchKind::WearOs)
		.navigation(WatchNavigation::Stack)
		.page(
			WatchPage::new(
				"home",
				view! {
					<View class="items-center justify-center gap-2 p-2">
						<Text class="text-sm">"Steps"</Text>
						<Text class="text-2xl font-bold">"8,432"</Text>
						<Button>"Open"</Button>
					</View>
				},
			)
			.title("Steps"),
		)
		.tile(WatchTile::new(
			"steps-tile",
			view! { <Text>"8.4k"</Text> },
		))
		.complication(WatchComplication::new(
			"steps-face",
			ComplicationFamily::CircularSmall,
			view! { <Text>"8k"</Text> },
		))
}
```

The same vocabulary works for watchOS with `WatchKind::WatchOs` and `WatchGlance` instead of (or beside) tiles. Content still uses leptoskit elements and classes; the container enforces the budget before a host presents it.

## Sensor-first wearables (`WearableApp`)

Trackers and biometric collectors spend most of their time with the screen off. `WearableApp` lets you declare the wrist streams, sampling policy (`Passive`, `Balanced`, `Continuous`, `Workout`), and delivery mode (`Direct`, `Companion`, or `DirectWithCompanionFallback`) first, and either render an automatic budget-checked status page, tile/glance, and complication from `WearableStatus` or pass a custom `WatchApp`:

```rust
use leptoskit::watch::*;

let tracker = WearableApp::new("quant-watch", WatchKind::WearOs)
	.streams(&[
		WearableStream::HeartRate,
		WearableStream::Steps,
		WearableStream::OxygenSaturation,
		WearableStream::WearState,
	])
	.policy(SamplingPolicy::Passive)
	.delivery(DeliveryMode::DirectWithCompanionFallback)
	.auto_status_surface();
```

## Platforms

| Platform                                    | Feature     | Host crate / extension         | Status                                           |
| ------------------------------------------- | ----------- | ------------------------------ | ------------------------------------------------ |
| Android Wear OS (Pixel Watch, Galaxy Watch) | `wearos`    | `leptoskit_wearos` + Android   | Experimental Gradle APK + runtime plugins        |
| Apple watchOS                               | `watchos`   | `leptoskit_watchos` + Apple    | Experimental Xcode build + runtime plugins       |
| Huawei Watch (HarmonyOS Wearable)           | `harmonyos` | `leptoskit_platform_harmonyos` | Experimental HAP (`device-types = ["wearable"]`) |

Pair with `leptoskit_plugin_wearable` (direct wrist sensors), `leptoskit_plugin_health` (`HealthKit`, Health Connect, HarmonyOS Health), `leptoskit_plugin_watch_sync` (phone ↔ watch link), and `leptoskit_plugin_ble` (BLE GATT heart-rate straps and rings).

## What is refused on purpose

Watch surfaces reject phone-oriented widgets such as `ScrollView`, `TextInput`, `PasswordInput`, `TextArea`, `CanvistEditor`, `Checkbox`, `Radio`, `Slider`, `Rive`, and `Scene3d`. Deep or wordy trees fail validation against the surface budget. That is intentional: a complication is not a settings screen.

## Watch hardware, Always-On Display, workouts, and companion sync

Beyond UI surfaces, `leptoskit::watch` models the hardware and OS lifecycles shared across watchOS and Wear OS:

| Feature                      | API                                                                                 | watchOS / Wear OS mapping                                            |
| ---------------------------- | ----------------------------------------------------------------------------------- | -------------------------------------------------------------------- |
| Digital Crown & rotary bezel | `CrownConfig`, `CrownEvent`, `RotaryState`                                          | `digitalCrownRotation` / Wear rotary scroll with detent haptics      |
| Always-On Display (AOD)      | `AmbientModeState`, `AmbientPolicy`                                                 | `isLuminanceReduced` / `AmbientLifecycleObserver` burn-in protection |
| Smart Stack & Tile relevance | `RelevanceHint`                                                                     | watchOS Smart Stack relevance windows & Wear tile hints              |
| Live workout sessions        | `WorkoutSession`, `WorkoutActivity`, `WorkoutMetrics`                               | `HKWorkoutSession` / Health Services `ExerciseClient`                |
| Phone-watch companion sync   | `CompanionLink`, `CompanionMessage`, `CompanionDeliveryMode`, `MemoryCompanionLink` | `WatchConnectivity` (`WCSession`) / Wear Data Layer                  |
| Haptics & wrist gestures     | `WatchHaptic`, `WatchGesture`, `WatchCapabilities`                                  | `WKHapticType`, Double Tap, Wrist Flick, Action Button               |

```rust
use leptoskit::watch::CrownConfig;
use leptoskit::watch::CrownEvent;
use leptoskit::watch::RotaryState;
use leptoskit::watch::WorkoutActivity;
use leptoskit::watch::WorkoutLocation;
use leptoskit::watch::WorkoutSession;

let mut dial = RotaryState::new(CrownConfig::new(0.0, 60.0, 5.0).unwrap(), 15.0);
let detents = dial.apply(CrownEvent {
	delta: 10.0,
	velocity: 2.0,
	idle: false,
});
assert_eq!(detents, 2);

let mut run = WorkoutSession::prepare(WorkoutActivity::Running, WorkoutLocation::Outdoor);
run.start().unwrap();
run.pause().unwrap();
run.resume().unwrap();
run.end().unwrap();
```

## Permissions and entitlements

Documented beside each platform page:

- [Wear OS](wearos.md): watch feature flag, standalone metadata, tile service, complication data source, Wear Health Services & Samsung Health Sensor SDK.
- [watchOS](watchos.md): Watch App target, `HKWorkoutSession`, `WidgetKit` glances, complications, `WatchConnectivity`.
- [HarmonyOS](harmonyos.md): `device-types = ["wearable"]`, `@kit.SensorServiceKit`, `@kit.HealthServiceKit`.
