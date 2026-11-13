# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Features

#### Generated hosts deliver app-shortcut launches

New iOS, Android, and macOS hosts call `noteLaunch` for home-screen quick actions, launcher shortcuts, and dock-menu picks when `app-shortcuts` is selected. Android activities use `singleTop` so a warm shortcut launch reaches `onNewIntent`.

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-13)

### Features

#### Android alarms that run app code

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #627](https://github.com/leptoskit/leptoskit/pull/627)

`leptoskit_plugin_android_alarm_manager` ports Flutter's `android_alarm_manager_plus`. Guests schedule one-shot and repeating Android `AlarmManager` alarms by integer ID, with exact, wakeup, allow-while-idle, alarm-clock and reschedule-on-reboot options, check and request the Android 12+ exact-alarm permission (or fall back to inexact timing), cancel and list alarms, and receive fired alarms through a bounded on-device queue that survives process death: `next()` waits for one, `drain()` takes the ones that fired while the app wasn't running. An alarm marked `headless` starts the app's guest without a window when no guest is running, so `handle_launch` can run app code for it within an eight-second budget. iOS, macOS, Windows, Linux, HarmonyOS and the browser return `Unavailable`.

The plugin declares its alarm and boot receivers with the `android-receivers` runtime metadata, which the host-project docs now describe. The Android host gains `NativeHeadlessGuest`, which runs the bundled guest with no window for a receiver or job and discards what it draws.

- **`leptoskit debug` prints a running app's element tree, runtime errors, and render commits from the devtools hub, and `leptoskit debug open` prints the hub address for the inspector.** `leptoskit mcp` adds `runtime_errors`, `performance`, and `read_logs`, so an agent can see the tree, the errors, the logs, and the commits of an app that `leptoskit dev` is running, and can still capture a screenshot where the platform allows it. `leptoskit device install` installs an Android package, an iOS app, or checks a desktop path. `leptoskit device open` launches it. `leptoskit device sanity` does both and then injects tap, long press, pan, and pinch on the targets that can produce them: every gesture on Android and on an iOS simulator or device, tap and long press and pan on macOS, Windows, and Linux. Linux pinches through `--driver`. A mouse session cannot pinch, and the reply says so. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #740](https://github.com/leptoskit/leptoskit/pull/740)
- **`leptoskit dev` serves phone guests on the local network and prints a QR code for them.** The LeptosKit app scans that code, or takes the address, and runs the unsigned debug guest. Only a local `http` or `https` address is accepted. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #727](https://github.com/leptoskit/leptoskit/pull/727)
- **New database and sign-in apps start with a real notes screen.** A `leptoskit new --db sqlite`, `--db postgres`, or `--auth` project now opens on a notes screen instead of the counter. It uses the shared typed `Notes` and `AddNote` operations the scaffold already publishes: loading, empty, failed, and loaded states, body validation before the mutation, save feedback, and a Refresh button. The browser follows the live socket; native apps use the HTTP plugin and Refresh because their generated hosts don't grant the WebSocket plugin, and the Linux, Windows, and terminal hosts say plainly that they can't reach the server. With `--auth`, the screen is rebuilt per account and the previous account's cached notes are dropped first, so signing out or signing in as someone else never shows the previous person's list. `--db` now also grants the native HTTP plugin and configures its origin. Projects without a database keep the counter. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #609](https://github.com/leptoskit/leptoskit/pull/609)
- **Multiple frontend applications in one workspace.** `leptoskit new --frontends landing,app,admin` scaffolds a `leptoskit.workspace.toml` with separate apps under `apps/` (landing and admin web-only; `app` keeps native hosts). Workspace applications may declare an optional `kind` (`landing`, `product`, `admin`, `server`) for tooling. Docs cover the multi-frontend layout; shared auth across frontends remains out of scope. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #701](https://github.com/leptoskit/leptoskit/pull/701)

#### Versioned native capability contracts

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #630](https://github.com/leptoskit/leptoskit/pull/630)

Native builds now resolve a typed host capability descriptor from the destination flavour, the plugin registrations they actually generated, and the non-secret plugin options. `native bundle` writes `capabilities.json` beside the bundle with a stable SHA-256 digest, and the mobile host embeds the same descriptor in `codepush.json`.

`[native]` gains `required-plugins`, `optional-plugins`, `required-capabilities`, and `optional-capabilities`. A required plugin or capability the selected host cannot run fails the build before packaging, regardless of `missing-plugins`; an optional omission warns with the platform and reason. Existing `plugins` and `missing-plugins` keep their behavior when no requirement lists are set, and a capability declaration never grants access.

Signed releases carry versioned required/optional requirements in `runtime.json`. The installed host checks them against its own descriptor before staging or activating an update; unsupported, unknown, or experimental-without-opt-in requirements are refused. Releases built before this format still pass the legacy ABI, shell, and plugin checks and are reported as legacy, never as verified-compatible. Unknown contract versions and conflicting or duplicate entries are rejected.

Guest requirements bind the bridge protocol as well as the rendering ABI. Explicitly optional plugins no longer enter the legacy required-plugin list. Watch preparation checks requirements before writing its stubs and emits a descriptor; standalone bundles place host evidence beside the output directory.

Updates whose older-style plugin grants have no available implementation report `incomplete` while preserving the existing missing-plugin fallback. Explicit required entries still reject unavailable plugins; explicit optional entries may be omitted. A versioned host descriptor must agree with its own grant list before it can check a release.

Generated iOS and Android update controls show whether host requirements were checked, plugin availability remains incomplete, or only legacy checks ran. A missing report from an older runtime is shown as unreported. None of these states claims that the updated app passed device tests.

The descriptor also names `host.timer` where the generated host enables the reserved `leptoskit.timer` plugin, so `required-capabilities = ["host.timer"]` fails before packaging on hosts that run no guest and is checked by code push on older store builds.

- **`leptoskit dev` rebuilds an edit with a line-table dev profile, compiles the server and the wasm client in parallel when the edit needs both, and copies assets without invoking cargo.** `leptoskit budgets` runs the caps and boot checks and names the build, test, lint, and workspace commands. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #770](https://github.com/leptoskit/leptoskit/pull/770)
- **New apps use the file router by default.** `leptoskit new` enables the `router` plugin, scaffolds `index.rs` / `_layout.rs` / `+not-found.rs` under the app crate's `src/routes/`, commits a generated `routes.rs`, and depends on `leptoskit_router`. Builder `[lint]` stays the default so the starter `Page` and `Layout` components stay clean under `leptoskit lint --deny-warnings`. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #702](https://github.com/leptoskit/leptoskit/pull/702)

#### Share files, subject and title, and where it went

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #625](https://github.com/leptoskit/leptoskit/pull/625)

`leptoskit_plugin_sharing` now covers Flutter `share_plus`'s features. A share can carry up to 32 files, optionally with text: `File::app("exports/report.pdf")` names a file in your app's files directory (the root the `filesystem` plugin writes), and `File::bytes("chart.png", bytes)` shares data from memory, each with an optional MIME type and display name. `with_subject` sets the email subject, `with_title` the sheet or chooser title, `with_anchor` where the iPad popover or macOS picker points, and `excluding` leaves iOS activities out of the sheet.

`share` now returns a `ShareResult`: `Success` with the destination the user picked (the iOS activity type, the macOS service title, or the Android component), `Dismissed`, or `Unavailable` when the platform cannot tell, as in browsers. A result never confirms delivery. On Android this replaces the old behaviour, where dismissing the chooser and choosing an app looked the same.

Native hosts check that an app file stays inside the files directory, refusing symlinks that lead out of it, and copy shared files into a private cache that the next file share clears. On Android the plugin ships its own `FileProvider` and grants read access only to the chosen app, only for those copies; it adds `androidx.core:core`. Browsers share in-memory files through Web Share Level 2 and return `Unavailable`, without a download fallback, when they cannot. Windows and Linux still have no native share sheet.

Breaking: the wire protocol is versioned and changed. The previous request (a kind and one value) and its empty reply are refused, `share` returns `ShareResult` instead of `()`, `web::share` resolves to a `ShareResult` (a browser cancellation is `Dismissed`, no longer a `Cancelled` error, and an iOS or macOS dismissal is `Dismissed` rather than a `Cancelled` error), and `Content::as_str` and `Content::is_url` are replaced by `shared_text`, `shared_url`, `shared_files`, `subject` and `title`.

The CLI reads a new `android-file-providers` plugin metadata key: it copies the plugin's paths XML into the generated resources and declares an unexported `FileProvider` in the generated Android manifest.

- **`leptoskit new` opens on a home screen, a kit of the shared controls, and a list of the platforms the app runs on, in a tab bar.** The home screen still counts with **Add one**. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #727](https://github.com/leptoskit/leptoskit/pull/727)

#### Describe independently configured applications

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #629](https://github.com/leptoskit/leptoskit/pull/629)

Add a separate `leptoskit.workspace.toml` manifest and `lk workspace explain` / `check`. Each application references its own configuration. Startup dependencies are validated independently of call bindings, so bidirectional calls do not become readiness cycles. Unknown references, escaping paths, repeated configurations, conflicting managed identities, and duplicate binding variables fail before a build starts.

The versioned report exposes only selected configuration and environment variable names. It explicitly marks artifacts, API compatibility, database ownership, service identity, and deployment as not checked. Workspace inspection runs no plugin programs or build tools. Image creation, binding injection, change impact, and release orchestration remain future consumers of this graph.

#### Release independently built workspace applications

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #629](https://github.com/leptoskit/leptoskit/pull/629)

Add `workspace plan` and `workspace deploy` for a set of prebuilt, independently pinned image descriptors. Plans freeze the workspace, parsed app configurations, artifact pins, readiness order, and credential variable references. Execution stages exact descriptors, invokes the existing managed release checks in each app directory, and retains a durable progress journal when a later app fails.

Add `deploy --reuse-current` to revalidate a current verified release without booting it again. Reuse checks current authorization, descriptor identity, recorded configuration, database history, image identity, running-container membership, effective app environment, and `/readyz`. Older history without configuration evidence requires a new version. Changed credentials fail reuse rather than being silently ignored. Successful release operation state is now written after backup installation and pruning finish.

Workspace retries revalidate every selected app, including previously completed entries; the journal never authorizes reuse. Automatic builds, source-change impact, private networking, caller identity, and cross-application compatibility remain separate work.

#### Estimated depth for phones that measure none

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #704](https://github.com/leptoskit/leptoskit/pull/704)

The `xr` plugin now gives depth to phones whose platform has none: the Solana Seeker and other phones without ARCore, ARCore phones without the Depth API, and iPhones without LiDAR. Put the `leptoskit_depth` model in the app's `assets/` folder and name it:

```toml
[native.plugin-options.xr]
depth-model = "depth/depth_anything_v2_metric_indoor_small.onnx"
```

The native app loads the model in the background when it starts and checks it against `leptoskit_depth::MODEL`; a missing or different file is logged and leaves depth off. Once it has loaded, sessions on those phones offer `Features::DEPTH`: the model reads about one camera picture a second on its own thread, and the newest reading becomes the frame's depth, so a `Scene3d` over the camera hides things behind real furniture and `DEPTH` hit tests reach the room. Phones that measure depth keep their own.

A reading can be a second old, so hit tests cast against it from where the phone was when the picture was taken (`XrFrame::depth_view`), and occlusion turns it to the current view (`XrFrame::depth_transform`, built on `hit_test::view_to_view`). The new `depth-estimation` feature adds `host::ModelEstimator` and `host::install_depth_model`, for hosts that load the model themselves; `leptoskit build` turns it on only for apps that set `depth-model`.

`DepthEstimator::latest` now returns a `DepthEstimate`, the depth with the view and time of the frame it was read from, and `CameraFrame` carries that frame's `view` in place of `pose`.

Plugin crates can declare `option-features` in their runtime metadata, such as `{ depth-model = "depth-estimation" }`: `leptoskit build` turns the feature on when the app sets the option.

Nothing here has run on a phone yet.

#### 3D scenes over the live camera on iOS and Android

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #681](https://github.com/leptoskit/leptoskit/pull/681)

A `Scene3d` whose backdrop is `Backdrop::Camera` now draws over the camera while an `xr` session runs on a phone. The generated iOS and Android apps hand the scene the session's newest frame: the camera picture behind the models, turned upright for the screen's rotation and cropped to fill the view, the session's own viewpoint in place of the scene's camera, and the room's measured depth hiding the parts of the scene behind real things. Start the session with `Features::CAMERA_IMAGE` (and `Features::DEPTH` for occlusion). Without a session the scene draws as before, transparent behind its models.

Android phones without ARCore, such as the Solana Seeker, now show the back camera in `Orientation` sessions too, through Camera2, with the rotation sensor turning the view and the camera's own field of view keeping the scene to the picture's scale. Such a session leaves the picture out, rather than failing, until the user allows the camera, unless it requires `CAMERA_IMAGE`.

On Android a `Scene3d` showing the camera moves below the window's own views, so the interface laid over it stays visible.

For hosts that draw scenes themselves, `leptoskit_3d::host::Hub::set_camera_feed` takes the picture a camera backdrop draws over, `View::set_rotation` the interface's rotation, and the bridge adds `lk_scene3d_view_set_rotation` and `lk_scene3d_view_shows_camera`. `leptoskit_xr`'s new `scene3d` feature makes the plugin's sessions that feed, and `leptoskit build` turns it on, with any other async plugin's `scene3d` feature, when the app shows 3D scenes. `leptoskit_xr::host::install_depth_estimator` (or `XrRuntime::set_depth_estimator`) takes a `DepthEstimator`, which turns small CPU copies of the camera picture (`CameraFrame`, with the camera's `Intrinsics`) into depth for phones that measure none.

#### Immersive sessions from Meta Quest panel apps

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #691](https://github.com/leptoskit/leptoskit/pull/691)

A Quest app now draws its 3D scene into the headset. Start an `ImmersiveVr` or `ImmersiveAr` session and choose the scene with the new `Session::present(Some(scene_id))`: each eye sees the scene from the player's eye, over passthrough in `ImmersiveAr`. An app built with `[native.quest] immersive = false`, a 2D panel, opens an immersive activity over the panel for the session and comes back to the panel in Horizon Home when the session ends, while its code keeps running and receiving frames.

`leptoskit build --platform quest` declares that activity in every Quest app, marks a panel app's launcher with Horizon OS's 2D category and targets Horizon OS v69, and turns on `leptoskit_xr`'s new `quest` feature (or any async plugin's `quest` feature). For hosts that draw scenes themselves, `leptoskit_3d::host::Hub::draw_scene` draws a scene on another device, such as a headset's, and `Hub::forget_device` moves scenes off it again.

None of this has run on a headset yet.

#### Augmented and virtual reality, and Meta Quest apps

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #659](https://github.com/leptoskit/leptoskit/pull/659)

The new `leptoskit_xr` crate and its `xr` plugin put an app in the real world. Start a session in the best mode the device has — `PhoneAr` through ARKit on iOS and ARCore on Android, `ImmersiveVr` and `ImmersiveAr` on headsets through OpenXR, WebXR in the browser, or `Orientation`, which turns the view with the phone on any device — and read a stream of frames: where the viewer is, each view's projection, the floors, tables, and walls found so far, pinned anchors, the room's light, and controllers and hands. Hit tests find where a tap or a controller's ray meets the real world, against planes and, on phones that measure it, the depth of the room. The camera and depth pictures stay in the native app for the renderer, and the frames your code receives are capped at `[native.plugin-options.xr] max-frame-rate`, always the newest. A recording replays a session exactly, for tests and simulators, with `replay = "…"`.

`leptoskit build --platform quest` builds the Android project as a Meta Quest app, with the manifest Horizon OS expects: OpenXR, head tracking, optional hand tracking and passthrough, the supported headsets, and a full-screen immersive launcher. `[native.quest]` configures it, and `leptoskit dev --platform quest` installs onto a connected headset.

With the `3d` feature, `host::scene3d::draw` draws a `leptoskit_3d` scene into a session: the camera picture behind it, real-world depth hiding what is behind real things, and one view per eye on a headset.

The backends build for their platforms but have not yet run on a device, so treat them as experimental.

### Fixes

- **`leptoskit native build` for Android reports the APK size from the app's own `android/` project.** It was still looking under `hosts/android`, so a build that produced an APK said there was nothing to measure. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #748](https://github.com/leptoskit/leptoskit/pull/748)
- **Keep Android plugins' native libraries on the JNI link line when Cargo prints coloured output, including AAudio for audio plugins.** _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #734](https://github.com/leptoskit/leptoskit/pull/734)
- **An unknown or custom widget is an empty container on Android, in both the backend and the generated app.** It used to stop the host, which closed the app. GTK already drew one as an empty view. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #727](https://github.com/leptoskit/leptoskit/pull/727)
- **`leptoskit device open` fails when the app never takes the screen, instead of reporting success while the launcher is still in front.** A tap, long press, or pan past the display fails too: `input` exits 0 for those points and delivers nothing. An Android pinch still waits for its touchscreen to attach, but that wait is no longer part of the gesture clock, so a 300 ms pinch is 300 ms of contact with both pointers. `device list` marks an AVD as running when that emulator is already up. `leptoskit mcp http` answers `application/json`, and the screenshot tool's schema resolves. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #751](https://github.com/leptoskit/leptoskit/pull/751)

#### Android and Apple hosts ship every view they draw

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #711](https://github.com/leptoskit/leptoskit/pull/711)

Generated Android projects now compile: the platform inventories include `NativeCamera.kt`, `NativeMap.kt`, `NativeLivePhoto.kt`, and `NativeServiceWakeBroker.kt`, so `NativeWidgetRenderer` and `LeptosKitView` resolve the views they draw and the background wake they install. The macOS and iOS inventories likewise include `NativeCamera.swift`, `NativeMap.swift`, and `NativeLivePhoto.swift`, which their widget renderers already used.

A test now fails when a shipped Kotlin or Swift source refers to a type declared only in a file the platform inventory leaves out, so this class of missing source is caught before Gradle or Xcode sees it.

#### Android hosts link the libraries their plugins need

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #716](https://github.com/leptoskit/leptoskit/pull/716)

Generated Android and Quest projects now link. The host's `CMakeLists.txt` names the Rust archive and a fixed list of NDK libraries, and a static archive carries none of Cargo's link directives, so a plugin whose Rust code uses a platform library outside that list failed at `ld.lld` with undefined symbols — `AAudio_createStreamBuilder` and its neighbours once the audio plugin was selected.

The Android host build now asks rustc for the libraries the archive needs (`cargo rustc -- --print native-static-libs`) and writes them into the generated link line, in rustc's order and without repeats, so any plugin's libraries (`aaudio`, `OpenSLES`, `mediandk`, `camera2ndk`, `EGL`, `GLESv3`, `vulkan`, …) arrive with the plugin rather than with a list the CLI has to keep in step.

- **Keep the Android API level in native C and C++ builds when a compiler cache wraps the NDK launcher.** _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #734](https://github.com/leptoskit/leptoskit/pull/734)
- **An Android pinch from `leptoskit device` is two pointers again.** Two `input swipe`s each own one pointer, and on API 35 the activity received only one of them while the step still reported success. The pinch now registers one touchscreen with `uinput` and moves both fingers in the same report. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #745](https://github.com/leptoskit/leptoskit/pull/745)

#### Android Back goes back a route

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #726](https://github.com/leptoskit/leptoskit/pull/726)

The Back gesture and Back button in an Android app now go back one route through the file router, and leave the app only from the first route. Before, Back always left the app: Android 16 delivers Back to an app that targets it only through an `OnBackInvokedCallback`, which the generated activity never registered, and on every Android version the host's question to the router went nowhere.

- `leptoskit_wasm`: `WasmHost::system_back` offers Back to the guest's optional `leptoskit_system_back` export, then flushes and commits what changed. A guest without the export answers `EventOutcome::Unhandled`, and the platform leaves.
- `leptoskit_bridge_ffi`: `lk_guest_system_back` offers Back on the guest thread, and `lk_guest_take_system_back` takes the answers in order. `LeptosKitApplicationView.offerSystemBack` now takes a callback, `(Boolean) -> Unit`, that runs on the main thread with `true` when the router went back.
- `leptoskit_cli`: the generated `MainActivity.kt` registers an `OnBackInvokedCallback` on Android 13 and later and keeps `onBackPressed` for earlier versions; the generated manifest sets `android:enableOnBackInvokedCallback="true"`.

Apps scaffolded before this keep leaving on Back until they copy the new `MainActivity.kt` Back code and the manifest attribute from a new project; [the Android page](https://leptoskit.dev/docs/platforms/android.html#the-back-gesture) lists them.

#### Boot iOS for sanity checks and use a mouse on macOS

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #747](https://github.com/leptoskit/leptoskit/pull/747)

`leptoskit device sanity` on an iOS simulator now boots that simulator before `simctl install`. A shut-down simulator refuses the install. One that is already booted stays up: the boot step treats that state as success. A sanity run that does not install, and a run on a real iPhone, still do not boot.

Pointer actions sent to a macOS session use a mouse. Mac2 does not accept a touch pointer. iOS, Android, Linux, Windows, and the web still send touch.

- **Reading a watched file does not restart the app.** `leptoskit dev` ignores filesystem access events before scheduling a rebuild or marking the configuration, database tables, or Git branch as changed. On Linux, the compiler and CLI reading their inputs could otherwise trigger another build repeatedly, even without an edit. Source edits and file replacements still rebuild the app. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #774](https://github.com/leptoskit/leptoskit/pull/774)
- **Validate the dev address before watching the workspace.** `leptoskit dev` rejects an invalid web serve address before inspecting Cargo metadata, starting a database, or watching files. Watching still begins before the initial build, so edits made during that build trigger a rebuild. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #734](https://github.com/leptoskit/leptoskit/pull/734)
- **`lk dev` keeps edits made while it starts.** `leptoskit dev` now starts watching your project before its first build rather than after every platform has started. An edit saved while that first build ran, which takes minutes on a fresh checkout, used to go unseen: the server came up on the code from before the edit and stayed on it until you saved again. The edit now waits in the watcher's queue, and `dev` rebuilds and restarts the server as soon as the platforms are up. The `watching … for changes` line still prints once everything has started. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #761](https://github.com/leptoskit/leptoskit/pull/761)
- **Connected iPhones show up in `device list`.** `leptoskit device list` asked `devicectl` for `--json` and parsed a `devices` array at the top of that output. devicectl does not print JSON that way. It writes a file for `--json-output`, and the phones are under `result.devices`. The id an install, a launch, and an XCUITest session need is the hardware UDID, not the Core Device identifier. A plugged-in iPhone never appeared. Watches and simulators in the same file stay out of the iPhone list. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #746](https://github.com/leptoskit/leptoskit/pull/746)
- **Image builds retry their tool download.** The Dockerfile that `leptoskit deploy` generates for web apps downloads binaryen's `wasm-opt` from GitHub. It now retries that download up to five times and unpacks it only once it is complete, so a brief GitHub outage no longer fails the image build. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #668](https://github.com/leptoskit/leptoskit/pull/668)

#### File-route navigators, deep links, and system Back

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #679](https://github.com/leptoskit/leptoskit/pull/679)

Layouts can export `navigator()` returning a typed `Stack`, `Tabs`, or `Drawer`. Folder groups like `(tabs)/` stay URL-less; the navigator return value chooses the chrome. Tab and drawer roots are `impl Into<Route>`, not string names. Stacks can present matched routes as a sheet or modal above the previous screen, and optionally offer an edge hit target for Back.

`Navigation::open_url` / `open_url_with` turn incoming URLs into typed routes (custom schemes and `http(s)`). `apply_activations` drains a host deep-link queue into the router. `bind_system_back` and `leptoskit_system_back` let Android system Back pop router history when the host offers it. Generated Android activities call into that path before finishing.

[`examples/file_routes`](https://github.com/leptoskit/leptoskit/tree/main/examples/file_routes) shows Stack + Tabs + Drawer, deep links, and Back. True UIKit interactive-pop and AppKit history swipe still need host presentation; the guide names those gaps.

- **Builder lint keeps the SSR shell; apps pin Rust.** `markup = "builder"` no longer reports ordinary HTML tags in the SSR document `shell` (`<html>`, `<head>`, …)—only leptoskit component trees. The agent skill and linting guide say the same. Deploy generation names `--sdk-path` when a path dependency sits outside the workspace. (New apps already pin `rust-toolchain.toml` from the matching scaffold work.) _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #650](https://github.com/leptoskit/leptoskit/pull/650)

#### Generated sign-in effects run in native apps

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #609](https://github.com/leptoskit/leptoskit/pull/609)

`leptoskit new --auth` and `leptoskit generate auth` now use isomorphic effects for the sign-in screen's mount step, the provider-notice step, and the post-sign-in navigation. `Effect::new` is compiled out in a native guest (the generated app does not enable leptos's `effects` feature), so the screen could stay on "Checking whether you're signed in…" forever. The effects still skip server rendering, so the first server HTML matches the browser's first render.

The generated notes journey is now exercised end to end in the real interpreter: the optimized guest loads and refreshes notes over the HTTP plugin, validates the draft, shows pending/saved/failed saves, signs in through the development mailbox, switches accounts without showing the previous account's rows, and rejects late list and save replies after their screen is disposed.

The generated `add_note` operation trims the body before it checks the note's length, so a direct API caller cannot store a note that is only whitespace even though the screen already trims its draft.

The sign-in countdown is tested apart from its executor: a host timer counts the whole wait down, a missing or rejected timer leaves the server's message and a usable manual retry without claiming time passed, a newer refusal or starting over stops an older countdown, and disposing the screen stops it without touching a disposed signal.

`leptoskit generate auth` on an existing database app still adds the screens and asks the app to call `auth::provide()` and put `auth::screens().gate(…)` in front of its own home screen. Rewriting an app's root automatically is not safe, so that manual step stays open; the journey covers the fully scaffolded `--auth` app, which is already wired.

- **Give the HarmonyOS host's line-height values named, explicit types so generated applications compile with the public API 12 ArkTS compiler.** _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #734](https://github.com/leptoskit/leptoskit/pull/734)
- **`lk lint` passes on a fresh clone.** `leptoskit lint` no longer fails on a fresh clone of an app with native platforms. App-owned runners include a registrant under `.leptoskit/<platform>/`, which Git ignores and only native builds wrote, so clippy stopped on the missing file until you ran a native build. Before clippy starts, `lk lint` now writes missing registrants for the enabled hosts that compile on the current machine and enables the Cargo features their plugins need. Existing registrants stay untouched until the next native build. If a host cannot be prepared, lint reports a warning and clippy names what it cannot compile. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #753](https://github.com/leptoskit/leptoskit/pull/753)

#### Linux pointer gestures from `leptoskit device` no longer pause for about fifteen seconds when the pointer is already on the point.

`xdotool mousemove --sync` waits for a motion event that never arrives in that case, so a tap followed by a long press or a pan at the same point stalled. The move is sent without that wait.

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #744](https://github.com/leptoskit/leptoskit/pull/744)

Opening a Linux app also waits until one of its windows is visible, the process exits, or two seconds pass. Gestures used to be injected the moment the process was spawned, which is before the window is mapped, so the tap and the long press landed on the desktop.

Opening an Android app waits until that package is the focused window, then a little longer, or two seconds pass. `am start` was returning while the launcher was still focused, and the focus line itself updates before the activity accepts a tap, so the sanity tap and long press never reached the activity.

`leptoskit device boot` of an AVD that is already running fails immediately and names the emulator serial, instead of starting a second emulator and waiting out the boot deadline. If the emulator process exits before the device is online, the boot fails then, instead of blocking on `adb wait-for-device`.

- **More ergonomic multi-frontend scaffold.** `--frontends` alone scaffolds landing, app, and admin. Ports are stable per kind. The product app under `apps/app` uses the workspace name as its Cargo package. Frontend names trim whitespace and accept ASCII case; `ApplicationKind` gains `FromStr`, `Display`, and helpers used by the scaffold. Each frontend app uses the same file-router defaults as a normal `leptoskit new`. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #703](https://github.com/leptoskit/leptoskit/pull/703)
- **Native capability parity: badges and shortcut hosts.** Browser pages can set and clear app-icon badges through the Badging API (`web::set` / `web::clear`), and `web::get` returns the last count this adapter published. Generated iOS, Android, and macOS hosts now call `noteLaunch` when the OS activates an app shortcut: Android dynamic shortcuts carry a unique intent per id, the macOS host builds the dock menu from `DockShortcutStore`, and iOS notes home-screen quick actions. Docs match the real backends for menus, badges, and shortcuts. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #628](https://github.com/leptoskit/leptoskit/pull/628)

#### Plugin options the app would crash on stop the build instead

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #729](https://github.com/leptoskit/leptoskit/pull/729)

An Android app whose `leptoskit.toml` had an empty `[native.plugin-options.websocket]` built, installed, and then stopped at launch with `NativeApplicationInitializationFailure: register plugin: InvalidArgument`. The WebSocket plugin can't start without `origins`, the build only checked that the table was there, and the error didn't say which plugin had failed.

`leptoskit build` now stops with the plugin and the option it needs:

```
plugin `websocket` needs `origins` in [native.plugin-options.websocket]: without it, the app stops at launch when the plugin starts
```

An async plugin lists the options its factory can't start without in its runtime metadata, and an empty string, list, or table counts as left out:

```toml
[package.metadata.leptoskit.runtime]
name = "websocket"
rust-factory = "from_config"
rust-async = true
required-options = ["origins"]
```

`http` and `websocket` require `origins`, `shell` requires `commands`, and `xr` requires nothing. Each crate's tests hold its factory to its list.

When a plugin still fails to start, the error names it: ``register plugin `websocket`: InvalidArgument``. Generated registrants install every plugin through the new `App::register`, and a host directory that can't be used is reported as `app context: …` rather than as a plugin's failure.

The `shell` plugin is now left out of iOS, Android, and HarmonyOS builds, where it can't start processes and its factory always failed, which also stopped the app at launch.

- **Harden native-wave plugin correctness.** Live Activities on iOS cancel push-token waits without freezing the per-id busy lock, replace duplicate ActivityKit ids on start, and reject duplicate content keys. Picture-in-picture leases best-effort exit system PiP and observe user exit; `request_overlay` docs match launch-time status. Home widgets stop reporting unrelated pins for unknown kinds, commit preferences before success, and reject dangerous URL schemes. Device-policy and Live Activities stubs validate payloads before Unavailable. Wear/watchOS hosts return errors instead of panicking on viewport kind mismatch. New Android hosts declare `supportsPictureInPicture` on MainActivity. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #618](https://github.com/leptoskit/leptoskit/pull/618)

#### Add the recorder plugin: continuous microphone segments

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #737](https://github.com/leptoskit/leptoskit/pull/737)

Add a `leptoskit_plugin_recorder` crate with a typed binary guest client, Swift and Kotlin handlers, and automatic native registration under the runtime name `recorder`. It records the microphone continuously in the background as timed audio segments: AAC-LC, mono, `.m4a`, rotated at each boundary with no gap between segments, for apps that upload and transcribe each segment themselves.

`start` takes a directory inside the app's own data or cache directories and a segment length (default 300 s, clamped 5–3600 s), sample rate (default 16 kHz), AAC bitrate (default 24 kbps) and activity threshold (default −45 dBFS). `next_segment` is a cancellable wait that returns each finished segment with its path, start time, duration, size, peak level and `active_ms` — the milliseconds of 30 ms frames above the threshold — so the app can skip uploading quiet segments without decoding them. The plugin never deletes a delivered segment and never decides whether one is silent; the app owns both choices. `status` reports idle, recording, interrupted (a call or Siri), silenced (Android: another app holds the microphone) or failed with a reason, plus the open segment's start time.

A small append-only journal beside the segments survives a crash: the next `start` (or a `recover` call) delivers leftover finalised files as `ended_by = recovered` segments and deletes the truncated files that cannot be read, reporting each deletion once. iOS records through an `AVAudioEngine` input tap into an AAC `AVAudioFile`, configures `.playAndRecord` with `.mixWithOthers`, `.allowBluetooth` and `.defaultToSpeaker` so the owner's music and calls keep working, closes the segment on an interruption and resumes when the system allows it, and rebuilds the tap on route changes and after a media services reset; recording can only start in the foreground, and the `audio` background mode keeps it running. Android records `AudioRecord` PCM into a `MediaCodec` AAC encoder and `MediaMuxer` on its own thread, reports `silenced` through the audio recording callback, and requires the `background` plugin's microphone-typed foreground service for background recording — which Android 14 cannot start from `BOOT_COMPLETED`, so the owner must open the app once after a reboot. The `background` plugin now exposes the running service's type mask so `start` can fail with a clear error instead of recording into a process the system will kill.

#### Preserve null, empty collections and omitted arguments in generated SDK requests.

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #669](https://github.com/leptoskit/leptoskit/pull/669)

Query and form encoders now match the server's serde_qs 1.1 protocol: explicit null is a bare key and an empty collection is `key=`. JSON and live encoders keep explicit null, and required nullable arguments in Go and Rust keep their keys. Python optional arguments use the exported `UNSET` for omission and `None` for explicit null. Generated Rust and CLI packages need `serde_qs` 1.1.3; update an existing package's manifest when regenerating its code.

Clients, OpenAPI, AsyncAPI and compatibility checks now follow each argument's `required` flag rather than inferring it from nullability. Specs from `#[api]` functions mark `Option` arguments optional, because the server reads a missing key as `None`. The spec format is now version 4; older specs still load, and their nullable arguments stay optional. A spec written by this version needs an equally new CLI to generate clients.

In regenerated Dart clients, `Option` arguments become optional `Presence<T?>` parameters instead of required `T?` ones. Omit them, or pass `Presence.present(value)`.

Strict diagnostics report a Go or Rust optional nullable argument only when its schema declares a default, and no longer report a Dart query/form limitation that the encoder now handles. Mandatory SDK acceptance decodes generated HTTP requests with the server codec.

- **New apps write builder lint to match the starter screens.** `leptoskit new` now sets `[lint] markup = "builder"` with `markup_style` and `component_attribute` denied, so a fresh scaffold's builder home screen is clean under `leptoskit lint --deny-warnings`. The skill, creating guide, linting guide, and small-bundles guide match: omitted `render` still defaults to csr in the schema, but new projects keep the explicit ssr they already write. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #653](https://github.com/leptoskit/leptoskit/pull/653)
- **Scaffold uses shared Health; skill documents web-only trim.** New apps re-export `Health` from the shared crate in the app library so `leptoskit lint --deny-warnings` no longer fails on an unused `*_shared` dependency before any RPC types are used. The coding-agent skill says there is no frontend-only `new` flag: after scaffolding, set `platforms = ["web"]` and optionally trim unused host workspace members. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #649](https://github.com/leptoskit/leptoskit/pull/649)
- **New apps pin rustc; skill matches the scaffold.** `leptoskit new` now writes a `rust-toolchain.toml` that pins the SDK channel and the `wasm32-unknown-unknown` target, so `edition = "2024"` projects build even when rustup's default toolchain is older. The coding-agent skill no longer claims the default starter uses regenerating file routes, tells agents to keep the HTML `shell` in leptos `view!` (with a line-local lint allow) when enforcing builders, and shows `#[component]` helpers returning `Element` for `.child(…)` composition. The creating guide notes the SSR default and the toolchain pin. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #648](https://github.com/leptoskit/leptoskit/pull/648)
- **Web-only lint skips unused hosts; drop an import.** `leptoskit lint`'s clippy pass excludes workspace host crates whose platforms are not listed in `app.platforms`, so setting `platforms = ["web"]` is enough for a green deny-warnings run without GTK or native prepare. `--db` / `--auth` scaffolds no longer put an unused `use leptos::prelude::*` in `App()`. The skill documents the platforms-based clippy skip. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #657](https://github.com/leptoskit/leptoskit/pull/657)

#### Native scenes load models on the first launch

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #734](https://github.com/leptoskit/leptoskit/pull/734)

Android, iOS and macOS install the scene asset reader before starting the guest. Models queued before the first native scene view exists now load from the app bundle on a cold launch.

Generated Axum route lists also return their owned paths directly, keeping generated apps free of redundant conversion warnings.

#### Service workers and stronger extension messaging

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #584](https://github.com/leptoskit/leptoskit/pull/584)

Web apps can enable the `service-worker` feature to register a worker script, exchange messages, describe caching strategies, open Cache Storage, and manage Web Push subscriptions. Calls return `Unavailable` outside a secure browser context.

Browser extensions gain long-lived `runtime::connect` ports (with `Background::on_connect`), storage `clear` / `keys` / `get_bytes_in_use`, and permissions `get_all` / `on_added` / `on_removed`. The generated background worker keeps ports alive so the wasm side can post and disconnect by id.

- **SSR apps now register every file route with `leptos_axum`, and the server render reads the request URL instead of always opening on Home.** _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #743](https://github.com/leptoskit/leptoskit/pull/743)

#### Server-rendered queries hydrate without a mismatch

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #756](https://github.com/leptoskit/leptoskit/pull/756) · _Closed issues:_ [#722](https://github.com/leptoskit/leptoskit/issues/722)

A page that rendered a query on the server now hydrates as the same page in the browser. Before, only a query's data was carried, so a query that failed on the server started from "loading" in the browser, leptos reported a hydration mismatch, and the page stopped updating.

- `QueryScope::with_ssr_errors` (and `with_error_codec`) streams a server render's error alongside the data, so the browser shows the same error first. RPC queries and paginated queries opt in, and `FnError` is now `Serialize` and `Deserialize`.
- A query seeded from the server doesn't fetch while the page hydrates. Its mount refetch — stale data in the background, or a retry after an error — starts once hydration has finished, so the first render matches the server's HTML.
- On the server, reading a query that is still loading inside `<Suspense>` holds the boundary until it settles, as a leptos resource does, so the server renders the outcome it streams.
- Streamed payloads escape `<`, `>`, `&`, and U+2028/U+2029, so data and errors can't close the `<script>` they're embedded in.

Apps using `render = "ssr"` should read queries inside `<Suspense>`; outside a boundary the server renders whatever the query holds at that moment.

New apps put their web adapter inside that boundary, including the database and sign-in scaffolds. Existing portable apps should wrap the closure that calls `leptoskit::web::render` in the same way.

Generated browser entrypoints configure the RPC and auth clients inside the mount callback, after leptos has set up its executor and owner, so the live socket starts on the first page load. A query observer reads `enabled` without subscribing while it binds or runs its refetch interval, so an `enabled` closure that reads another query no longer logs a reactive-tracking warning on every page.

#### Accept managed registries with explicit ports

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #629](https://github.com/leptoskit/leptoskit/pull/629)

Managed image configuration accepts a registry hostname with a numeric port and an image repository under that registry. Invalid ports, embedded credentials, paths in registry authorities, and tagged image repositories remain rejected.

Verified image packaging accepts `--service <infra.name>` to include Kamal’s required service label in the independently pinned image configuration. Reusing an image checks the label along with the rest of that configuration.

Deployment accepts Docker’s null defaults for absent Podman `Cmd`, `OnBuild`, and `Volumes` inspection fields while still requiring the exact content-addressed image ID. Non-null changes remain rejected.

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_cli: the command line

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_cli` is the `leptoskit` and `lk` command: scaffold a project with `new`, run every platform at once with `dev`, then `build`, `serve`, and `test`. It also writes routes, translations, and database migrations, generates sign-in screens and plugins, ships code push releases, uploads to the app and extension stores with `publish`, drives devices and simulators (also over MCP), and lints and formats the code.

Deployment lives here too: `infra` manages OpenTofu infrastructure, `deploy` releases a commit or rolls back, `console` hosts or connects to the self-hosted deployment service, or pairs a worker with the hosted LeptosKit app, and `verify` reproduces backend builds and checks signed deployment policies. New projects resolve their development secrets through Monosecret, and `leptoskit dev --no-secrets` uses the existing environment. The binary is a thin wrapper over the library, so tooling can drive the same commands directly.

### Features

#### Try native plugins written only in Rust

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #506](https://github.com/leptoskit/leptoskit/pull/506)

Set `native.experimental-rust-plugins = true` in `leptoskit.toml` to use the experimental pure-Rust `clipboard` and `haptics` plugins on iOS, macOS, and Android in place of their Swift and Kotlin versions. Without the setting, builds keep the Swift and Kotlin versions.

A plugin of your own can do the same in its `[package.metadata.leptoskit.runtime]` tables. With `rust-context = true`, its factory receives a `leptoskit_bridge::AppContext`, whose `platform()` lends the host's native handles: the `JavaVM` and app `Context` on Android, and the on-screen view controller on iOS and macOS. With `rust-main-thread = true`, its calls run on the platform's UI thread, which UIKit, AppKit, and most Android UI APIs require (`App::install_main` installs such a plugin by hand). Mark a platform's registration `experimental = true` to use it only when the setting is on.

### Fixes

- **Reserve private operation logs before launching the deployment command so a repeated operation cannot execute again when its logs already exist.** _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #516](https://github.com/leptoskit/leptoskit/pull/516)
- **Resolve deployment Git operations against the registered repository even when the CLI runs from a Git hook.** _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #516](https://github.com/leptoskit/leptoskit/pull/516)
- **Explain how to connect to the deployment service when its saved credential is missing on Unix.** _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #516](https://github.com/leptoskit/leptoskit/pull/516)
- **Paint italic text and per-edge borders on HarmonyOS.** The ArkUI host now applies `italic` and per-edge `border_widths` (`border-t`, `border-l-2`, …) the same way Android and GTK do, instead of leaving those strokes undrawn and italic upright. Deno render tests lock the Yoga frames from `grid_parity_probe` so absolute ArkUI placement stays aligned with the web HTML probe. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #539](https://github.com/leptoskit/leptoskit/pull/539)
- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
- **Run recovery SQL without reading psql startup files, so local formatting settings cannot break backup verification.** _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #516](https://github.com/leptoskit/leptoskit/pull/516)
