# Platform-specific features

Write your screens once, then opt into what each device and browser uniquely supports: an iOS/iPadOS menu bar or Live Activity, an Android Quick Settings tile or intent, a macOS/Windows/Linux system tray or always-on-top overlay, a watch Digital Crown, or a Chrome/Firefox side panel.

## The problem it solves

A universal app should not collapse to the lowest common denominator. Without a first-class way to inspect the active platform and branch cleanly, platform-specific code turns into scattered `#[cfg(...)]` attributes and hand-rolled fallbacks that are hard to test across targets.

`leptoskit::target` gives every screen and library crate one consistent vocabulary for:

- detecting the active [`TargetPlatform`](../platforms/readme.md),
- selecting platform- or family-specific values with `PlatformSwitch`,
- conditionally building platform-specific UI with `on_ios`, `on_android`, `on_desktop`, `on_extension`, and `on_watch`,
- querying what OS-level capabilities a target supports with `PlatformCapabilities`, and
- swapping whole modules per target with `platform_module!`.

## Example: one screen with platform-native extras

```rust
use leptoskit::prelude::*;
use leptoskit::target::Capability;
use leptoskit::target::PlatformCapabilities;
use leptoskit::target::PlatformSwitch;
use leptoskit::target::on_desktop;
use leptoskit::target::on_ios;

#[component]
pub fn WorkspaceHeader() -> Element {
	let subtitle = PlatformSwitch::new()
		.ios("Press Cmd+K or open the iPadOS menu bar")
		.android("Pin a Quick Settings tile for one-tap capture")
		.desktop("Keep this window pinned above other apps")
		.extension("Open the side panel beside any tab")
		.default("Shared across every platform")
		.resolve();

	let caps = PlatformCapabilities::current();
	let has_live_status = caps.is_available(Capability::LiveActivities);
	let desktop_hint = on_desktop(|| "Tray & always-on-top ready");
	let ios_hint = on_ios(|| "Menu bar & Live Activities ready");

	view! {
		<View class="gap-2 p-4">
			<Text class="text-lg font-bold">"Workspace"</Text>
			<Text class="text-sm">{subtitle}</Text>
			<Text class="text-xs">
				{format!(
					"Live status: {has_live_status} · desktop: {} · ios: {}",
					desktop_hint.unwrap_or("no"),
					ios_hint.unwrap_or("no"),
				)}
			</Text>
		</View>
	}
}
```

This screen compiles and runs on web, iOS, Android, macOS, Windows, Linux, the terminal, browser extensions, HarmonyOS (experimental), and Wear OS / watchOS (experimental).

## Precedence in `PlatformSwitch`

`PlatformSwitch` resolves the most specific branch first:

1. Exact platform (`.ios`, `.android`, `.macos`, `.windows`, `.linux`, `.web`, `.extension`, `.harmonyos`, `.wearos`, `.watchos`, `.quest`, `.tui`).
2. Platform family (`.mobile`, `.desktop`, `.watch`, and `.web` as a fallback for `.extension`).
3. Ecosystem grouping (`.apple` for iOS, macOS, and watchOS; `.native` for every native host).
4. `.default(...)`.

Use `.try_resolve()` when a branch is optional and returns `Option<T>`.

## Testing every platform from one host

`with_platform` temporarily overrides `TargetPlatform::current()` on the current thread and restores the previous value automatically, even on panic. That lets unit tests exercise iOS, Android, macOS, Windows, Linux, watch, and extension branches on any development machine:

```rust
use leptoskit::target::PlatformSwitch;
use leptoskit::target::TargetPlatform;
use leptoskit::target::with_platform;

let badge = with_platform(TargetPlatform::Android, || {
	PlatformSwitch::new()
		.ios("WidgetKit")
		.android("Glance")
		.default("Standard")
		.resolve()
});
assert_eq!(badge, "Glance");
```

## Querying OS capabilities

`PlatformCapabilities::for_platform(platform)` returns `CapabilitySupport::Supported`, `CapabilitySupport::Conditional`, or `CapabilitySupport::Unavailable` for each `Capability` variant:

- `Supported`: first-class OS feature on that platform (for example, `Capability::IosMenuBar` on iOS, `Capability::AndroidQuickSettingsTiles` on Android, `Capability::AlwaysOnTopWindow` on macOS/Windows/Linux, `Capability::DigitalCrown` on watchOS/Wear OS).
- `Conditional`: depends on the host, browser family, or OS version (for example, `Capability::ExtensionSidePanel` works on Chromium and Firefox but not Safari; `Capability::GlobalShortcuts` works on Linux X11 but not Wayland; `Capability::LiveActivities` on Android maps to Android 16+ Live Updates).
- `Unavailable`: the OS does not expose that concept.

## Swapping whole modules with `platform_module!`

When a platform needs separate dependencies or larger native integration code, call `leptoskit_build::platforms()` in your app's `build.rs` and declare a platform module:

```rust,ignore
leptoskit::platform_module!(pub mod chrome = "chrome");
```

Create `src/chrome.rs` as the shared fallback, and add any target overrides next to it: `src/chrome.ios.rs`, `src/chrome.android.rs`, `src/chrome.macos.rs`, `src/chrome.windows.rs`, `src/chrome.linux.rs`, `src/chrome.web.rs`, `src/chrome.extension.rs`, `src/chrome.harmonyos.rs`, `src/chrome.wearos.rs`, `src/chrome.watchos.rs`, `src/chrome.quest.rs`, or `src/chrome.tui.rs`.

## Where each platform's APIs live

| Platform family        | Guide                              | Key APIs                                                                                                                                                           |
| ---------------------- | ---------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| iOS & iPadOS           | [iOS](ios.md)                      | Menu bar (`leptoskit_plugin_menu::ios`), Live Activities & Dynamic Island, WidgetKit & Control Center, App Intents & Spotlight, Handoff                            |
| Android                | [Android](android.md)              | Quick Settings tiles, App Widgets, Intents, Bubbles, Material You, Picture-in-Picture & overlay windows, Live Updates                                              |
| watchOS & Wear OS      | [Watch UIs](watch.md)              | Pages, tiles, glances, complications, Digital Crown / rotary input, Always-On Display, workouts, companion sync                                                    |
| macOS, Windows & Linux | [Electron concepts](electron.md)   | Native menus & context menus, system tray, always-on-top overlays, frameless/vibrancy windows, menu-bar/background-only mode, auto-updater                         |
| Browser extensions     | [Browser extensions](extension.md) | Browser detection (`Chrome`, `Edge`, `Brave`, `Opera`, `Arc`, `Vivaldi`, `Firefox`, `Safari`), side panel / sidebar, windows, offscreen, omnibox, native messaging |
