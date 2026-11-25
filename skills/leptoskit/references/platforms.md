# Platform-specific features

leptoskit lets you keep one shared UI tree and add platform-native capabilities for iOS, Android, macOS, Windows, Linux, watchOS, Wear OS, HarmonyOS, and Chromium/Firefox/Safari browser extensions.

Use `https://leptoskit.dev/llms.txt` (structured chapter index) or `https://leptoskit.dev/llms-full.txt` (full plain-Markdown book) when you need deeper reference details for any platform or plugin.

## Detecting the platform and capabilities

Use `leptoskit::target` for both compile-time (`TargetPlatform::current()`) and runtime branching (always use builder syntax in agent-written code):

```rust
use leptoskit::prelude::*;
use leptoskit::target::Capability;
use leptoskit::target::PlatformCapabilities;
use leptoskit::target::PlatformSwitch;
use leptoskit::target::TargetPlatform;

pub fn screen() -> Element {
	let caps = PlatformCapabilities::current();
	let has_live_updates = caps.is_available(Capability::LiveActivities);

	let subtitle = PlatformSwitch::new()
		.ios("iOS layout")
		.android("Android layout")
		.desktop("Desktop layout")
		.extension("Extension popup or sidebar")
		.default("Shared layout")
		.resolve();

	View()
		.class("gap-4 p-4")
		.child(Text(subtitle))
		.children(has_live_updates.then(|| Text("Live updates supported")))
}
```

Helpers in `leptoskit::target`:

- `TargetPlatform`: `Web`, `Extension`, `Ios`, `Android`, `Macos`, `Windows`, `Linux`, `Harmonyos`, `Wearos`, `Watchos`, `Quest`, `Tui`, `Server`, `Shared`.
- `PlatformCapabilities::for_platform(p)`, `p.supports(cap)`, `caps.is_supported(cap)`, `caps.is_available(cap)`, `caps.supported_capabilities()`, `caps.available_capabilities()`: query platform support without hand-rolled `#[cfg]` trees.
- `Capability::ALL`, `cap.as_str()`, `cap.has_builtin_plugin()`, `cap.plugin_crate()`, `cap.runtime_plugin_name()`, `cap.cargo_feature()`, `cap.summary()`: inspect what leptoskit wires natively vs. what requires host-supplied native code.
- `on_ios`, `on_android`, `on_mobile`, `on_macos`, `on_windows`, `on_linux`, `on_desktop`, `on_apple`, `on_watch`, `on_extension`, `on_web`, `with_platform`: ergonomic conditional and testing helpers.
- `leptoskit::platform_module!`: load separate module files per platform (`foo.ios.rs`, `foo.android.rs`, `foo.extension.rs`, …) when implementations diverge.
- Build scripts: `leptoskit_build::platforms()` inspects the crate's platform Cargo features (`web`, `ios`, `android`, `macos`, `windows`, `linux`, `harmonyos`, `wearos`, `watchos`, `quest`, `tui`, `extension`) and sets `LEPTOSKIT_PLATFORM` for both `platform_module!` and `TargetPlatform::current()`.

## Opting in through `leptoskit` Cargo features

Enable individual features (`live-activities`, `home-widgets`, `app-shortcuts`, `android-intent`, `menu`, `window-state`, `updater`) or bundles (`mobile-surfaces`, `desktop-extras`) on `leptoskit` to import from `leptoskit::platform::*`. Enabling `leptoskit/guest` forwards `guest` to each enabled runtime plugin. For native host auto-registration, also add each runtime plugin crate as a direct dependency of the platform host package and list its runtime name under `native.plugins` in `leptoskit.toml`.

## iOS and iPadOS

Enable plugins in `leptoskit.toml` under `[native] plugins = [...]` or through host registration.

| Capability                                          | Plugin / API                                                                                    | Notes                                                                                                                                                                                                                                                                                                                                                                           |
| --------------------------------------------------- | ----------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Live Activities & Dynamic Island                    | `leptoskit::platform::live_activities` / `leptoskit_plugin_live_activities` (`live-activities`) | ActivityKit `start`/`update`/`end`/`list`, `DynamicIslandLayout` via `ContentState::with_dynamic_island`, `start_scheduled`/`update_scheduled` (`ActivitySchedule`), `wait_push_token`, and iOS 17.2+ `wait_push_to_start_token`. Requires `NSSupportsLiveActivities`, a Widget Extension rendering `LeptosKitLiveActivityAttributes`, and Push Notifications for APNs updates. |
| Home, Lock Screen, StandBy & Control Center widgets | `leptoskit::platform::home_widgets` / `leptoskit_plugin_home_widgets` (`home-widgets`)          | `set`/`get`/`clear`/`reload`, `set_timeline`/`get_timeline` (`TimelineSchedule`), and iOS 18+ `set_control`/`get_control`/`reload_controls` (`ControlCenterControl`). Reads `LeptosKitAppGroup` from `Info.plist` automatically; Swift extensions decode via `LeptosKitWidgetStore.readSnapshot` / `readControl`.                                                               |
| iPadOS / Catalyst menu bar & shortcuts              | `leptoskit::platform::menu::ios` / `leptoskit_plugin_menu` (`ios::IosMenuBar`)                  | `MenuBar`, `Submenu`, `Command`, `Check`, `Radio`, `Role`, `MenuRole`, `ContextMenu`, and `UIMenuBuilder` / `UIKeyCommand` descriptors.                                                                                                                                                                                                                                         |
| Quick Actions, CoreSpotlight, Handoff & Share       | `leptoskit::platform::app_shortcuts` / `leptoskit_plugin_app_shortcuts` (`app-shortcuts`)       | `UIApplicationShortcutItem` (`set`/`clear`/`consume_launch`), `CoreSpotlight` (`index_spotlight`/`delete_spotlight`), `NSUserActivity` Handoff (`start_user_activity`/`end_user_activity`), `consume_incoming_share` (via `noteIncomingShare`), plus `AppIntentDefinition`, `MultiSceneConfig`, `PencilPreference` in `integrations`.                                           |
| Sign-in, Passkeys & Hardware Keys                   | `passkeys`, `web-auth`, `device-keys`, `local-authentication`                                   | `ASAuthorizationController`, Face ID / Touch ID, Secure Enclave keys.                                                                                                                                                                                                                                                                                                           |
| Health, BLE, NFC, Watch & XR                        | `health`, `ble`, `nfc`, `watch-sync`, `wearable`, `xr`                                          | HealthKit, CoreBluetooth, CoreNFC NDEF read, WatchConnectivity, ARKit + `Scene3d`.                                                                                                                                                                                                                                                                                              |
| Media, Camera, Live Photo & Maps                    | `camera`, `video`, `audio`, `recorder`, `live-photo`, `maps`, `sharing`                         | `AVCaptureSession`, `AVPlayer`, Core Audio, Live Photos, MapKit, `UIActivityViewController`.                                                                                                                                                                                                                                                                                    |
| Privacy, Store Review, Screen Capture & Push        | `tracking-transparency`, `store-review`, `screen-capture`, `notifications`, `badge`             | ATT prompt, `SKStoreReviewController`, capture observation, APNs enrollment & local notifications, icon badge.                                                                                                                                                                                                                                                                  |

### iOS capabilities not yet wired as native plugins

`Capability::has_builtin_plugin()` returns `false` for these — use a host-registered Swift `NativePlugin`:

- StoreKit 2 in-app purchases & subscriptions
- Apple Pay (`PassKit`) & Apple Wallet passes
- CallKit / PushKit VoIP calling UI
- VisionKit document / barcode / Live Text scanner
- `SFSpeechRecognizer` speech-to-text (`speech` is TTS only) & `MPNowPlayingInfoCenter` media controls
- Apple Intelligence / Foundation Models / Core ML on-device inference
- App Clips, SharePlay (`GroupActivities`), CloudKit / iCloud Key-Value Sync, AirPlay, `CLMonitor` geofencing, FamilyControls / Screen Time shields
- Live App Attest (`app-integrity` currently ships an `Unavailable` stub)
- Periodic `background-task` (returns `Unavailable` on Apple until declared `BGTaskScheduler` IDs land; use `background` for basic iOS background tasks)

## Android

| Capability                                       | Plugin / API                                                                                                                                                                                  | Notes                                                                                                                                                                                                                                                             |
| ------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| App Widgets, Glance & Quick Settings tiles       | `leptoskit::platform::home_widgets` / `leptoskit_plugin_home_widgets` (`home-widgets`)                                                                                                        | `set`/`get`/`clear`/`reload`/`installed`, `set_timeline`/`get_timeline`, and `set_quick_tile`/`get_quick_tile` (`QuickSettingsTile`). Kotlin `AppWidgetProvider` / Glance / `TileService` subclasses read via `HomeWidgetsPlugin.readSnapshot` / `readQuickTile`. |
| Android 16+ Live Updates & ongoing notifications | `leptoskit::platform::live_activities` / `leptoskit_plugin_live_activities` (`live-activities`)                                                                                               | `post_android_live_update` / `end_android_live_update`: API 36 `Notification.ProgressStyle` + promoted ongoing chip with automatic standard ongoing progress notification fallback on API < 36.                                                                   |
| Intents & Exact Background Alarms                | `leptoskit::platform::android_intent` (`android_intent`), `android_alarm_manager`                                                                                                             | Launch/broadcast/resolve intents; exact/wakeup/reboot alarms that can wake the guest headlessly for up to 8 seconds (without code push).                                                                                                                          |
| PiP, Draw Over Apps & Foldables                  | `picture-in-picture`, `posture`                                                                                                                                                               | Activity PiP, `SYSTEM_ALERT_WINDOW` system overlay, Jetpack WindowManager folding posture.                                                                                                                                                                        |
| Shortcuts, Incoming Shares, Badges & NFC         | `leptoskit::platform::app_shortcuts` (`app-shortcuts`), `badge`, `nfc`, `sharing`                                                                                                             | Dynamic `ShortcutManager` shortcuts, automatic incoming `ACTION_SEND` share capture (`consume_incoming_share`), NDEF reads, badges.                                                                                                                               |
| Health, BLE, Wearables, Security, MDM & Capture  | `health`, `ble`, `wearable`, `watch-sync`, `device-keys`, `passkeys`, `local-authentication`, `device-policy`, `app-usage`, `screen-capture`, `store-review`, `background`, `background-task` | Health Connect, Android BLE, Wear OS Data Layer, StrongBox, Credential Manager, Device Admin, UsageStatsManager, `FLAG_SECURE`, WorkManager.                                                                                                                      |
| Media, Camera, Maps & ARCore                     | `camera`, `video`, `audio`, `recorder`, `maps`, `xr`                                                                                                                                          | CameraX, Media3, OpenGL ES/Vulkan `Scene3d` + ARCore, Google Maps.                                                                                                                                                                                                |

### Android capabilities not yet wired as native plugins

`Capability::has_builtin_plugin()` returns `false` for these (or only typed descriptors exist):

- Google Play Billing (in-app purchases & subscriptions) and Google Play In-App Updates (`AppUpdateManager`)
- Google Pay & Google Wallet passes
- `MediaSession` / lock-screen media transport controls
- ML Kit barcode & document scanner, `SpeechRecognizer` (`speech` is TTS only)
- Posting `Notification.BubbleMetadata` & Material You dynamic theme extraction (typed `BubbleConfig` / `DynamicColorRole` descriptors exist in `app-shortcuts`)
- Jetpack `AppSearch` (`index_spotlight` returns `Unavailable` on Android), `GeofencingClient`, Google Cast, `ConnectionService` VoIP
- Predictive back swipe animations beyond `OnBackInvokedCallback` routing, Android Auto, Gemini Nano (`AICore`), Play Instant, Cloud Backup
- Live Play Integrity attestation (`app-integrity` currently ships an `Unavailable` stub)

## Desktop (macOS, Windows, Linux)

- **Windowing & background apps:** `leptoskit::platform::window_state::desktop` (`WindowOptions`, `WindowLevel`, `WindowChrome`, `VibrancyMaterial`, `CloseBehavior`, `ActivationPolicy`, `TaskbarProgress`, `DockAttention`, `WindowController`), `NativeMode::Background`, and `[native.window]` in `leptoskit.toml`.
- **Menus & context menus:** `leptoskit::platform::menu` (`MenuBar`, `Submenu`, `Command`, `Check`, `Radio`, `Role`, `MenuRole`, `ContextMenu`).
- **Auto-updates:** `leptoskit::platform::updater` (`Updater`, `FeedBackend`, `ReleaseManifest`, `UpdateChannel`, `UpdatePolicy`).
- **Tray, lifecycle & system hooks:** `tray`, `single_instance`, `power_monitor`, `deep_link`, `shell`, `dialog`, `clipboard`, `notifications`.

## Watches (watchOS, Wear OS, HarmonyOS)

- Use `WearableApp` and `leptoskit::watch` (`crown`, `aod`, `workout`, `companion`, `interaction`, pages, tiles, glances, complications) alongside the `wearable`, `health`, `watch-sync`, and `ble` plugins.

## Browser extensions (Chrome, Edge, Brave, Opera, Arc, Vivaldi, Firefox, Safari)

- See [Browser extensions](extensions.md) and `leptoskit_extension::browser` (`BrowserFamily`, `ExtensionCapabilities`, `side_panel`, `windows`, `offscreen`, `omnibox`, `native_messaging`).
