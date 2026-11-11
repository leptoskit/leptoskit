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

## Home-screen widgets

The [`home-widgets`](plugin-inventory.md) plugin publishes a bounded text snapshot into app-private preferences and asks `AppWidgetManager` to refresh the app's own App Widget providers. Your app still owns each `AppWidgetProvider` and its layout. Kind-to-provider matching is best-effort by class name; name providers after their kinds when you need a precise reload. `installed` reports whether any matching provider currently has pinned instances, and returns none when no provider class name matches the kind. Snapshot URLs must be HTTP(S) or an app deep-link scheme (`javascript:`, `data:`, `file:`, `blob:`, and `vbscript:` are rejected).

## Android intents

Some things only exist on Android: opening one particular settings page, handing a link to a specific app with the extras it expects, or sending a broadcast your own receiver listens for. The [`android_intent`](plugin-inventory.md) plugin sends any intent you describe, as Flutter's `android_intent_plus` does. Add `leptoskit_plugin_android_intent` to your app and `"android_intent"` to `native.plugins`:

```rust
use leptoskit_plugin_android_intent::guest::AndroidIntent;
use leptoskit_plugin_android_intent::{action, Intent};

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

## Ship an update without the Play Store

The Play Store build carries the Kotlin activity, the interpreter, and a copy of your screens. To change the screens afterwards, publish a code push release for Android:

```bash
leptoskit native bundle --target android --out dist/android
leptoskit codepush release --from dist/android --platform android --channel production
```

The app downloads the release when the user asks for an update, tries it, and keeps it only if it renders. Anything that needs new native code, such as a new plugin or permission, still needs a Play Store release. [Code push](../guide/05-code-push.md) covers signing, publishing, and rollback.
