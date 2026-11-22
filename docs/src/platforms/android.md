# Android

On Android, your screens are drawn with Android views: a `Button` is an `android.widget.Button`, a `TextInput` an `EditText`, a `ScrollView` a `ScrollView`. The app that `leptoskit new` generates is a small Kotlin activity that runs your screens as WebAssembly, so it can also replace them over the air with [code push](../guide/05-code-push.md). Text is measured with the same fonts Android draws it with, so each label and button gets exactly the space it needs.

The generated app runs your screens' code on a thread of its own, not Android's main thread. Updating state, laying the screen out, and measuring text happen there, and the main thread only applies the result to the views. A slow update delays what you draw next, but scrolling, animations, and touches keep running meanwhile.

## What you need

- Java 17 or newer, the Android SDK with platform 36, CMake, and an NDK. Set `ANDROID_HOME` to the SDK; `ANDROID_NDK_HOME` picks a specific NDK.
- The Rust targets for your device or emulator, and for WebAssembly:

```bash
rustup target add aarch64-linux-android x86_64-linux-android wasm32-unknown-unknown
```

## Develop with live reload

```bash
leptoskit dev --platform android
```

`dev` builds the app, installs it on every attached phone and emulator, and opens it. With nothing attached, it boots your first emulator and leaves it running for next time. When you save, it rebuilds your interface and the running app swaps it in. It finds `adb` and the emulator through `ANDROID_HOME`. [Building and serving](../guide/04-building.md#leptoskit-dev) covers the rest.

## Build and run

```bash
leptoskit build --platform android
```

This builds an arm64 debug APK, for phones and for emulators on Apple silicon, at `android/app/build/outputs/apk/debug/app-debug.apk`. Install it on a running emulator or a connected phone:

```bash
adb install -r android/app/build/outputs/apk/debug/app-debug.apk
```

For an x86-64 emulator, build `leptoskit native build android-x86_64` instead. Add `--release` for a release APK, and an App Bundle beside it for Google Play, both signed with your upload key from the `LEPTOSKIT_ANDROID_KEYSTORE` (or `LEPTOSKIT_ANDROID_KEYSTORE_BASE64`), `LEPTOSKIT_ANDROID_KEYSTORE_PASSWORD`, `LEPTOSKIT_ANDROID_KEY_ALIAS`, and `LEPTOSKIT_ANDROID_KEY_PASSWORD` variables, and copied to `.leptoskit/release/android/`. Without them, a release build stops and names what is missing; `--unsigned` builds one unsigned on purpose. See [Signing a release](../guide/24-signing.md). `leptoskit publish play` uploads to Google Play; see [Publishing to the stores](../guide/24-publishing.md#google-play).

You can also open the `android/` folder in Android Studio and run it from there. Build once with the CLI first, and again after you change your app or its plugins: `leptoskit native prepare android` regenerates the Rust inputs without running Gradle.

The `android/` folder is yours: the activity, the manifest, the Gradle files, and `Cargo.toml` for native Rust dependencies such as [runtime plugins](plugins.md). The CLI keeps its generated files in `.leptoskit/android/`, including the R8 keep rules that release builds need for the native library; put rules for your own code in `android/app/proguard-rules.pro`.

## Foldable phones

Folding a Pixel Fold, Galaxy Z Fold, Z Flip, or similar resizes the activity instead of recreating it: the generated manifest lists `orientation|screenSize|screenLayout|smallestScreenSize` in `android:configChanges`, and `LeptosKitView` remeasures on configuration changes. Your guest keeps its state; Tailwind responsive classes and the layout engine see the new size.

To know _how_ the phone is folded — closed on the cover screen, half open as a book or tabletop, or open flat — and where each hinge crosses the window, add the `posture` plugin and watch it as [Foldable phones](../guide/21-foldable-phones.md) shows. Android reports Jetpack WindowManager folding features (state, axis, bounds, occlusion, and `isSeparating`) plus the hinge angle sensor. A fully open continuous foldable reports its crease with `separating = false`, so `panes()` and `is_dual_pane()` stay single-pane until the device is half open or a dual-screen hinge truly splits the window. No Android permission is required; selecting `posture` in `native.plugins` adds `androidx.window:window` to the generated app.

```toml
[native]
plugins = ["posture"]
```

```rust
let posture = RwSignal::new(Posture::default());
let fold = use_posture(posture);
// Dual-pane list/detail when a separating fold crosses the window:
// if fold.is_dual_pane.get() { … panes as percentage widths … }
```

Apps scaffolded before foldable support need `screenLayout|smallestScreenSize` added to `android:configChanges` by hand so folding does not restart the activity.

## The Back gesture

Swiping back from the edge of the screen, or pressing the Back button, goes back one route through your [file router](../guide/10-file-routes.md#deep-links-and-system-back). On the first route there is nothing to go back to, so Back leaves the app as it does in any other app.

Your router answers from its own thread, a moment after the gesture. Two quick Backs are answered in order, so from two routes deep they return to the first route, and from one route deep they go back and then leave. Because only your router knows whether it can go back, the app always takes Back, and Android does not show its preview of the home screen while you swipe.

Apps scaffolded before Back reached the router need three changes, which a new project shows:

- `MainActivity.kt` registers an `OnBackInvokedCallback` in `onCreate` on Android 13 and later, keeps `onBackPressed` for earlier versions, and asks `LeptosKitApplicationView.offerSystemBack` before leaving. Android 16 calls nothing else for an app that targets it, so without the callback every Back leaves the app.
- `AndroidManifest.xml` sets `android:enableOnBackInvokedCallback="true"` on `<application>`, so Android 13, 14, and 15 use the same callback.
- Your app's `lib.rs` calls `navigation.bind_system_back()` and exports `leptoskit_system_back`, as the [file routes guide](../guide/10-file-routes.md#deep-links-and-system-back) shows.

## Test it

Each project includes an instrumented test that taps **Add one** and waits for **Count: 1**. Update it when you change the home screen, then run it with an emulator running:

```bash
cd android
./gradlew connectedDebugAndroidTest
```

## How it differs from macOS

- Transforms and [animations](../guide/32-animations.md) run on Android's animators, springs included. A widget animating out is drawn above its former siblings until it is gone. Only background colors animate; text and border colors jump.
- The widget backend recognizes [taps, long presses, pans, pinches, rotations, and flings](../guide/38-gestures.md) on the view under the finger. A drag that wins cancels the control underneath so it does not also click. The generated Swift and Kotlin renderer does not forward that pointer stream yet, so a gesture there does not run.
- Generated apps also draw `Switch`, `Checkbox`, `Radio`, `Slider`, `LinearProgressIndicator`, and `ActivityIndicator` with Android controls. Switches and checkboxes report their checked state; radio options select within the group your screen controls, and sliders report values on their configured step grid. Tapping outside an editor clears focus and dismisses the keyboard. The keyboard moves to the next enabled field, or submits and dismisses from the last field. [The Back gesture](#the-back-gesture) goes back one route.
- [3D scenes](../guide/33-3d-scenes.md) draw into a transparent `TextureView` with OpenGL ES, scheduled through Choreographer only while the scene changes or animates. The model is composited with the other views: backgrounds stay behind it and controls stay above it, including over a camera picture. Choose Vulkan with the `dev.leptoskit.scene3d.backend` manifest metadata set to `vulkan`. The APK carries the app's `assets/` folder when the app uses `leptoskit`'s `3d` feature. Native scene views send primary-pointer input through `Scene::next_pointer`: positions are fractions of the view and movement uses layout units, so drag sensitivity stays the same across display densities.
- [Rive](../guide/20-rive.md) uploads native vector frames to a reusable Android bitmap. The generated host schedules through Choreographer, stops when idle or offscreen, and treats disabled system animations as reduced motion. Native playback rejects text, image, layout, script, and audio assets.
- A [`Modal`](../guide/07-shared-components.md#show-a-dialog-or-a-menu-above-the-screen) is a dialog and a `Popover` is a popup window, each in a window of its own above the activity. The Back gesture, Escape and a touch outside ask a modal to close, and it stays until your `open` signal closes it. A touch outside a popover asks in the same way. The Back gesture closes a popover first and asks second, so a popover your app keeps open is shown again with the next change to the screen. A dialog needs an activity: a tree mounted in a view with no activity behind it leaves modals down.
- A `Custom` widget kind with no registered native implementation is an empty container, the same as on the other platforms.
- TalkBack gets a smaller set of roles than macOS's accessibility API.
- Italic text and per-edge borders (`border-t`, `border-l-2`, …) paint through Android views the same way they do on the web.
- [Sign-in](../guide/23-sign-in.md#native-apps) uses Credential Manager for passkeys once your site vouches for the app's signing certificate. Other providers open in Chrome's Auth Tab, which needs Chrome 137 or later as the default browser; with another default browser, provider sign-in ends with the outcome `unsupported`. Sign in with Apple uses Apple's web sign-in, in the same Auth Tab.
- `Image` loads [`asset://` pictures](../guide/36-assets.md) from the APK's assets, local files, and `android.resource://` URIs, not URLs.
- The generated app keeps your screen clear of the status and navigation bars. The bars take your root view's background, as a browser paints the page's background across the whole window, with light icons on a dark one. A root without an opaque background leaves the theme's white bars.
- With the backend crate, the host reports the window insets for the system bars and display cutouts, and the [safe-area utilities](../guide/07-shared-components.md#clear-the-status-bar-and-home-indicator) pad by them.
- The [`picture-in-picture`](plugins.md) plugin can enter activity picture-in-picture when the hosting activity declares `android:supportsPictureInPicture="true"` (scaffolded hosts set this), and can hold a system overlay after the user grants `SYSTEM_ALERT_WINDOW` in settings. Declaring the permission in the manifest never grants it; `request_overlay` opens the system grant screen and returns the status at launch—poll `overlay_status` after the user returns.
- [Android intents](#android-intents) (`android_intent`) launch, broadcast, and resolve arbitrary intents with typed extras. Other platforms return `Unavailable`.
- [Device policy](plugins.md#notes-on-specific-plugins) (`device-policy`) can lock the screen, set password rules, disable the camera, or wipe the device after the user enables an app-owned device-admin receiver. Status never prompts. Your `AndroidManifest.xml` must declare `DevicePolicyReceiver` with `BIND_DEVICE_ADMIN`, and `res/xml/device_admin.xml` must list only the policies you call. Google Play restricts Device Admin for many consumer apps; wipe in particular needs a clear enterprise reason.

## Home-screen widgets, Quick Settings tiles, Live Updates, and incoming shares

Enable `leptoskit`'s `mobile-surfaces` feature (or individual features `home-widgets`, `live-activities`, `app-shortcuts`, `android-intent`) in your app crate, add the matching plugin crates to your Android host package, and list their runtime names (`"home-widgets"`, `"live-activities"`, `"app-shortcuts"`, `"android_intent"`) under `native.plugins` in `leptoskit.toml`.

### App Widgets, Jetpack Glance, and Quick Settings tiles

The [`home-widgets`](plugin-inventory.md) plugin stores bounded widget payloads in app-private `SharedPreferences` and triggers `AppWidgetManager` refreshes:

- **Snapshots & multi-entry timelines:** `set` / `get` / `clear`, `set_timeline` / `get_timeline` (`TimelineSchedule`), `reload`, and `installed`. Snapshot URLs must be HTTP(S) or an app deep-link scheme (`javascript:`, `data:`, `file:`, `blob:`, and `vbscript:` are rejected).
- **Quick Settings tiles:** `set_quick_tile` / `get_quick_tile` (`QuickSettingsTile` with `TileState::Active`, `Inactive`, or `Unavailable`, label, subtitle, icon resource name, and action ID).

Your app still owns each `AppWidgetProvider` (or Jetpack Glance receiver) and `TileService` subclass. Read published values directly in Kotlin with `HomeWidgetsPlugin.readSnapshot` and `HomeWidgetsPlugin.readQuickTile`:

```kotlin
import dev.leptoskit.plugins.homewidgets.HomeWidgetsPlugin

// Inside an AppWidgetProvider or GlanceAppWidget:
val snapshot = HomeWidgetsPlugin.readSnapshot(context, "summary")

// Inside an android.service.quicksettings.TileService:
val tile = HomeWidgetsPlugin.readQuickTile(this, "focus-mode")
```

Kind-to-provider matching is best-effort by class name; name providers after their kinds when you need a precise reload. `installed` reports whether any matching provider currently has pinned instances, and returns none when no provider class name matches the kind.

### Android 16+ Live Updates

The [`live-activities`](plugin-inventory.md) plugin wires `post_android_live_update` and `end_android_live_update` on Android: on Android 16 (API 36) and newer it builds a promoted ongoing `Notification.ProgressStyle` with colored `ProgressSegment`s and a short status-bar critical chip, and on older Android versions it falls back automatically to a standard ongoing progress notification on channel `dev.leptoskit.live_updates`.

```rust
use leptoskit::platform::live_activities::AndroidLiveUpdateRequest;
use leptoskit::platform::live_activities::Identifier;
use leptoskit::platform::live_activities::guest::LiveActivities;
use leptoskit::platform::live_activities::island::AndroidLiveUpdate;
use leptoskit::platform::live_activities::island::ProgressSegment;

let update = AndroidLiveUpdate::new(
	"Order #42",
	"Driver is 4 minutes away",
	"4m",
	0.7,
	vec![
		ProgressSegment { length: 70, color_hex: Some("#22C55E".into()) },
		ProgressSegment { length: 30, color_hex: Some("#CBD5E1".into()) },
	],
)?;
let activities = LiveActivities::new()?;
activities
	.post_android_live_update(&AndroidLiveUpdateRequest::new(
		Identifier::new("order-42")?,
		update,
	))
	.await?;
```

### Incoming shares, shortcuts, Bubbles, and Material You

- **Incoming `ACTION_SEND` share target:** when your `AndroidManifest.xml` declares an `ACTION_SEND` intent filter, generated Android hosts capture incoming shared text, URLs, and subjects automatically through `AppShortcutsPlugin`, and your guest drains them via `leptoskit::platform::app_shortcuts::guest::AppShortcuts::consume_incoming_share()`.
- **Launcher shortcuts:** `set`, `clear`, and `consume_launch` publish dynamic `ShortcutManager` shortcuts.
- **Typed Android descriptors:** `AppIntentDefinition`, `BubbleConfig` (`Notification.BubbleMetadata`), `StatusBarConfig`, and `DynamicColorRole` (Material You Monet palette roles) live in `leptoskit::platform::app_shortcuts::integrations`.

## Android intents

Some things only exist on Android: opening one particular settings page, handing a link to a specific app with the extras it expects, or sending a broadcast your own receiver listens for. The [`android_intent`](plugin-inventory.md) plugin sends any intent you describe, as Flutter's `android_intent_plus` does. Enable `leptoskit`'s `android-intent` (or `mobile-surfaces`) feature, add `leptoskit_plugin_android_intent` to your Android host package, and add `"android_intent"` to `native.plugins`:

```rust
use leptoskit::platform::android_intent::guest::AndroidIntent;
use leptoskit::platform::android_intent::{Intent, action};

// Unavailable on the web, iOS, macOS, Windows, Linux, the terminal, HarmonyOS,
// Wear OS, and watchOS: only Android phones and tablets register the plugin.
let intents = AndroidIntent::new()?;
let settings = Intent::new()
	.action(action::APPLICATION_DETAILS_SETTINGS)
	.data("package:com.example.app");
intents.launch(&settings).await?;
```

It can also open an intent through a titled chooser, send broadcasts, start services, ask which activity would handle an intent, and launch `intent:` links. A call returns `Unavailable` when no app handles the intent and `PermissionDenied` when Android refuses it.

Because your screens can be updated over the air, the plugin never lets an intent give another app access to your app's files: it refuses the `FLAG_GRANT_*_URI_PERMISSION` flags and `file:` links, and extras can only be numbers, text, arrays, and nested bundles. It can still reach anything your app's own permissions allow, so only select it when you need it.

On Android 11 and later, asking whether an app can handle an intent only sees apps your manifest declares. Add a `<queries>` element to `android/app/src/main/AndroidManifest.xml` listing the intents or packages you check; launching doesn't need it.

## Complete Android capability map

Query any capability at runtime with `leptoskit::target::Capability` and `PlatformCapabilities::for_platform(TargetPlatform::Android)`, or inspect `Capability::ALL` (each item reports `has_builtin_plugin()`, `plugin_crate()`, `runtime_plugin_name()`, and `cargo_feature()`).

### Built into leptoskit today

| Android feature                                           | Crate / `native.plugins` name                                                                 | What works today                                                                                                                      |
| --------------------------------------------------------- | --------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| App Widgets, Glance Timelines & Quick Settings Tiles      | `leptoskit_plugin_home_widgets` (`home-widgets`, feature `home-widgets`)                      | `SharedPreferences` snapshots & timelines, `APPWIDGET_UPDATE` reloads, `QuickSettingsTile` storage, `readSnapshot` / `readQuickTile`  |
| Android 16+ Live Updates & Ongoing Progress Notifications | `leptoskit_plugin_live_activities` (`live-activities`, feature `live-activities`)             | API 36 `Notification.ProgressStyle` + promoted ongoing chip with automatic standard ongoing progress notification fallback below API 36 |
| Arbitrary Explicit/Implicit Intents & Choosers            | `leptoskit_plugin_android_intent` (`android_intent`, feature `android-intent`)                | Start activities, choosers, broadcasts, services, `can_resolve_activity`, and safe `intent:` URI launches                             |
| Exact, Wake-up, Boot-Persisted & Headless Alarms          | `android_alarm_manager`                                                                       | `AlarmManager` exact/alarm-clock/reboot alarms, durable fired queue, and up to 8-second headless guest wake (without code push)       |
| Picture-in-Picture & Draw-Over-Apps System Overlay        | `picture-in-picture`                                                                          | Activity PiP leases (`supportsPictureInPicture`) plus `SYSTEM_ALERT_WINDOW` status, grant screen launch, and overlay leases           |
| Foldable Posture & Hinge Sensor                           | `posture`                                                                                     | Jetpack WindowManager folding features (`isSeparating`, axis, bounds) and hinge angle sensor for dual-pane and tabletop layouts       |
| Launcher Shortcuts, Incoming Shares & Badges              | `leptoskit_plugin_app_shortcuts` (`app-shortcuts`, feature `app-shortcuts`), `badge`          | Dynamic `ShortcutManager` shortcuts, automatic incoming `ACTION_SEND` share capture, and launcher badge counts                        |
| Local & FCM Push Notifications & Channels                 | `notifications`                                                                               | Notification channels, local posting, and FCM registration token enrollment (paired with [notifications](../guide/notifications/readme.md)) |
| Foreground Services & WorkManager Background Tasks        | `background`, `background-task`                                                               | Persistent foreground services (e.g. location/microphone) and periodic WorkManager tasks (interval ≥ 15 min)                          |
| Passkeys, Biometrics, StrongBox & Keystore                | `passkeys`, `web-auth`, `local-authentication`, `device-keys`, `secure-store`                 | Credential Manager passkeys, Chrome 137+ Auth Tab, API 30+ strong `BiometricPrompt`, AndroidKeyStore/StrongBox keys, AES-GCM secrets  |
| Health Connect, BLE, NFC & Wear OS Companion              | `health`, `ble`, `nfc`, `watch-sync`, `wearable`                                              | Health Connect reads + change tokens, Android BLE GATT central, NFC reader mode NDEF reads, Wear OS Data Layer sync, watch sensors    |
| ARCore, 3D Scenes & Rive                                  | `xr`, `leptoskit` (`3d`, `rive`)                                                              | ARCore / Camera2 tracking + depth, OpenGL ES / Vulkan `Scene3d` overlays, and native Rive vector animations                           |
| CameraX, Media3 Video, Audio, Recorder & Google Maps      | `camera`, `video`, `video-thumbnails`, `audio`, `recorder`, `maps`                            | CameraX preview/still capture, Media3 ExoPlayer, Rust audio engine, AAC background recording, thumbnails, and Google Maps `MapView`   |
| MediaStore, Contacts, Calendar, Print, Share & Pickers    | `media-library`, `image-picker`, `document-picker`, `contacts`, `calendar`, `sharing`, `print`, `sms`, `mail-composer`, `web-browser` | MediaStore, system pickers, `ContactsContract`, `CalendarContract`, `FileProvider` sharing, `PrintManager`, and Custom Tabs |
| Screen Capture Protection, In-App Review, MDM & Usage     | `screen-capture`, `store-review`, `device-policy`, `app-usage`                                | `FLAG_SECURE` window protection, Google Play In-App Review, `DevicePolicyManager` admin actions, and `UsageStatsManager` summaries    |
| Location, Sensors, Light Sensor, Cellular & Device State  | `location`, `sensors`, `light_sensor`, `pedometer`, `battery`, `thermal`, `network`, `network-info`, `cellular`, `haptics`, `speech`, `keep-awake`, `brightness`, `screen-orientation` | `LocationManager`, `SensorManager` (including ambient lux), TTS (`TextToSpeech`), haptics, and connectivity/VPN/satellite hints |
| Over-the-air Wasm updates                                 | `leptoskit_codepush` (feature `codepush`)                                                     | Signed, verified Wasm UI releases and rollback without a new Play Store binary                                                        |

### Android capabilities not yet wired as built-in plugins

These Android APIs do not have a built-in leptoskit runtime plugin yet (`Capability::has_builtin_plugin()` returns `false`, or only typed descriptors exist). To use them today, register your own Kotlin `NativePlugin` inside `android/`:

| Android capability                      | Native Android / Play API                              | Current status in leptoskit                                                                                   |
| --------------------------------------- | ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------- |
| In-App Purchases & Subscriptions        | Google Play Billing Library                            | Not wired; implement in a host Kotlin plugin                                                                  |
| Native Binary In-App Updates            | Google Play In-App Updates (`AppUpdateManager`)        | Not wired (distinct from Wasm code push)                                                                      |
| Google Pay & Google Wallet passes       | Google Pay API / Wallet API                            | Not wired; implement in a host Kotlin plugin                                                                  |
| Lock Screen & Media Transport Controls  | Media3 `MediaSession`                                  | Not wired (`audio` and `video` handle playback without media session controls)                                |
| Camera Barcode & Document Scanner       | ML Kit Barcode / Document Scanner                      | Not wired (`dev-client` only scans dev URLs; `camera` captures still images only)                             |
| Live Speech-to-Text Recognition         | `android.speech.SpeechRecognizer`                      | Not wired (`speech` is text-to-speech via `TextToSpeech`)                                                     |
| Conversation Bubbles & Material You     | `Notification.BubbleMetadata` / DynamicColors          | Typed `BubbleConfig` and `DynamicColorRole` descriptors exist in `app-shortcuts`; host posting/theming custom |
| On-Device Search Indexing               | Jetpack `AppSearch`                                    | Not wired (`app-shortcuts` wires `CoreSpotlight` on Apple only; returns `Unavailable` on Android)             |
| Predictive Back Transition Animations   | `OnBackAnimationCallback`                              | Generated host wires `OnBackInvokedCallback` to the file router without home-preview swipe animations         |
| System VoIP Calling UI                  | `android.telecom.ConnectionService`                    | Not wired; implement in a host Kotlin plugin                                                                  |
| Background Geofencing & Google Cast     | `GeofencingClient` / Google Cast SDK                   | Not wired (`location` handles fixes and foreground-service continuous streams)                                |
| On-Device AI & Android Auto             | Gemini Nano (`AICore`) / Car App Library               | Not wired; requires dedicated Android host modules                                                            |
| Google Play Instant & Cloud Backup      | Play Instant / Android Backup / Block Store            | Not wired; use `leptoskit::rpc` or host Android manifest/backup wiring                                        |
| Hardware App Attestation                | Google Play Integrity API                              | `app-integrity` currently ships an `Unavailable` stub until live Play Integrity wiring lands                  |

## Ship an update without the Play Store

The Play Store build carries the Kotlin activity, the interpreter, and a copy of your screens. To change the screens afterwards, publish a code push release for Android:

```bash
leptoskit native bundle --target android --out dist/android
leptoskit codepush release --from dist/android --platform android --channel production
```

The app downloads the release when the user asks for an update, tries it, and keeps it only if it renders. Anything that needs new native code, such as a new plugin or permission, still needs a Play Store release. [Code push](../guide/05-code-push.md) covers signing, publishing, and rollback.
