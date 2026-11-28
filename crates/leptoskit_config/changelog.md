# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-28)

### Features

- **`[native.android] push` opts the Android app into FCM push enrollment.** It is a boolean, off by default: an app without it generates byte-for-byte as before. With it, Android generation adds the pinned `com.google.firebase:firebase-messaging` dependency, writes `.leptoskit/android/gradle-plugins.txt` with Google's services plugin, generates the messaging service that forwards FCM token refreshes to the notifications plugin, and declares that service in the manifest. Push on also requires the `notifications` plugin in `[native] plugins`, the app author's `google-services.json` at `hosts/android/app/google-services.json` — never generated or committed by leptoskit — and an app Gradle script that reads the generated plugins file (the current `leptoskit new` scaffold does; older apps get the exact lines from the generation error). The configuration schema carries the new key. _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)

#### Add cloud launch/deploy, storage config, and MCP tools

_Owner:_ Ifiok Jr. · _Introduced in:_ [94859ef](https://github.com/leptoskit/leptoskit/commit/94859efebd25f1bfb3f1c055c3172de4c309cad9)

- Adds `[cloud.storage.buckets.<name>]` configuration (`public`, `max_size_gb`) and updates the published `leptoskit.toml` JSON schemas.
- Adds `lk cloud launch`, `lk cloud deploy`, `lk cloud status`, `lk cloud logs [--follow]`, `lk cloud estimate`, and `lk cloud open`, wires `lk deploy` to dispatch to LeptosKit Cloud whenever `[cloud].project` is set, and adds Cargo-style external subcommand dispatch (`lk cloud <name>` → `leptoskit-cloud-<name>` on `PATH`, enabling `lk cloud platform`, `lk cloud worker`, and `lk cloud control`).
- Adds `--web-only` and `--cloud` flags to `lk new` and `lk generate app`, allows `--frontends` to combine with `--db`, `--auth`, `--web-only`, and `--cloud`, and falls back with a clear warning instead of hard-failing `lk dev` when `monosecret` is not installed.
- Exposes six cloud and database tools on `lk mcp` (`cloud_estimate`, `cloud_deploy`, `cloud_status`, `cloud_logs`, `cloud_secrets_list`, `db_check`) without ever returning secret values.

- **The groundwork for LeptosKit Cloud.** `leptoskit.toml` accepts a `[cloud]` table that says where LeptosKit Cloud should run your app and how big: a project, an optional region, and per-environment size, replicas, sleeping when idle and custom domains, plus the database's size and backups. Config checks it as it loads, with errors that name the table and key, such as a SQLite app asking for more than one replica, or sleeping while it serves live queries. The new `leptoskit_cloud` crate holds the shared types behind it: slugs, sizes, deployment states, the source archive a deploy will upload (deterministic, with secret files refused), and a monthly cost estimate for a `[cloud]` table. The cloud itself doesn't run yet, so nothing deploys there today. _Owner:_ Ifiok Jr. · _Introduced in:_ [403aaf0](https://github.com/leptoskit/leptoskit/commit/403aaf07e6df872150ffa904a48c1a9d55e1b6ed)
- **`[native.ios] aps-environment` names the APNs environment an iOS app opts into: `development` or `production`, exactly Apple's entitlement strings.** Unset by default. _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)
- **`[native.macos] aps-environment` names the APNs environment a macOS app opts into: `development` or `production`, exactly Apple's entitlement strings.** Unset by default. _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)

#### Add desktop window overlays and background modes

_Owner:_ Ifiok Jr. · _Introduced in:_ [9714421](https://github.com/leptoskit/leptoskit/commit/9714421722143255a82869b80b11b2b7be40f7ed)

- `leptoskit_plugin_window_state::desktop` adds Electron-class desktop windowing and lifecycle types: `WindowOptions` (with `floating_overlay` preset), `WindowLevel` (`Normal`, `Floating`, `AlwaysOnTop`, `Status`, `PopUpMenu`, `ScreenSaver`), `WindowChrome` (`Standard`, `Frameless`, `HiddenTitleBar`), `VibrancyMaterial` (macOS vibrancy and Windows Mica/Acrylic), `CloseBehavior` (`Quit`, `HideToTray`, `KeepRunningInBackground`, `Minimize`), `ActivationPolicy` (`Regular`, `Accessory`, `BackgroundOnly`), `TaskbarProgress`, `DockAttention`, `WindowController`, and `MemoryWindowController`.
- `leptoskit_config` adds `NativeMode::Background`, allows `menu-bar` and `background` startup modes across desktop platforms (`macos`, `windows`, `linux`), and adds the `[native.window]` configuration table (`DesktopWindowConfig`).

#### Add extension browser detection and full side panel API

_Owner:_ Ifiok Jr. · _Introduced in:_ [c4aa7df](https://github.com/leptoskit/leptoskit/commit/c4aa7df45dd07fbead27b9b05bba1d67ecfacd7f)

- `leptoskit_extension::browser` adds runtime browser detection (`ExtensionBrowser` covering Chrome, Edge, Brave, Opera, Arc, Vivaldi, Firefox, and Safari; `BrowserFamily`; `BrowserInfo`; `BrowserSwitch`; and `with_browser` test scoping).
- `leptoskit_extension::side_panel` adds full Chromium `chrome.sidePanel` and Firefox `browser.sidebarAction` support: `open_target`, `open_for_tab`, `close`, `toggle`, `is_open`, `set_options` / `get_options`, `set_behavior` / `get_behavior`, `get_layout`, `set_title` / `get_title`, and `capabilities`.
- Adds `leptoskit_extension::windows`, `leptoskit_extension::offscreen`, `leptoskit_extension::omnibox`, and `leptoskit_extension::native_messaging`.
- Adds `[extension].optional_permissions` and `[extension.side_panel]` `title` / `open_at_install` options to `leptoskit_config` and generated MV3 manifests.

- **JSON RPC wire and ambient context.** Queries and mutations speak JSON through serde on `POST /rpc/<name>`. `[rpc]` sets `path`, `live_path`, `transports`, and `default_transport`; the only transport is `json`. A query or mutation can omit its context argument and call `rpc::cx()` or `rpc::mutation_cx()`. _Owner:_ Ifiok Jr. · _Introduced in:_ [8bd1891](https://github.com/leptoskit/leptoskit/commit/8bd1891b8d987c9abe19128343e7107d706ecd88)

#### Meta Horizon Store publishing for Meta Quest

_Owner:_ Ifiok Jr. · _Introduced in:_ [f9e7c49](https://github.com/leptoskit/leptoskit/commit/f9e7c495e9fb4ef73cd8458d03b103a37083ff31)

Add `[publish.quest]` configuration and `leptoskit publish quest` to build a release-signed Meta Quest APK and upload it to Meta Horizon Store release channels (`alpha`, `beta`, `rc`, `store`, or subchannels) through Meta's `ovr-platform-util upload-quest-build`.

- Configures `app_id`, `channel`, `age_group` (`teens_and_adults`, `mixed_ages`, `children`), `draft`, and optional `obb`, `assets_dir`, and `debug_symbols_dir` in `[publish.quest]` with updated JSON schemas.
- Accepts `LEPTOSKIT_PUBLISH_QUEST_APP_SECRET` (preferred) or `LEPTOSKIT_PUBLISH_QUEST_TOKEN`, supports `LEPTOSKIT_OVR_PLATFORM_UTIL` overrides, redacts secrets from tool errors, and rejects x86-only ABIs before compiling.
- Defaults to `--draft` uploads until `--submit` is passed with an `age_group`, prints the remaining Meta Horizon Developer Dashboard steps (listing copy/art, Data Use Checkup, content rating, and VRC/Content review submission), and wires `quest` into the generated GitHub Actions `release.yml` workflow.

- **Multiple frontend applications in one workspace.** `leptoskit new --frontends landing,app,admin` scaffolds a `leptoskit.workspace.toml` with separate apps under `apps/` (landing and admin web-only; `app` keeps native hosts). Workspace applications may declare an optional `kind` (`landing`, `product`, `admin`, `server`) for tooling. Docs cover the multi-frontend layout; shared auth across frontends remains out of scope. _Owner:_ Ifiok Jr. · _Introduced in:_ [6fed659](https://github.com/leptoskit/leptoskit/commit/6fed65923cda6adbd7f8ac645ff2555ff34fb819)

#### Versioned native capability contracts

_Owner:_ Ifiok Jr. · _Introduced in:_ [442b38b](https://github.com/leptoskit/leptoskit/commit/442b38b81352231ebb9810f646247bd548ed353c)

Native builds now resolve a typed host capability descriptor from the destination flavour, the plugin registrations they actually generated, and the non-secret plugin options. `native bundle` writes `capabilities.json` beside the bundle with a stable SHA-256 digest, and the mobile host embeds the same descriptor in `codepush.json`.

`[native]` gains `required-plugins`, `optional-plugins`, `required-capabilities`, and `optional-capabilities`. A required plugin or capability the selected host cannot run fails the build before packaging, regardless of `missing-plugins`; an optional omission warns with the platform and reason. Existing `plugins` and `missing-plugins` keep their behavior when no requirement lists are set, and a capability declaration never grants access.

Signed releases carry versioned required/optional requirements in `runtime.json`. The installed host checks them against its own descriptor before staging or activating an update; unsupported, unknown, or experimental-without-opt-in requirements are refused. Releases built before this format still pass the legacy ABI, shell, and plugin checks and are reported as legacy, never as verified-compatible. Unknown contract versions and conflicting or duplicate entries are rejected.

Guest requirements bind the bridge protocol as well as the rendering ABI. Explicitly optional plugins no longer enter the legacy required-plugin list. Watch preparation checks requirements before writing its stubs and emits a descriptor; standalone bundles place host evidence beside the output directory.

Updates whose older-style plugin grants have no available implementation report `incomplete` while preserving the existing missing-plugin fallback. Explicit required entries still reject unavailable plugins; explicit optional entries may be omitted. A versioned host descriptor must agree with its own grant list before it can check a release.

Generated iOS and Android update controls show whether host requirements were checked, plugin availability remains incomplete, or only legacy checks ran. A missing report from an older runtime is shown as unreported. None of these states claims that the updated app passed device tests.

The descriptor also names `host.timer` where the generated host enables the reserved `leptoskit.timer` plugin, so `required-capabilities = ["host.timer"]` fails before packaging on hosts that run no guest and is checked by code push on older store builds.

#### An app's own fonts on every platform

_Owner:_ Ifiok Jr. · _Introduced in:_ [b4744bd](https://github.com/leptoskit/leptoskit/commit/b4744bd432011c953a8a1a932f3306b270eed0ef)

Declare the typefaces an app bundles once, in `leptoskit.toml`, and choose one with the class the web already uses:

```toml
[theme.fonts.hand]
family = "Skribble Noto Sketch"
faces = [
	{ file = "fonts/NotoSketch-Regular.ttf" },
	{ file = "fonts/NotoSketch-Bold.ttf", weight = 700 },
	{ file = "fonts/NotoSketch-Italic.ttf", style = "italic" },
]

[theme.fonts.display]
faces = [{ file = "fonts/Recursive.ttf", weight = [300, 1000] }]
```

`font-[var(--hand)]` now draws the family on the web, iOS, Android, and macOS, and every text inside the element inherits it. Faces are matched by weight and style as `@font-face` rules are, a variable face draws the weight asked for on its `wght` axis, and layout measures each label in the face that draws it.

- The build checks every declared file and stops with an error naming the `[theme.fonts]` entry and the path it looked at when a file is missing or isn't a TrueType or OpenType font.
- Native builds write `fonts.json` beside the app bundle; the generated iOS, Android, and macOS hosts register the files before their first layout. The web build writes and links `leptoskit-fonts.css`.
- `leptoskit_native::fonts` holds the manifest, the face selection, and `install` for hosts written in Rust; `NativeProps::font_family` and `TextStyle::{italic, face}` carry the choice to measurement and drawing. `font-sans`, `font-serif`, and `font-mono` now return to the system font on native platforms.
- The AppKit and UIKit backends draw and measure declared faces once installed. The host text measurer's C ABI gains the face to measure in, and `lk_fonts_install` and `lk_fonts_select` let Swift and Kotlin hosts install the manifest and choose faces exactly as layout does.
- Windows, Linux, HarmonyOS, watchOS, the terminal, and the Rust `leptoskit_android` backend still draw the system font.

#### Push-enabled hosts now work the way the notifications guide describes.

_Owner:_ Ifiok Jr. · _Introduced in:_ [7f60843](https://github.com/leptoskit/leptoskit/commit/7f60843eba43202f72117226374f71d7b4f6528d)

**Android.** The scaffold's Gradle script applies Google's services plugin by id (`apply(plugin = plugin)`); the earlier `apply(plugin)` asked Gradle for a script file of that name and failed every push-enabled build. An app scaffolded with the earlier line gets a generation error that names the one-word change. The generated messaging service is declared with the `com.google.firebase.MESSAGING_EVENT` intent filter, so Firebase delivers token refreshes to it instead of to its own default service. The manifest names a default notification channel, `leptoskit.notifications.default`, which the notifications plugin creates at startup, and the notification icon when `[app.icons] notification` generated one. Release builds get R8 keep rules for the Firebase names the plugin looks up by reflection, the plugin finds the task's listener methods on the public `Task` type, and Firebase without configuration answers `firebase-not-configured` even when its exception arrives wrapped by the reflective call.

**Android channels.** The FCM transport sends `android.channel_id` under the `leptoskit.notifications.` prefix the plugin creates channels with, so a send that names `inbox` reaches the channel the app configured as `inbox`. An id that already carries the prefix is sent unchanged.

**iOS and macOS.** The permission request asks for alerts, badges, and sounds, so a push's `aps.badge` and `aps.sound` take effect where the person allows them; it asked for alerts only.

**macOS.** A push-enabled Mac app (`[native.macos] aps-environment`) now needs `[native.macos] provisioning-profile`, a new setting naming the profile to embed: Apple's push entitlement is restricted, and macOS refuses to launch an app that claims it without a matching embedded profile. Generation stops without the profile. `leptoskit build --platform macos` embeds it at `Contents/embedded.provisionprofile` and signs with the identity in `LEPTOSKIT_MACOS_IDENTITY`, refusing to ad-hoc sign; `--release --unsigned` builds without the push entitlement. `leptoskit publish macos` embeds the profile before signing with the Developer ID identity. This follows Apple's signing rules and has not been run against a real Apple developer account. Apps without `aps-environment` generate and sign exactly as before.

**Web.** The receiver worker claims the pages already open when it activates, so a click can route the tab a person enrolled from, and a click on an action button adds the button's id to the opened address as `lka`.

The guide's samples no longer stop on iOS and macOS at `configure_channel` or enroll after a refused permission, and it now covers Web Push on iPhone and iPad needing a Home Screen install, combining the receiver with an app's own service worker, which `aps-environment` value an iOS build wants, and the entitlement a time-sensitive push needs.

- **Publish to the Solana dApp Store.** `[publish.solana]` adds Solana Mobile's store for Seeker and Saga phones to `leptoskit publish`: without `--submit` the run builds the release-signed APK with your dApp Store keystore, checks the listing text and images, the signing key (which must differ from Google Play's), the `arm64-v8a` build target, and the publishing wallet's balance (0.016 SOL minimum, through one RPC call that may only warn on an outage), and sends nothing, because Solana Mobile's official `dapp-store` tool has no draft stage; with `--submit` it runs that tool against the Publisher Portal with the APK, the wallet keypair, and "What's New" from `store/en-US/release_notes.md`, the portal API key held only in that one tool's environment. The keystore reuses the release-signing variables under `LEPTOSKIT_SOLANA_*` names, `leptoskit publish credentials` lists the six new credentials, and the generated release workflow sets up Node and decodes the keystore for it. _Owner:_ Ifiok Jr. · _Introduced in:_ [78d16fe](https://github.com/leptoskit/leptoskit/commit/78d16feec6021830c6a57befe95ca0eb4b06bae9)

#### `[rpc].path` and `[rpc].live_path` decide where routes serve

_Owner:_ Ifiok Jr. · _Introduced in:_ [8bd1891](https://github.com/leptoskit/leptoskit/commit/8bd1891b8d987c9abe19128343e7107d706ecd88)

`RpcConfig.path`, `live_path`, and `transports` were accepted and validated but no server read them: every app served `/rpc` and `/live` whatever the config said. Now they decide the mounts, once, at the app's entrypoint.

The generated app reads `[rpc]` from `leptoskit.toml` and applies it: operation routes serve at `path` (with the JSON routes kept on the legacy `/api` prefix while JSON is a transport), and `Server::rpc_config` carries the resolved live mount through `Server::live`. `Live` takes the same config, so a router built by hand mounts the socket at `live_path` — `/live` while the config stays at its defaults — with its server-sent events and frame posts right next to it. `path` without `live_path` moves the socket to `{path}/live`; an explicit `live_path` mounts it anywhere.

`leptoskit rpc spec` exports the configured live mount, so regenerated clients open their socket where the app serves it. Generated HTTP calls still name the compile-time `/rpc/<op>` route: keep the default `path` when generated clients call the app directly, and use a custom one behind a proxy or gateway that remounts the prefix.

#### Wearable and smartwatch support across watch OSes and BLE

_Owner:_ Ifiok Jr. · _Introduced in:_ [5b1f3ba](https://github.com/leptoskit/leptoskit/commit/5b1f3ba88e456a3bd5b427603bda36022781444d)

Adds sensor-first wearable application APIs, watch host packaging, and wearable biometric/companion/BLE plugins:

- **`leptoskit_watch`**: adds `WatchKind::HarmonyOs`, Samsung Galaxy Watch and Huawei Watch viewport presets, and the sensor-first `WearableApp` API (`WearableStream`, `SamplingPolicy`, `DeliveryMode`, `WearableStatus`, `WearableSurfaceMode::AutoStatus` / `Custom`, and per-OS `required_permissions()`).
- **`leptoskit_plugin_health`**: adds `SkinTemperature`, `ActiveCalories`, `Distance`, and `FloorsClimbed`; enables `HealthKit` on watchOS alongside iOS; adds HarmonyOS `HealthPlugin.ts` (`@kit.HealthServiceKit`).
- **`leptoskit_plugin_wearable`**: new direct on-watch live & passive biometric sensor plugin covering Wear OS Health Services (`PassiveMonitoringClient`, `MeasureClient`, `SensorManager`, plus optional Samsung Health Sensor SDK runtime hook), watchOS (`HKWorkoutSession`, `HKLiveWorkoutBuilder`, `HKObserverQuery`, `CMPedometer`), and HarmonyOS Wearable (`@kit.SensorServiceKit`).
- **`leptoskit_plugin_watch_sync`**: new bidirectional phone ↔ watch companion sync plugin over Apple `WatchConnectivity`, Android/Wear OS Wearable Data & Message Layer, and Huawei Wear Engine.
- **`leptoskit_plugin_ble`**: new BLE GATT central plugin (`CoreBluetooth`, Android/Wear OS `BluetoothGatt`, HarmonyOS `@kit.ConnectivityKit`) with pure-Rust Bluetooth SIG decoders for Heart Rate + RR-intervals/RMSSD, Pulse Oximeter (`SpO2`), Health Thermometer, Battery, Running Speed & Cadence, and Cycling Power.
- **HarmonyOS plugins**: adds ArkTS implementations for `battery`, `thermal`, and `pedometer`.
- **`leptoskit_config` & `leptoskit_cli`**: adds `[native.wearos]`, `[native.watchos]`, and `[native.harmonyos]` configuration tables; upgrades `wearos` to build Android APKs with Wear OS manifest transformation (watch feature, standalone metadata, tiles/complications); upgrades `watchos` to generate watchOS Xcode build files and invoke `xcodebuild`; updates HarmonyOS host generation to emit configured `deviceTypes` (including `"wearable"`) and `requestPermissions`.

#### Describe independently configured applications

_Owner:_ Ifiok Jr. · _Introduced in:_ [b47bc2e](https://github.com/leptoskit/leptoskit/commit/b47bc2e41920c89e7382665a7fccab1c9508de9d)

Add a separate `leptoskit.workspace.toml` manifest and `lk workspace explain` / `check`. Each application references its own configuration. Startup dependencies are validated independently of call bindings, so bidirectional calls do not become readiness cycles. Unknown references, escaping paths, repeated configurations, conflicting managed identities, and duplicate binding variables fail before a build starts.

The versioned report exposes only selected configuration and environment variable names. It explicitly marks artifacts, API compatibility, database ownership, service identity, and deployment as not checked. Workspace inspection runs no plugin programs or build tools. Image creation, binding injection, change impact, and release orchestration remain future consumers of this graph.

#### Augmented and virtual reality, and Meta Quest apps

_Owner:_ Ifiok Jr. · _Introduced in:_ [996f9b1](https://github.com/leptoskit/leptoskit/commit/996f9b1814996e5f6b3f565803d53266c3359e78)

The new `leptoskit_xr` crate and its `xr` plugin put an app in the real world. Start a session in the best mode the device has — `PhoneAr` through ARKit on iOS and ARCore on Android, `ImmersiveVr` and `ImmersiveAr` on headsets through OpenXR, WebXR in the browser, or `Orientation`, which turns the view with the phone on any device — and read a stream of frames: where the viewer is, each view's projection, the floors, tables, and walls found so far, pinned anchors, the room's light, and controllers and hands. Hit tests find where a tap or a controller's ray meets the real world, against planes and, on phones that measure it, the depth of the room. The camera and depth pictures stay in the native app for the renderer, and the frames your code receives are capped at `[native.plugin-options.xr] max-frame-rate`, always the newest. A recording replays a session exactly, for tests and simulators, with `replay = "…"`.

`leptoskit build --platform quest` builds the Android project as a Meta Quest app, with the manifest Horizon OS expects: OpenXR, head tracking, optional hand tracking and passthrough, the supported headsets, and a full-screen immersive launcher. `[native.quest]` configures it, and `leptoskit dev --platform quest` installs onto a connected headset.

With the `3d` feature, `host::scene3d::draw` draws a `leptoskit_3d` scene into a session: the camera picture behind it, real-world depth hiding what is behind real things, and one view per eye on a headset.

The backends build for their platforms but have not yet run on a device, so treat them as experimental.

### Fixes

- **Keep graph placement separate from application frontends.** Graph artifact placement uses `[infra.frontends.<name>]`. Application build frontends keep their existing top-level `[frontends.<name>]` contract. This avoids colliding configuration types when named application targets and deployment graphs are used in the same toolkit. Graph unit identifiers such as `frontends.docs` and all ownership, route and recovery checks are unchanged. _Owner:_ Ifiok Jr. · _Introduced in:_ [7c788ae](https://github.com/leptoskit/leptoskit/commit/7c788ae0d8522dc8f0576961ef4bcb9121603f70)
- **More ergonomic multi-frontend scaffold.** `--frontends` alone scaffolds landing, app, and admin. Ports are stable per kind. The product app under `apps/app` uses the workspace name as its Cargo package. Frontend names trim whitespace and accept ASCII case; `ApplicationKind` gains `FromStr`, `Display`, and helpers used by the scaffold. Each frontend app uses the same file-router defaults as a normal `leptoskit new`. _Owner:_ Ifiok Jr. · _Introduced in:_ [fa8c81a](https://github.com/leptoskit/leptoskit/commit/fa8c81afb142de65ec3b1f0908c787ac164cb306)

#### Accept managed registries with explicit ports

_Owner:_ Ifiok Jr. · _Introduced in:_ [b47bc2e](https://github.com/leptoskit/leptoskit/commit/b47bc2e41920c89e7382665a7fccab1c9508de9d)

Managed image configuration accepts a registry hostname with a numeric port and an image repository under that registry. Invalid ports, embedded credentials, paths in registry authorities, and tagged image repositories remain rejected.

Verified image packaging accepts `--service <infra.name>` to include Kamal’s required service label in the independently pinned image configuration. Reusing an image checks the label along with the rest of that configuration.

Deployment accepts Docker’s null defaults for absent Podman `Cmd`, `OnBuild`, and `Volumes` inspection fields while still requiring the exact content-addressed image ID. Non-null changes remain rejected.

#### Immersive sessions from Meta Quest panel apps

_Owner:_ Ifiok Jr. · _Introduced in:_ [e897f31](https://github.com/leptoskit/leptoskit/commit/e897f31c27dbc9cb0c39378f56f705e9ef8e83f8)

A Quest app now draws its 3D scene into the headset. Start an `ImmersiveVr` or `ImmersiveAr` session and choose the scene with the new `Session::present(Some(scene_id))`: each eye sees the scene from the player's eye, over passthrough in `ImmersiveAr`. An app built with `[native.quest] immersive = false`, a 2D panel, opens an immersive activity over the panel for the session and comes back to the panel in Horizon Home when the session ends, while its code keeps running and receiving frames.

`leptoskit build --platform quest` declares that activity in every Quest app, marks a panel app's launcher with Horizon OS's 2D category and targets Horizon OS v69, and turns on `leptoskit_xr`'s new `quest` feature (or any async plugin's `quest` feature). For hosts that draw scenes themselves, `leptoskit_3d::host::Hub::draw_scene` draws a scene on another device, such as a headset's, and `Hub::forget_device` moves scenes off it again.

None of this has run on a headset yet.

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_config: project configuration

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_config` loads and validates `leptoskit.toml`: app identity and platforms, server and build settings, native targets and plugin grants, code push channels, routes, translations, the database, sign-in, infrastructure, browser extensions, store publishing, plugins, formatting, and lint settings.

The CLI, build pipeline, dev server, and native hosts all read the same typed configuration, so they agree on what the app is. Unknown keys fail at load time instead of being ignored. The schema is versioned (version 0, unstable until version 1); `leptoskit migrate` upgrades an older file, and the `schema` feature generates its JSON Schema.

### Features

#### Try native plugins written only in Rust

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #506](https://github.com/leptoskit/leptoskit/pull/506)

Set `native.experimental-rust-plugins = true` in `leptoskit.toml` to use the experimental pure-Rust `clipboard` and `haptics` plugins on iOS, macOS, and Android in place of their Swift and Kotlin versions. Without the setting, builds keep the Swift and Kotlin versions.

A plugin of your own can do the same in its `[package.metadata.leptoskit.runtime]` tables. With `rust-context = true`, its factory receives a `leptoskit_bridge::AppContext`, whose `platform()` lends the host's native handles: the `JavaVM` and app `Context` on Android, and the on-screen view controller on iOS and macOS. With `rust-main-thread = true`, its calls run on the platform's UI thread, which UIKit, AppKit, and most Android UI APIs require (`App::install_main` installs such a plugin by hand). Mark a platform's registration `experimental = true` to use it only when the setting is on.

- **Theme tokens in leptoskit.toml and deeper CSS var().** `[theme.colors]` and `[theme.lengths]` in `leptoskit.toml` name design tokens that `Theme::install_maps` loads into the process-wide Theme map. Arbitrary color and length utilities accept `var(--name)`, one nested `var()` fallback, and `/opacity` on color vars. The web adapter prepends `Theme::css()` so the browser sees the same `:root` custom properties native resolves through Theme. Tailwind `@theme` / `tailwind.config` files remain unsupported. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #538](https://github.com/leptoskit/leptoskit/pull/538)

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
