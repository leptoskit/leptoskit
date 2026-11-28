# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Features

#### Generated hosts deliver app-shortcut launches

New iOS, Android, and macOS hosts call `noteLaunch` for home-screen quick actions, launcher shortcuts, and dock-menu picks when `app-shortcuts` is selected. Android activities use `singleTop` so a warm shortcut launch reaches `onNewIntent`.

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-28)

### Features

#### The admin's pages start moving to Rust in the browser

_Owner:_ Ifiok Jr. · _Introduced in:_ [7af0a28](https://github.com/leptoskit/leptoskit/commit/7af0a28a094034a9ef5649ac6fee38e6c4eecf93)

The admin's pages now live in a new crate, `leptoskit_admin_pages`, which builds for the server and for Wasm. `leptoskit_admin` renders a page there from a small model, and the browser hydrates the same component tree, so the page's behaviour is Rust instead of the hand-written `admin.mjs`.

The Glossary is the first page to move. Its theme switch, account menu, search, ⌘K (Ctrl+K elsewhere) to find a screen, and Shift+W for Workings mode all run in Wasm. Every other page still uses `admin.mjs` for now.

Build the client with `leptoskit build --release --platform web` in `crates/leptoskit_admin_pages`, and give the admin its output directory with the new `AdminBuilder::client_dir`, or `lk admin serve --client-dir` (also read from `LEPTOSKIT_ADMIN_CLIENT_DIR`). The admin serves it from `/_leptoskit/admin/pkg/`. Without it, moved pages render but don't respond to presses.

`Environment` now comes from `leptoskit_admin_pages` and is re-exported as before. Parsing one fails with `UnknownEnvironment` instead of `AdminError::Config`.

#### Android alarms that run app code

_Owner:_ Ifiok Jr. · _Introduced in:_ [24d6fbc](https://github.com/leptoskit/leptoskit/commit/24d6fbcae5293e9d397cd1d06938ce3ced7f6b54)

`leptoskit_plugin_android_alarm_manager` ports Flutter's `android_alarm_manager_plus`. Guests schedule one-shot and repeating Android `AlarmManager` alarms by integer ID, with exact, wakeup, allow-while-idle, alarm-clock and reschedule-on-reboot options, check and request the Android 12+ exact-alarm permission (or fall back to inexact timing), cancel and list alarms, and receive fired alarms through a bounded on-device queue that survives process death: `next()` waits for one, `drain()` takes the ones that fired while the app wasn't running. An alarm marked `headless` starts the app's guest without a window when no guest is running, so `handle_launch` can run app code for it within an eight-second budget. iOS, macOS, Windows, Linux, HarmonyOS and the browser return `Unavailable`.

The plugin declares its alarm and boot receivers with the `android-receivers` runtime metadata, which the host-project docs now describe. The Android host gains `NativeHeadlessGuest`, which runs the bundled guest with no window for a receiver or job and discards what it draws.

- **Opt-in FCM for generated Android apps.** `[native.android] push = true` writes the Firebase messaging dependency into `.leptoskit/android/plugin-dependencies.txt`, writes `.leptoskit/android/gradle-plugins.txt` (the Google services plugin id, applied by the app's Gradle script), generates `LeptosKitMessagingService.kt` — whose `onNewToken` lands FCM registration-token refreshes in the notifications plugin's broker — and declares that service in the manifest. With push off, none of it is written and stale copies are removed, so existing apps generate byte-for-byte as before. Generation with push on stops before any write when the `notifications` plugin is not selected, when the app author's `google-services.json` is missing, or when the app's `build.gradle.kts` doesn't read the generated plugins file, naming the file and the lines to add. New scaffolds apply the generated plugins list and put the Google services plugin on the classpath, and the Android snapshots cover the whole opt-in and opt-out. _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)
- **`lk admin serve --analytics-url`.** `lk admin serve` connects the admin's Analytics area to your site's analytics service. It takes the address from `--analytics-url`, or from `LEPTOSKIT_SERVICE_ANALYTICS_URL` when the flag isn't given, and the token from `LEPTOSKIT_SERVICE_ANALYTICS_TOKEN`. If an address is given but the token is missing, it stops and says which variable to set. With neither, the Analytics pages explain how to connect. _Owner:_ Ifiok Jr. · _Introduced in:_ [60e11ae](https://github.com/leptoskit/leptoskit/commit/60e11ae75fdffff7d6da8bc30d4369aced2e9d41)
- **`lk admin serve`.** `lk admin serve --project <repo> --url <admin origin> --main-site <site origin> --client-id <id>` runs the admin for the repositories you name, with the deployment service and its worker inside it. It reads the client secret your site issued from `LEPTOSKIT_ADMIN_CLIENT_SECRET`, its session keys from `LEPTOSKIT_ADMIN_KEYS`, and the deployment service's owner credential from `LEPTOSKIT_CONTROL_KEY`. `--role` changes which role gets in, and `--environment` labels a staging admin. _Owner:_ Ifiok Jr. · _Introduced in:_ [c3f0a63](https://github.com/leptoskit/leptoskit/commit/c3f0a63a4bb79a192f4abb5aabdb45dcfe308222)
- **`leptoskit debug` prints a running app's element tree, runtime errors, and render commits from the devtools hub, and `leptoskit debug open` prints the hub address for the inspector.** `leptoskit mcp` adds `runtime_errors`, `performance`, and `read_logs`, so an agent can see the tree, the errors, the logs, and the commits of an app that `leptoskit dev` is running, and can still capture a screenshot where the platform allows it. `leptoskit device install` installs an Android package, an iOS app, or checks a desktop path. `leptoskit device open` launches it. `leptoskit device sanity` does both and then injects tap, long press, pan, and pinch on the targets that can produce them: every gesture on Android and on an iOS simulator or device, tap and long press and pan on macOS, Windows, and Linux. Linux pinches through `--driver`. A mouse session cannot pinch, and the reply says so. _Owner:_ Ifiok Jr. · _Introduced in:_ [d161fdf](https://github.com/leptoskit/leptoskit/commit/d161fdf435c8f5e64cfb09f2ff53feec5619c921)
- **Add `lk cloud` account commands: device login, private rotating credentials, organizations and members, projects and environments, scoped API tokens, and project linking.** Every command supports JSON output. Deployment commands keep their existing behavior. _Owner:_ Ifiok Jr. · _Introduced in:_ [5e39638](https://github.com/leptoskit/leptoskit/commit/5e396387225c6d0f53e7762277214b6e54998208)
- **`lk cloud` manages an environment's variables, secrets, domains, deployments, rollback and database, publishes to stores from the cloud with `lk cloud publish`, and shows usage.** A CI token from `LEPTOSKIT_CLOUD_TOKEN` now goes only to the hosted cloud or to an origin named by `--url` or `LEPTOSKIT_CLOUD_URL`. _Owner:_ Ifiok Jr. · _Introduced in:_ [b89a935](https://github.com/leptoskit/leptoskit/commit/b89a935c46194ab9626490fc8a7e09f11d7a63fc)

#### Add cloud launch/deploy, storage config, and MCP tools

_Owner:_ Ifiok Jr. · _Introduced in:_ [94859ef](https://github.com/leptoskit/leptoskit/commit/94859efebd25f1bfb3f1c055c3172de4c309cad9)

- Adds `[cloud.storage.buckets.<name>]` configuration (`public`, `max_size_gb`) and updates the published `leptoskit.toml` JSON schemas.
- Adds `lk cloud launch`, `lk cloud deploy`, `lk cloud status`, `lk cloud logs [--follow]`, `lk cloud estimate`, and `lk cloud open`, wires `lk deploy` to dispatch to LeptosKit Cloud whenever `[cloud].project` is set, and adds Cargo-style external subcommand dispatch (`lk cloud <name>` → `leptoskit-cloud-<name>` on `PATH`, enabling `lk cloud platform`, `lk cloud worker`, and `lk cloud control`).
- Adds `--web-only` and `--cloud` flags to `lk new` and `lk generate app`, allows `--frontends` to combine with `--db`, `--auth`, `--web-only`, and `--cloud`, and falls back with a clear warning instead of hard-failing `lk dev` when `monosecret` is not installed.
- Exposes six cloud and database tools on `lk mcp` (`cloud_estimate`, `cloud_deploy`, `cloud_status`, `cloud_logs`, `cloud_secrets_list`, `db_check`) without ever returning secret values.

- **Show hosted notifications with `lk cloud notifications`.** `lk cloud notifications` shows whether the cloud's shared notifications service serves an environment, which push providers are set up, and the project's pushes this month. A failure shows the service's own reason, and a credential rotation says to deploy. The cloud guide has a section on hosted notifications. _Owner:_ Ifiok Jr. · _Introduced in:_ [d4f1c23](https://github.com/leptoskit/leptoskit/commit/d4f1c236b3130b7c7a9266215fc44b66fdc4054a)

#### A free plan's hosted pushes stop at its monthly allowance

_Owner:_ Ifiok Jr. · _Introduced in:_ [3d2887b](https://github.com/leptoskit/leptoskit/commit/3d2887b72737ef9972a8374a5a6d79b24500ff5a)

When an organisation on a plan that costs nothing has used its plan's `notification_pushes` allowance for the month, LeptosKit Cloud stops its hosted notifications from sending pushes until the next month or until it moves to a paid plan. The inbox, live updates and every other operation keep working, and `lk cloud notifications` says `Pushes are paused`. The pushes a service sends before it hears are free up to the allowance again, so they don't reach the free plan's spend cap and stop deploys; past twice the allowance they are priced and reach it, as before. A free plan that lists no `notification_pushes` allowance is billed as before. The price itself is the operator's catalogue: for example 10,000 pushes a month on every plan, then 10 cents per 1,000 on paid plans.

The control plane's `notification_tenants` answer gains `paused`, the cluster's tenants whose pushes are stopped, and the `notifications` summary gains `pushes_paused`; both default to empty and `false`. The hub stops and starts each tenant's pushes on its answer without reopening it.

`leptoskit_notifications` gains `service::push::Paused`, a switch on `service::push::Push` that a server's runner can flip while it runs. While it is on, each push is skipped with the new delivery reason `paused`, and `send_test` is refused with a `400`. Code that builds `Push` by hand adds `paused: Paused::default()`.

- **`leptoskit dev` serves phone guests on the local network and prints a QR code for them.** The LeptosKit app scans that code, or takes the address, and runs the unsigned debug guest. Only a local `http` or `https` address is accepted. _Owner:_ Ifiok Jr. · _Introduced in:_ [a7b4d40](https://github.com/leptoskit/leptoskit/commit/a7b4d400154fef9c3dcc13e434766a66240c90b2)

#### Add extension browser detection and full side panel API

_Owner:_ Ifiok Jr. · _Introduced in:_ [c4aa7df](https://github.com/leptoskit/leptoskit/commit/c4aa7df45dd07fbead27b9b05bba1d67ecfacd7f)

- `leptoskit_extension::browser` adds runtime browser detection (`ExtensionBrowser` covering Chrome, Edge, Brave, Opera, Arc, Vivaldi, Firefox, and Safari; `BrowserFamily`; `BrowserInfo`; `BrowserSwitch`; and `with_browser` test scoping).
- `leptoskit_extension::side_panel` adds full Chromium `chrome.sidePanel` and Firefox `browser.sidebarAction` support: `open_target`, `open_for_tab`, `close`, `toggle`, `is_open`, `set_options` / `get_options`, `set_behavior` / `get_behavior`, `get_layout`, `set_title` / `get_title`, and `capabilities`.
- Adds `leptoskit_extension::windows`, `leptoskit_extension::offscreen`, `leptoskit_extension::omnibox`, and `leptoskit_extension::native_messaging`.
- Adds `[extension].optional_permissions` and `[extension.side_panel]` `title` / `open_at_install` options to `leptoskit_config` and generated MV3 manifests.

- **New database and sign-in apps start with a real notes screen.** A `leptoskit new --db sqlite`, `--db postgres`, or `--auth` project now opens on a notes screen instead of the counter. It uses the shared typed `Notes` and `AddNote` operations the scaffold already publishes: loading, empty, failed, and loaded states, body validation before the mutation, save feedback, and a Refresh button. The browser follows the live socket; native apps use the HTTP plugin and Refresh because their generated hosts don't grant the WebSocket plugin, and the Linux, Windows, and terminal hosts say plainly that they can't reach the server. With `--auth`, the screen is rebuilt per account and the previous account's cached notes are dropped first, so signing out or signing in as someone else never shows the previous person's list. `--db` now also grants the native HTTP plugin and configures its origin. Projects without a database keep the counter. _Owner:_ Ifiok Jr. · _Introduced in:_ [1bd0618](https://github.com/leptoskit/leptoskit/commit/1bd0618c284e4dff885524d3d6a23db9cfc668f8)
- **Opt-in APNs for generated iOS apps.** `[native.ios] aps-environment = "development"` (or `"production"`) writes the `aps-environment` entitlement — Apple's exact string, not a boolean — into `.leptoskit/ios/App.entitlements`, and the generated app delegate forwards `didRegisterForRemoteNotificationsWithDeviceToken` and `didFailToRegisterForRemoteNotificationsWithError` to the notifications plugin's broker. It must match the app's provisioning profile and the notifications service's `LEPTOSKIT_NOTIFICATIONS_APNS_ENVIRONMENT`. Apps without the setting generate exactly as before, with no push entitlement, so local-only notifications users stay local-only; no provisioning or certificate access happens in the build. The configuration schema carries the new key. _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)
- **Opt-in APNs for generated macOS apps.** `[native.macos] aps-environment = "development"` (or `"production"`) writes the `com.apple.developer.aps-environment` entitlement — Apple's macOS key, its exact string — into `.leptoskit/macos/App.entitlements`; the local build's ad-hoc signature and a `leptoskit publish macos` distribution both sign the app with it, and the generated macOS app delegate forwards `didRegisterForRemoteNotificationsWithDeviceToken` and `didFailToRegisterForRemoteNotificationsWithError` to the notifications plugin's broker. It must match the notifications service's `LEPTOSKIT_NOTIFICATIONS_APNS_ENVIRONMENT`. Apps without the setting generate exactly as before, with no push entitlement, so local-only notifications users stay local-only. _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)

#### Meta Horizon platform extension and MHCP packager

_Owner:_ Ifiok Jr. · _Introduced in:_ [79d602f](https://github.com/leptoskit/leptoskit/commit/79d602ffab7e56bd032ca83be7a1fae42d704d98)

Add the experimental `leptoskit_platform_horizon` platform extension (`horizon`), registered under `leptoskit native platform` alongside `harmonyos` and `oculus`.

- **`leptoskit native platform scaffold horizon`** generates `hosts/horizon/` with Meta Horizon Worlds Desktop Editor TypeScript Custom UI host scripts (`scripts/scene.ts`, `scripts/LeptosKitHost.ts`, `scripts/scene_test.ts`) implementing leptoskit `version: 1` render transactions over `horizon/ui`, plus Meta Horizon Creator Program (MHCP) offline web submission templates (`mhcp/index.html`, `mhcp/BUILD_LOG.md`).
- **`leptoskit native platform package horizon`** builds and validates a deterministic offline HTML5/WebGL/Wasm `.zip` archive for MHCP hackathon submissions: rewrites root-relative asset references in `index.html` to `./...`, omits `.br`/`.gz` server sidecars, rejects external CDN/network URLs and missing relative assets, and enforces the configurable `--max-mb` size cap (default 35 MiB).

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

#### Taps, foreground pushes, and data messages reach app code

_Owner:_ Ifiok Jr. · _Introduced in:_ [b1d0c8b](https://github.com/leptoskit/leptoskit/commit/b1d0c8b741c4bd284362b53d998bd3c0b7b707cc)

A push used to end at the banner: nothing told the app which notification was tapped, a push that arrived while an iOS or macOS app was open showed nothing, an Android app in the foreground dropped it, and a silent data message ran no app code. One receiver now answers all of it on iOS, macOS, Android, and the web.

`leptoskit_notifications` gains the `receive` module, built with the `native` or `browser` feature. `Received` is `Opened` (a tap: the notification's id, the send's `link`, the pressed action's id, the send's `data`, a local notification's id, and `launched` for the tap that started the app), `Shown` (a notification arrived while the app was open), or `Data` (a silent data message with its `data` and `badge`). `Opened::route(inbox_route)` answers where the tap should go. `native::Receiver` and `browser::Receiver` both have `new`, `next`, and `pending`, and answer the launching tap first. Where nothing can be received they answer `ReceiveError::Unavailable`.

`leptoskit_plugin_notifications` gains method `RECEIVED` (10) on iOS, macOS, and Android, with `ReceivedItem`, `ReceivedKind`, `encode_received_wait`, `decode_received_wait`, and `guest::Notifications::received`. The host keeps the sixteen most recent items and hands each over once. On iOS and macOS a notification-centre delegate shows a notification while the app is open and records taps; on Android the plugin posts a push that Firebase hands to a foreground app, reads a tap from the activity's intent, and a local `show()` now opens the app when tapped.

`leptoskit_cli` generates the host side: new iOS and macOS apps install the delegate before launch finishes and forward `didReceiveRemoteNotification`, an iOS app with `aps-environment` set declares the `remote-notification` background mode, and the generated Android messaging service forwards `onMessageReceived`.

**What changes for an existing app:**

- `hosts/ios/App.swift` and `hosts/macos/App.swift` are written once and are the app's own, so an app generated earlier must add the delegate lines the guide lists. Without them it behaves as before. Android apps need no change.
- A push that arrives while an iOS, macOS, or Android app is open now shows a banner. An app that drew its own on those platforms would show two.
- The web receiver adds the notification's id as `lkn` to the address it opens for a send's own `link`, as it already did for the inbox route.
- The web receiver first offers a click to an open page and waits up to half a second for the page's answer before loading the address. Only a page that creates a `browser::Receiver` answers; other pages see a click arrive up to half a second later than before.
- A web push flagged as a data message, which the service never sends, now shows the generic banner when no page is in view instead of showing nothing.

The Swift and Kotlin code in this change was not compiled where it was written; its tests run in the Apple and Android CI jobs.

- **`leptoskit dev` rebuilds an edit with a line-table dev profile, compiles the server and the wasm client in parallel when the edit needs both, and copies assets without invoking cargo.** `leptoskit budgets` runs the caps and boot checks and names the build, test, lint, and workspace commands. _Owner:_ Ifiok Jr. · _Introduced in:_ [1f7f146](https://github.com/leptoskit/leptoskit/commit/1f7f1462a17a9a33eccb933909f6cdae20d03052)
- **Publish to the Solana dApp Store.** `[publish.solana]` adds Solana Mobile's store for Seeker and Saga phones to `leptoskit publish`: without `--submit` the run builds the release-signed APK with your dApp Store keystore, checks the listing text and images, the signing key (which must differ from Google Play's), the `arm64-v8a` build target, and the publishing wallet's balance (0.016 SOL minimum, through one RPC call that may only warn on an outage), and sends nothing, because Solana Mobile's official `dapp-store` tool has no draft stage; with `--submit` it runs that tool against the Publisher Portal with the APK, the wallet keypair, and "What's New" from `store/en-US/release_notes.md`, the portal API key held only in that one tool's environment. The keystore reuses the release-signing variables under `LEPTOSKIT_SOLANA_*` names, `leptoskit publish credentials` lists the six new credentials, and the generated release workflow sets up Node and decodes the keystore for it. _Owner:_ Ifiok Jr. · _Introduced in:_ [78d16fe](https://github.com/leptoskit/leptoskit/commit/78d16feec6021830c6a57befe95ca0eb4b06bae9)

#### Rate limits, timeouts, and guards for public operations

_Owner:_ Ifiok Jr. · _Introduced in:_ [4c41c6e](https://github.com/leptoskit/leptoskit/commit/4c41c6e61ebf8a586cfdd4e1e6183a9beafbffdc)

Every query, mutation, action, and stream a client can call can now be limited before its body runs, over HTTP and over a live connection alike.

```rust
use leptoskit::rpc::server::{Rate, RateLimit, Server};

let rpc = Server::new(db)
	.identify(auth)
	.client_ip_header("fly-client-ip")
	.rate_limit(RateLimit::new().anonymous(Rate::per_minute(60)))
	.guard(SignedInOnly);
```

- `Server::rate_limit(RateLimit)` counts each caller: by address when nobody is signed in (IPv6 by its /64), and by `Identity::caller` when someone is, with `RateLimit::key` to count every session of one person together. The defaults are 120 calls a minute per address and 600 per signed-in caller, refilled evenly (GCRA). Services and the app calling them aren't counted unless `RateLimit::internal` says so. A refused call is `429` with `Retry-After`, which clients already read as `FnError::Refused` with the code `rate_limited`.
- Counts live in a bounded `MemoryStore` by default. Implement `RateStore` to share them between servers, for example over Redis.
- `#[rpc::query(rate_limit = 10 / 1m, timeout = 5s, max_body = 64KiB)]`, and the same on `#[rpc::mutation]` and `#[rpc::action]`, set one operation's limits. An operation's own `rate_limit` holds even when the server sets no `RateLimit`. They are on `Registration::limits` as `OperationLimits`.
- A call past its timeout is answered `504` with the code `timed_out` (`rpc::server::TIMED_OUT`); arguments over the size limit are answered `413` with `too_large` (`rpc::server::TOO_LARGE`) instead of a `400`. `Server::timeout` and `Server::max_body` set the defaults; the body limit is still 2 MiB unless changed.
- `Server::guard(impl Guard)` runs an app's own check before every call. A `Guard` sees a `Call`: the operation's name and kind, the identity, the client's address, and whether it came over HTTP or a live connection. `Live` and `LiveStreams` take the same `guard`, `rate_limit`, and `client_ip_header`, and `Server::live()` passes its server's on.
- The HTTP routes now identify the caller and run the guards before reading the request body, so a refused caller costs no more than their headers. Calls the server makes in process while it renders a page are not checked again.

Rate limiting is opt-in for existing apps. New database and sign-in projects from `leptoskit new` start with `Server::new(db).rate_limit(RateLimit::new())` in the backend's `operations`.

`Registration` has a new `limits` field: code that builds one by hand adds `limits: OperationLimits::NONE`.

- **New apps use the file router by default.** `leptoskit new` enables the `router` plugin, scaffolds `index.rs` / `_layout.rs` / `+not-found.rs` under the app crate's `src/routes/`, commits a generated `routes.rs`, and depends on `leptoskit_router`. Builder `[lint]` stays the default so the starter `Page` and `Layout` components stay clean under `leptoskit lint --deny-warnings`. _Owner:_ Ifiok Jr. · _Introduced in:_ [a8d02ab](https://github.com/leptoskit/leptoskit/commit/a8d02abfb577de8ac09d481ee5844814147fc6aa)

#### Share files, subject and title, and where it went

_Owner:_ Ifiok Jr. · _Introduced in:_ [832bc2b](https://github.com/leptoskit/leptoskit/commit/832bc2b072feb4541efe34a14c6f53f8a220d6a7) · _Last updated in:_ [d0a8882](https://github.com/leptoskit/leptoskit/commit/d0a8882eeb43751a00f3e89cc07fb7b4e0b97413)

`leptoskit_plugin_sharing` now covers Flutter `share_plus`'s features. A share can carry up to 32 files, optionally with text: `File::app("exports/report.pdf")` names a file in your app's files directory (the root the `filesystem` plugin writes), and `File::bytes("chart.png", bytes)` shares data from memory, each with an optional MIME type and display name. `with_subject` sets the email subject, `with_title` the sheet or chooser title, `with_anchor` where the iPad popover or macOS picker points, and `excluding` leaves iOS activities out of the sheet.

`share` now returns a `ShareResult`: `Success` with the destination the user picked (the iOS activity type, the macOS service title, or the Android component), `Dismissed`, or `Unavailable` when the platform cannot tell, as in browsers. A result never confirms delivery. On Android this replaces the old behaviour, where dismissing the chooser and choosing an app looked the same.

Native hosts check that an app file stays inside the files directory, refusing symlinks that lead out of it, and copy shared files into a private cache that the next file share clears. On Android the plugin ships its own `FileProvider` and grants read access only to the chosen app, only for those copies; it adds `androidx.core:core`. Browsers share in-memory files through Web Share Level 2 and return `Unavailable`, without a download fallback, when they cannot. Windows and Linux still have no native share sheet.

Breaking: the wire protocol is versioned and changed. The previous request (a kind and one value) and its empty reply are refused, `share` returns `ShareResult` instead of `()`, `web::share` resolves to a `ShareResult` (a browser cancellation is `Dismissed`, no longer a `Cancelled` error, and an iOS or macOS dismissal is `Dismissed` rather than a `Cancelled` error), and `Content::as_str` and `Content::is_url` are replaced by `shared_text`, `shared_url`, `shared_files`, `subject` and `title`.

The CLI reads a new `android-file-providers` plugin metadata key: it copies the plugin's paths XML into the generated resources and declares an unexported `FileProvider` in the generated Android manifest.

- **`leptoskit new` opens on a home screen, a kit of the shared controls, and a list of the platforms the app runs on, in a tab bar.** The home screen still counts with **Add one**. _Owner:_ Ifiok Jr. · _Introduced in:_ [a7b4d40](https://github.com/leptoskit/leptoskit/commit/a7b4d400154fef9c3dcc13e434766a66240c90b2)

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

#### Release independently built workspace applications

_Owner:_ Ifiok Jr. · _Introduced in:_ [b47bc2e](https://github.com/leptoskit/leptoskit/commit/b47bc2e41920c89e7382665a7fccab1c9508de9d)

Add `workspace plan` and `workspace deploy` for a set of prebuilt, independently pinned image descriptors. Plans freeze the workspace, parsed app configurations, artifact pins, readiness order, and credential variable references. Execution stages exact descriptors, invokes the existing managed release checks in each app directory, and retains a durable progress journal when a later app fails.

Add `deploy --reuse-current` to revalidate a current verified release without booting it again. Reuse checks current authorization, descriptor identity, recorded configuration, database history, image identity, running-container membership, effective app environment, and `/readyz`. Older history without configuration evidence requires a new version. Changed credentials fail reuse rather than being silently ignored. Successful release operation state is now written after backup installation and pruning finish.

Workspace retries revalidate every selected app, including previously completed entries; the journal never authorizes reuse. Automatic builds, source-change impact, private networking, caller identity, and cross-application compatibility remain separate work.

#### Estimated depth for phones that measure none

_Owner:_ Ifiok Jr. · _Introduced in:_ [b878753](https://github.com/leptoskit/leptoskit/commit/b878753721d8c81819b8b332cc0a7b9455097c86)

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

_Owner:_ Ifiok Jr. · _Introduced in:_ [c215d24](https://github.com/leptoskit/leptoskit/commit/c215d24d84051613fef7bf2171094a72208dabce)

A `Scene3d` whose backdrop is `Backdrop::Camera` now draws over the camera while an `xr` session runs on a phone. The generated iOS and Android apps hand the scene the session's newest frame: the camera picture behind the models, turned upright for the screen's rotation and cropped to fill the view, the session's own viewpoint in place of the scene's camera, and the room's measured depth hiding the parts of the scene behind real things. Start the session with `Features::CAMERA_IMAGE` (and `Features::DEPTH` for occlusion). Without a session the scene draws as before, transparent behind its models.

Android phones without ARCore, such as the Solana Seeker, now show the back camera in `Orientation` sessions too, through Camera2, with the rotation sensor turning the view and the camera's own field of view keeping the scene to the picture's scale. Such a session leaves the picture out, rather than failing, until the user allows the camera, unless it requires `CAMERA_IMAGE`.

On Android a `Scene3d` showing the camera moves below the window's own views, so the interface laid over it stays visible.

For hosts that draw scenes themselves, `leptoskit_3d::host::Hub::set_camera_feed` takes the picture a camera backdrop draws over, `View::set_rotation` the interface's rotation, and the bridge adds `lk_scene3d_view_set_rotation` and `lk_scene3d_view_shows_camera`. `leptoskit_xr`'s new `scene3d` feature makes the plugin's sessions that feed, and `leptoskit build` turns it on, with any other async plugin's `scene3d` feature, when the app shows 3D scenes. `leptoskit_xr::host::install_depth_estimator` (or `XrRuntime::set_depth_estimator`) takes a `DepthEstimator`, which turns small CPU copies of the camera picture (`CameraFrame`, with the camera's `Intrinsics`) into depth for phones that measure none.

#### Immersive sessions from Meta Quest panel apps

_Owner:_ Ifiok Jr. · _Introduced in:_ [e897f31](https://github.com/leptoskit/leptoskit/commit/e897f31c27dbc9cb0c39378f56f705e9ef8e83f8)

A Quest app now draws its 3D scene into the headset. Start an `ImmersiveVr` or `ImmersiveAr` session and choose the scene with the new `Session::present(Some(scene_id))`: each eye sees the scene from the player's eye, over passthrough in `ImmersiveAr`. An app built with `[native.quest] immersive = false`, a 2D panel, opens an immersive activity over the panel for the session and comes back to the panel in Horizon Home when the session ends, while its code keeps running and receiving frames.

`leptoskit build --platform quest` declares that activity in every Quest app, marks a panel app's launcher with Horizon OS's 2D category and targets Horizon OS v69, and turns on `leptoskit_xr`'s new `quest` feature (or any async plugin's `quest` feature). For hosts that draw scenes themselves, `leptoskit_3d::host::Hub::draw_scene` draws a scene on another device, such as a headset's, and `Hub::forget_device` moves scenes off it again.

None of this has run on a headset yet.

#### Augmented and virtual reality, and Meta Quest apps

_Owner:_ Ifiok Jr. · _Introduced in:_ [996f9b1](https://github.com/leptoskit/leptoskit/commit/996f9b1814996e5f6b3f565803d53266c3359e78)

The new `leptoskit_xr` crate and its `xr` plugin put an app in the real world. Start a session in the best mode the device has — `PhoneAr` through ARKit on iOS and ARCore on Android, `ImmersiveVr` and `ImmersiveAr` on headsets through OpenXR, WebXR in the browser, or `Orientation`, which turns the view with the phone on any device — and read a stream of frames: where the viewer is, each view's projection, the floors, tables, and walls found so far, pinned anchors, the room's light, and controllers and hands. Hit tests find where a tap or a controller's ray meets the real world, against planes and, on phones that measure it, the depth of the room. The camera and depth pictures stay in the native app for the renderer, and the frames your code receives are capped at `[native.plugin-options.xr] max-frame-rate`, always the newest. A recording replays a session exactly, for tests and simulators, with `replay = "…"`.

`leptoskit build --platform quest` builds the Android project as a Meta Quest app, with the manifest Horizon OS expects: OpenXR, head tracking, optional hand tracking and passthrough, the supported headsets, and a full-screen immersive launcher. `[native.quest]` configures it, and `leptoskit dev --platform quest` installs onto a connected headset.

With the `3d` feature, `host::scene3d::draw` draws a `leptoskit_3d` scene into a session: the camera picture behind it, real-world depth hiding what is behind real things, and one view per eye on a headset.

The backends build for their platforms but have not yet run on a device, so treat them as experimental.

### Fixes

- **`leptoskit native build` for Android reports the APK size from the app's own `android/` project.** It was still looking under `hosts/android`, so a build that produced an APK said there was nothing to measure. _Owner:_ Ifiok Jr. · _Introduced in:_ [a8fec99](https://github.com/leptoskit/leptoskit/commit/a8fec995896be1fea532ee589dfb8869c28f1df7)
- **Keep Android plugins' native libraries on the JNI link line when Cargo prints coloured output, including AAudio for audio plugins.** _Owner:_ Ifiok Jr. · _Introduced in:_ [7237130](https://github.com/leptoskit/leptoskit/commit/723713098b125431f6e762cbf4b6f3ebe5506a23)
- **An unknown or custom widget is an empty container on Android, in both the backend and the generated app.** It used to stop the host, which closed the app. GTK already drew one as an empty view. _Owner:_ Ifiok Jr. · _Introduced in:_ [a7b4d40](https://github.com/leptoskit/leptoskit/commit/a7b4d400154fef9c3dcc13e434766a66240c90b2)
- **`leptoskit device open` fails when the app never takes the screen, instead of reporting success while the launcher is still in front.** A tap, long press, or pan past the display fails too: `input` exits 0 for those points and delivers nothing. An Android pinch still waits for its touchscreen to attach, but that wait is no longer part of the gesture clock, so a 300 ms pinch is 300 ms of contact with both pointers. `device list` marks an AVD as running when that emulator is already up. `leptoskit mcp http` answers `application/json`, and the screenshot tool's schema resolves. _Owner:_ Ifiok Jr. · _Introduced in:_ [a9678a5](https://github.com/leptoskit/leptoskit/commit/a9678a57ec571f2318342fd1f04d68c9666b072d)

#### Android and Apple hosts ship every view they draw

_Owner:_ Ifiok Jr. · _Introduced in:_ [6f7114d](https://github.com/leptoskit/leptoskit/commit/6f7114d8355acef2a1e98cba223a58c5a7a15f5b)

Generated Android projects now compile: the platform inventories include `NativeCamera.kt`, `NativeMap.kt`, `NativeLivePhoto.kt`, and `NativeServiceWakeBroker.kt`, so `NativeWidgetRenderer` and `LeptosKitView` resolve the views they draw and the background wake they install. The macOS and iOS inventories likewise include `NativeCamera.swift`, `NativeMap.swift`, and `NativeLivePhoto.swift`, which their widget renderers already used.

A test now fails when a shipped Kotlin or Swift source refers to a type declared only in a file the platform inventory leaves out, so this class of missing source is caught before Gradle or Xcode sees it.

- **Android plugin builds read rustc's `native-static-libs` note even when Cargo colors it.** A colored `note:` label was skipped, so the generated CMake link line omitted libraries rustc named, including `aaudio` and `OpenSLES`, and the plugin failed to link. _Owner:_ Ifiok Jr. · _Introduced in:_ [f5104c6](https://github.com/leptoskit/leptoskit/commit/f5104c6f382ac2e42b512fb160280ee8d375cb9e)

#### Android hosts link the libraries their plugins need

_Owner:_ Ifiok Jr. · _Introduced in:_ [c0c5013](https://github.com/leptoskit/leptoskit/commit/c0c5013b708e17723cb48062036b40f22f694a57)

Generated Android and Quest projects now link. The host's `CMakeLists.txt` names the Rust archive and a fixed list of NDK libraries, and a static archive carries none of Cargo's link directives, so a plugin whose Rust code uses a platform library outside that list failed at `ld.lld` with undefined symbols — `AAudio_createStreamBuilder` and its neighbours once the audio plugin was selected.

The Android host build now asks rustc for the libraries the archive needs (`cargo rustc -- --print native-static-libs`) and writes them into the generated link line, in rustc's order and without repeats, so any plugin's libraries (`aaudio`, `OpenSLES`, `mediandk`, `camera2ndk`, `EGL`, `GLESv3`, `vulkan`, …) arrive with the plugin rather than with a list the CLI has to keep in step.

- **An Android build points cc-rs at the NDK's `llvm-ranlib`, as it already did for `llvm-ar`.** A dependency that builds a vendored OpenSSL through `make` no longer fails with `aarch64-linux-android-ranlib: not found` on machines without `llvm-ranlib` on `PATH`. _Owner:_ Ifiok Jr. · _Introduced in:_ [d163d4c](https://github.com/leptoskit/leptoskit/commit/d163d4ce01a3de4209f807b028ade3ed6cf7df1c)
- **Keep the Android API level in native C and C++ builds when a compiler cache wraps the NDK launcher.** _Owner:_ Ifiok Jr. · _Introduced in:_ [7237130](https://github.com/leptoskit/leptoskit/commit/723713098b125431f6e762cbf4b6f3ebe5506a23)
- **An Android pinch from `leptoskit device` is two pointers again.** Two `input swipe`s each own one pointer, and on API 35 the activity received only one of them while the step still reported success. The pinch now registers one touchscreen with `uinput` and moves both fingers in the same report. _Owner:_ Ifiok Jr. · _Introduced in:_ [b969920](https://github.com/leptoskit/leptoskit/commit/b9699202035dc9c7245236c7d5e8750e816f3e2c)

#### Android Back goes back a route

_Owner:_ Ifiok Jr. · _Introduced in:_ [a93214e](https://github.com/leptoskit/leptoskit/commit/a93214edc8e5f6c7999c628285093f15cf188639)

The Back gesture and Back button in an Android app now go back one route through the file router, and leave the app only from the first route. Before, Back always left the app: Android 16 delivers Back to an app that targets it only through an `OnBackInvokedCallback`, which the generated activity never registered, and on every Android version the host's question to the router went nowhere.

- `leptoskit_wasm`: `WasmHost::system_back` offers Back to the guest's optional `leptoskit_system_back` export, then flushes and commits what changed. A guest without the export answers `EventOutcome::Unhandled`, and the platform leaves.
- `leptoskit_bridge_ffi`: `lk_guest_system_back` offers Back on the guest thread, and `lk_guest_take_system_back` takes the answers in order. `LeptosKitApplicationView.offerSystemBack` now takes a callback, `(Boolean) -> Unit`, that runs on the main thread with `true` when the router went back.
- `leptoskit_cli`: the generated `MainActivity.kt` registers an `OnBackInvokedCallback` on Android 13 and later and keeps `onBackPressed` for earlier versions; the generated manifest sets `android:enableOnBackInvokedCallback="true"`.

Apps scaffolded before this keep leaving on Back until they copy the new `MainActivity.kt` Back code and the manifest attribute from a new project; [the Android page](https://leptoskit.dev/docs/platforms/android.html#the-back-gesture) lists them.

#### Boot iOS for sanity checks and use a mouse on macOS

_Owner:_ Ifiok Jr. · _Introduced in:_ [c5db144](https://github.com/leptoskit/leptoskit/commit/c5db144b468b11cafed744ffb4ac59356e8488d9) · _Last updated in:_ [b4efffc](https://github.com/leptoskit/leptoskit/commit/b4efffc4eb47c86b523454bfc2508c0fcd97a1e1)

`leptoskit device sanity` on an iOS simulator now boots that simulator before `simctl install`. A shut-down simulator refuses the install. One that is already booted stays up: the boot step treats that state as success. A sanity run that does not install, and a run on a real iPhone, still do not boot.

Pointer actions sent to a macOS session use a mouse. Mac2 does not accept a touch pointer. iOS, Android, Linux, Windows, and the web still send touch.

- **Refuse graph-root auth generation before writes.** Auth generation checks that it is running in an application configuration before writing client screens or dependency features. Deployment graph roots now fail before any generation writes, including with `--force`, instead of rejecting the application-only auth settings after partially changing a workspace client. _Owner:_ Ifiok Jr. · _Introduced in:_ [ac480c5](https://github.com/leptoskit/leptoskit/commit/ac480c5935b3132c20821e2b5143b1e3e5c884fe)
- **Generate auth in the selected backend package.** Auth generation follows `build.server_package`, including a named target's resolved backend, while keeping client selection independent. It defaults to `app.name` when no server package is configured. The generator no longer writes server modules or dependency features into an unselected package that happens to share the application's name. _Owner:_ Ifiok Jr. · _Introduced in:_ [3bfdd7f](https://github.com/leptoskit/leptoskit/commit/3bfdd7fec9c797b7262acefce6264a7124887ef8)
- **The background plugin no longer crashes iOS apps.** iOS ends an app that registers a `BGTaskScheduler` handler after launch finishes, or registers the same one twice, and the `background` plugin registered its handlers in `start`, which the guest calls once the app is already running. The handlers are now registered by `BackgroundPlugin.registerLaunchHandlers()`, which the generated iOS app delegate calls from `application(_:willFinishLaunchingWithOptions:)`; repeated calls are ignored, and a task whose identifier or background mode is missing from the app's `Info.plist` is skipped instead of registered. `start` now only schedules the next app refresh. An app whose `hosts/ios/App.swift` was generated before this keeps launching, and its `start` answers `Unavailable` on iOS until it adds the call. _Owner:_ Ifiok Jr. · _Introduced in:_ [1c9b39d](https://github.com/leptoskit/leptoskit/commit/1c9b39d6edb307df0591ef530ac490b4576942e9)

#### Cloud serves notifications from a shared hub

_Owner:_ Ifiok Jr. · _Introduced in:_ [78c0bf4](https://github.com/leptoskit/leptoskit/commit/78c0bf47afd1e2e6c741d5d3cb79f0692ef0b985)

An app that declares `[services.notifications]` with `crate = "leptoskit_notifications"` runs no process for it in LeptosKit Cloud. Its environment becomes a tenant of its cluster's notifications hub, a worker of the new role `notifications`:

- `lk cloud deploy` reports the service as `hosted = "notifications"`, with whether the app forwards identity. The build worker builds nothing for it.
- A deployment makes the tenant once: a database of its own, `ntf_` and the environment's ID, on the cluster's Postgres server, and a token the app and the hub share, both sealed in the vault. A cluster without a hub or a Postgres server refuses the deployment as `notifications_unavailable`.
- The release bridges to the hub as a platform service, routes `/_leptoskit/services/notifications/` on every hostname to it when the service is public, and gives the app `LEPTOSKIT_SERVICE_NOTIFICATIONS_URL`, `_IDENTITY` and, leased with the release, `_TOKEN`. The cluster's worker is leased the database's address as `LEPTOSKIT_NOTIFICATIONS_DATABASE_URL`.
- Push credentials and settings are the service's own `LEPTOSKIT_NOTIFICATIONS_*` names, set as secrets or variables. They are exempt from the reserved prefix, never given to the app's processes, and changing one reloads the tenant on the hub without a deployment.
- The hub asks for its cluster's tenants with `notification_tenants`, is given each changed one's values (recorded in `notification_leases` first), and reports how each is doing and how many pushes each sent with `report_notifications`. Pushes are the new meter `notification_pushes`.
- `notifications` tells anyone who can view the project the tenant's state, which providers have settings, and the project's pushes this month.

#### Publish to app and extension stores from the cloud

_Owner:_ Ifiok Jr. · _Introduced in:_ [101b2ee](https://github.com/leptoskit/leptoskit/commit/101b2ee1e7acf5a060554a6a0bd7c9f597ff7903)

`leptoskit_cloud::publishing` adds `publish`, which builds an uploaded archive for its stores and uploads it on one of the organisation's build workers, as `lk publish` does, with the environment's secrets named as `lk publish` reads them. `publication`, `publications` and `publication_log` read a publish back, with its report and its log. A publish is never run twice, since it may have reached a store. `JobKind`, `JobInput` and `JobOutcome` gain `Publish`; store credentials may now be set as secrets, and no release is given them.

`leptoskit publish` takes a keystore's `_BASE64` variable in place of its path for Google Play and the Solana dApp Store, and with `LEPTOSKIT_PUBLISH_CONTENTS_ONLY=1` refuses a credential that names a file.

- **Reading a watched file does not restart the app.** `leptoskit dev` ignores filesystem access events before scheduling a rebuild or marking the configuration, database tables, or Git branch as changed. On Linux, the compiler and CLI reading their inputs could otherwise trigger another build repeatedly, even without an edit. Source edits and file replacements still rebuild the app. _Owner:_ Ifiok Jr. · _Introduced in:_ [60ff4c1](https://github.com/leptoskit/leptoskit/commit/60ff4c128909e3837a936d04e92b4037c58cb14e)
- **Validate the dev address before watching the workspace.** `leptoskit dev` rejects an invalid web serve address before inspecting Cargo metadata, starting a database, or watching files. Watching still begins before the initial build, so edits made during that build trigger a rebuild. _Owner:_ Ifiok Jr. · _Introduced in:_ [7237130](https://github.com/leptoskit/leptoskit/commit/723713098b125431f6e762cbf4b6f3ebe5506a23)
- **Flush the development watcher announcement so processes reading piped logs receive it before the initial build finishes.** _Owner:_ Ifiok Jr. · _Introduced in:_ [94cd633](https://github.com/leptoskit/leptoskit/commit/94cd633355dffb56cc7249e234ea5fc994c9ba20)
- **`lk dev` keeps edits made while it starts.** `leptoskit dev` now starts watching your project before its first build rather than after every platform has started. An edit saved while that first build ran, which takes minutes on a fresh checkout, used to go unseen: the server came up on the code from before the edit and stayed on it until you saved again. The edit now waits in the watcher's queue, and `dev` rebuilds and restarts the server as soon as the platforms are up. The `watching … for changes` line still prints once everything has started. _Owner:_ Ifiok Jr. · _Introduced in:_ [fa3d29f](https://github.com/leptoskit/leptoskit/commit/fa3d29f8e8c837b19f92f1e9a95f699803543767)
- **`leptoskit dev` says it is watching as soon as the watcher is running, before the first build.** That build can take minutes, and the message used to wait until the platforms had started, so an edit during it looked ignored. _Owner:_ Ifiok Jr. · _Introduced in:_ [f5104c6](https://github.com/leptoskit/leptoskit/commit/f5104c6f382ac2e42b512fb160280ee8d375cb9e)
- **Connected iPhones show up in `device list`.** `leptoskit device list` asked `devicectl` for `--json` and parsed a `devices` array at the top of that output. devicectl does not print JSON that way. It writes a file for `--json-output`, and the phones are under `result.devices`. The id an install, a launch, and an XCUITest session need is the hardware UDID, not the Core Device identifier. A plugged-in iPhone never appeared. Watches and simulators in the same file stay out of the iPhone list. _Owner:_ Ifiok Jr. · _Introduced in:_ [2ce47e0](https://github.com/leptoskit/leptoskit/commit/2ce47e01eedb1af4bc45cb05bca2d01b10c886ad)
- **Image builds retry their tool download.** The Dockerfile that `leptoskit deploy` generates for web apps downloads binaryen's `wasm-opt` from GitHub. It now retries that download up to five times and unpacks it only once it is complete, so a brief GitHub outage no longer fails the image build. _Owner:_ Ifiok Jr. · _Introduced in:_ [3cd7307](https://github.com/leptoskit/leptoskit/commit/3cd7307fbdba2e00927bf057a258097652aefca7)

#### File-route navigators, deep links, and system Back

_Owner:_ Ifiok Jr. · _Introduced in:_ [bbc358b](https://github.com/leptoskit/leptoskit/commit/bbc358babf47a0c482e74527c7011408f620dfe8)

Layouts can export `navigator()` returning a typed `Stack`, `Tabs`, or `Drawer`. Folder groups like `(tabs)/` stay URL-less; the navigator return value chooses the chrome. Tab and drawer roots are `impl Into<Route>`, not string names. Stacks can present matched routes as a sheet or modal above the previous screen, and optionally offer an edge hit target for Back.

`Navigation::open_url` / `open_url_with` turn incoming URLs into typed routes (custom schemes and `http(s)`). `apply_activations` drains a host deep-link queue into the router. `bind_system_back` and `leptoskit_system_back` let Android system Back pop router history when the host offers it. Generated Android activities call into that path before finishing.

[`examples/file_routes`](https://github.com/leptoskit/leptoskit/tree/main/examples/file_routes) shows Stack + Tabs + Drawer, deep links, and Back. True UIKit interactive-pop and AppKit history swipe still need host presentation; the guide names those gaps.

#### Repair the failures blocking the push gate

_Owner:_ Ifiok Jr. · _Introduced in:_ [81054a0](https://github.com/leptoskit/leptoskit/commit/81054a09cf174adcc8509131104b170f42054055)

Test-only fixes: a missing semicolon in a `leptoskit_cloud_runtime` test that clippy rejects with `semicolon_if_nothing_returned`, the `pushes_paused` field in a `leptoskit_cli` cloud test's notifications fixture, the `lk cloud notifications` help and output snapshots that `leptoskit_cli` was missing or held stale, and the migration count a `leptoskit_notifications` restart test pins, which the push acceptance ledger's migration left behind.

The demo's generation fixture also builds its git repository outside the one a `pre-push` hook names, so the hook's own tests no longer stage into the repository being pushed. Its pinned workspace schema snapshot is refreshed for the notification tenant, lease, and push-acceptance tables that landed after it was pinned.

- **Fix wearable wire codecs, watch plugins, and host builds.** Fixes cross-platform wire mismatches in Health (`ABSENT_LENGTH`, `ANDROID_TYPES` / `HARMONY_TYPES` bitmask preserving `HRV_RMSSD`, and exercise `Activity` codes 8..14), HarmonyOS Pedometer (`bounded_reply` 36-byte framing) and Battery charge state mapping, and BLE 16-bit/128-bit UUID normalization. Fixes Apple/Android runtime bugs across `ble`, `watch-sync`, `wearable`, and watchOS `haptics`, adds `Reading::Ecg` for `SensorMask::ECG`, fixes `RotaryState` wrap-around detent math, adds watchOS 10 support to `LeptosKitBridge` and wearable SwiftPM manifests, enables native plugin resolution and unified output paths for `wearos`, `watchos`, and `harmonyos` in `leptoskit_cli`, and adds the wearable crates to CI unit tests. _Owner:_ Ifiok Jr. · _Introduced in:_ [21a5f55](https://github.com/leptoskit/leptoskit/commit/21a5f55fea25e9c1f043da51aae77bcc6296aeb3)
- **Builder lint keeps the SSR shell; apps pin Rust.** `markup = "builder"` no longer reports ordinary HTML tags in the SSR document `shell` (`<html>`, `<head>`, …)—only leptoskit component trees. The agent skill and linting guide say the same. Deploy generation names `--sdk-path` when a path dependency sits outside the workspace. (New apps already pin `rust-toolchain.toml` from the matching scaffold work.) _Owner:_ Ifiok Jr. · _Introduced in:_ [d6f5489](https://github.com/leptoskit/leptoskit/commit/d6f5489be53cd5b202bfd0f8476839a9ce508d41) · _Last updated in:_ [442b38b](https://github.com/leptoskit/leptoskit/commit/442b38b81352231ebb9810f646247bd548ed353c)

#### Generated sign-in effects run in native apps

_Owner:_ Ifiok Jr. · _Introduced in:_ [1bd0618](https://github.com/leptoskit/leptoskit/commit/1bd0618c284e4dff885524d3d6a23db9cfc668f8)

`leptoskit new --auth` and `leptoskit generate auth` now use isomorphic effects for the sign-in screen's mount step, the provider-notice step, and the post-sign-in navigation. `Effect::new` is compiled out in a native guest (the generated app does not enable leptos's `effects` feature), so the screen could stay on "Checking whether you're signed in…" forever. The effects still skip server rendering, so the first server HTML matches the browser's first render.

The generated notes journey is now exercised end to end in the real interpreter: the optimized guest loads and refreshes notes over the HTTP plugin, validates the draft, shows pending/saved/failed saves, signs in through the development mailbox, switches accounts without showing the previous account's rows, and rejects late list and save replies after their screen is disposed.

The generated `add_note` operation trims the body before it checks the note's length, so a direct API caller cannot store a note that is only whitespace even though the screen already trims its draft.

The sign-in countdown is tested apart from its executor: a host timer counts the whole wait down, a missing or rejected timer leaves the server's message and a usable manual retry without claiming time passed, a newer refusal or starting over stops an older countdown, and disposing the screen stops it without touching a disposed signal.

`leptoskit generate auth` on an existing database app still adds the screens and asks the app to call `auth::provide()` and put `auth::screens().gate(…)` in front of its own home screen. Rewriting an app's root automatically is not safe, so that manual step stays open; the journey covers the fully scaffolded `--auth` app, which is already wired.

- **Wait for Caddy to finish executing under its dedicated account before checking graph release readiness.** Recheck process identity and socket ownership after probing so systemd startup and process replacement cannot produce premature restoration or a false ready result. _Owner:_ Ifiok Jr. · _Introduced in:_ [7c788ae](https://github.com/leptoskit/leptoskit/commit/7c788ae0d8522dc8f0576961ef4bcb9121603f70)
- **Keep graph placement separate from application frontends.** Graph artifact placement uses `[infra.frontends.<name>]`. Application build frontends keep their existing top-level `[frontends.<name>]` contract. This avoids colliding configuration types when named application targets and deployment graphs are used in the same toolkit. Graph unit identifiers such as `frontends.docs` and all ownership, route and recovery checks are unchanged. _Owner:_ Ifiok Jr. · _Introduced in:_ [7c788ae](https://github.com/leptoskit/leptoskit/commit/7c788ae0d8522dc8f0576961ef4bcb9121603f70)
- **Allow protected systemd drop-in masks that point exactly to /dev/null during graph release and recovery.** Active overrides and masks with unsafe ownership or parents remain rejected. _Owner:_ Ifiok Jr. · _Introduced in:_ [7c788ae](https://github.com/leptoskit/leptoskit/commit/7c788ae0d8522dc8f0576961ef4bcb9121603f70)
- **Keep graph verification checks portable.** Graph directory creation only mutates its builder when applying Unix permissions. On other hosts, descriptor capability methods retain their instance API and continue refusing operations that require Unix metadata. Windows Clippy no longer rejects these unsupported-platform branches. _Owner:_ Ifiok Jr. · _Introduced in:_ [7c788ae](https://github.com/leptoskit/leptoskit/commit/7c788ae0d8522dc8f0576961ef4bcb9121603f70)
- **Give the HarmonyOS host's line-height values named, explicit types so generated applications compile with the public API 12 ArkTS compiler.** _Owner:_ Ifiok Jr. · _Introduced in:_ [7237130](https://github.com/leptoskit/leptoskit/commit/723713098b125431f6e762cbf4b6f3ebe5506a23)
- **The generated iOS `App.swift` now imports `LeptosKitNotifications` under its `canImport` guard: the app delegate referenced the plugin's broker type whenever the notifications plugin was selected, but no module was imported, so the generated project failed to compile with the plugin present.** A macOS test now generates the project both with and without the plugin, builds the plugin's Swift package for the iOS simulator, and typechecks the generated app sources — template, registrant, and the bridge's mobile sources — against the real modules, so the conditional import is proven both ways instead of textually. _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)
- **`lk lint` passes on a fresh clone.** `leptoskit lint` no longer fails on a fresh clone of an app with native platforms. App-owned runners include a registrant under `.leptoskit/<platform>/`, which Git ignores and only native builds wrote, so clippy stopped on the missing file until you ran a native build. Before clippy starts, `lk lint` now writes missing registrants for the enabled hosts that compile on the current machine and enables the Cargo features their plugins need. Existing registrants stay untouched until the next native build. If a host cannot be prepared, lint reports a warning and clippy names what it cannot compile. _Owner:_ Ifiok Jr. · _Introduced in:_ [a7d277c](https://github.com/leptoskit/leptoskit/commit/a7d277c8d7e0b13b2ad4eb084b8aac83b312577c)

#### Linux pointer gestures from `leptoskit device` no longer pause for about fifteen seconds when the pointer is already on the point.

`xdotool mousemove --sync` waits for a motion event that never arrives in that case, so a tap followed by a long press or a pan at the same point stalled. The move is sent without that wait.

_Owner:_ Ifiok Jr. · _Introduced in:_ [86bb058](https://github.com/leptoskit/leptoskit/commit/86bb058f3c5c948f444fe75099ec639b02e7933d)

Opening a Linux app also waits until one of its windows is visible, the process exits, or two seconds pass. Gestures used to be injected the moment the process was spawned, which is before the window is mapped, so the tap and the long press landed on the desktop.

Opening an Android app waits until that package is the focused window, then a little longer, or two seconds pass. `am start` was returning while the launcher was still focused, and the focus line itself updates before the activity accepts a tap, so the sanity tap and long press never reached the activity.

`leptoskit device boot` of an AVD that is already running fails immediately and names the emulator serial, instead of starting a second emulator and waiting out the boot deadline. If the emulator process exits before the device is online, the boot fails then, instead of blocking on `adb wait-for-device`.

- **Cloud tools on `lk mcp` take no `url`.** The `lk mcp` cloud tools (`cloud_estimate`, `cloud_deploy`, `cloud_status`, `cloud_logs`, `cloud_secrets_list`) take no `url` argument. It came from the model or the client, and with `LEPTOSKIT_CLOUD_TOKEN` set one tool call could send that token to any origin the argument named. The tools now choose the cloud as `lk cloud` does without `--url`: `LEPTOSKIT_CLOUD_URL`, then `[cloud].url` in `leptoskit.toml`, then the hosted cloud. The token still goes only to the hosted cloud or to the origin `LEPTOSKIT_CLOUD_URL` names; when `leptoskit.toml` alone points somewhere else, the tool refuses and says to set `LEPTOSKIT_CLOUD_URL`. _Owner:_ Ifiok Jr. · _Introduced in:_ [637cdc4](https://github.com/leptoskit/leptoskit/commit/637cdc48582dd612b257fb3728c590854bc56a86)
- **More ergonomic multi-frontend scaffold.** `--frontends` alone scaffolds landing, app, and admin. Ports are stable per kind. The product app under `apps/app` uses the workspace name as its Cargo package. Frontend names trim whitespace and accept ASCII case; `ApplicationKind` gains `FromStr`, `Display`, and helpers used by the scaffold. Each frontend app uses the same file-router defaults as a normal `leptoskit new`. _Owner:_ Ifiok Jr. · _Introduced in:_ [fa8c81a](https://github.com/leptoskit/leptoskit/commit/fa8c81afb142de65ec3b1f0908c787ac164cb306)
- **Native capability parity: badges and shortcut hosts.** Browser pages can set and clear app-icon badges through the Badging API (`web::set` / `web::clear`), and `web::get` returns the last count this adapter published. Generated iOS, Android, and macOS hosts now call `noteLaunch` when the OS activates an app shortcut: Android dynamic shortcuts carry a unique intent per id, the macOS host builds the dock menu from `DockShortcutStore`, and iOS notes home-screen quick actions. Docs match the real backends for menus, badges, and shortcuts. _Owner:_ Ifiok Jr. · _Introduced in:_ [0c66c58](https://github.com/leptoskit/leptoskit/commit/0c66c58142338c3399198783a442bc777f04476c)
- **Android native hosts link AAudio even when rustc omits it from its reported libraries.** HarmonyOS uses named border and accessibility interfaces so ArkTS can compile the host contract. _Owner:_ Ifiok Jr. · _Introduced in:_ [6919585](https://github.com/leptoskit/leptoskit/commit/691958516d206c0fc9d4f3c879e8fd3b8982d4fa)

#### Android presents Modal as a dialog and Popover as a popup

_Owner:_ Ifiok Jr. · _Introduced in:_ [f740aa1](https://github.com/leptoskit/leptoskit/commit/f740aa1e5c264305477d17f75c66296993833da2)

On Android a `Modal` is now a dialog in a window of its own: it dims the screen, keeps touches, the keyboard and TalkBack to itself, and is as large as what the modal holds. A `Popover` is a popup window placed beside its anchor where `placement` says. The Back gesture, Escape and a touch outside ask to dismiss. A dialog stays until your `open` signal closes it. The Back gesture closes a popup window before your app hears of it, so a popover your app keeps open is shown again. `initial_focus` moves focus to the named element when the overlay opens. This holds for generated Android apps (the Kotlin renderer) and for Rust hosts on `AndroidBackend`.

What differs from the web: the modal's own background is not drawn, because the dialog dims the screen, and a dialog needs an Activity behind the view the tree is mounted in.

`WidgetHost` gains `overlayShown`, `overlayDialog` and `overlayPopup`. Generated Android projects ship one more Kotlin file, `NativeOverlayView.kt`.

This also restores two things the Android host needed to compile: an import in `WidgetHost.kt` and the `Dismiss` case in the probe.

#### iOS presents Modal as a sheet and Popover as a popover

_Owner:_ Ifiok Jr. · _Introduced in:_ [9055533](https://github.com/leptoskit/leptoskit/commit/905553347de313983dc7fadd12b419d286dabe38)

On iOS a `Modal` is now a form sheet: it dims the screen, keeps touches and VoiceOver to itself, and gives focus back when it closes. A `Popover` is a system popover pointing at its anchor, on an iPhone as well as an iPad. A swipe down on the sheet, a press outside the popover, Escape on a hardware keyboard and the VoiceOver escape gesture all ask to dismiss, and the overlay stays until your `open` signal closes it. `initial_focus` raises the keyboard for a field and moves the VoiceOver cursor for anything else. This holds for generated iOS apps (the Swift renderer) and for Rust hosts on `UIKitBackend`.

What differs from the web: a sheet is as large as what the modal holds on an iPad and as wide as the screen on an iPhone, and the modal's own background is not drawn; and a popover is centred on its anchor, so `placement` picks above or below only.

`UIKitBackend` gains `overlay_shown`, `overlay_controller`, `perform_dismiss` and `present_overlays`. A controller that mounts the tree itself calls `present_overlays` from `viewDidAppear:`; `UIKitHost` does so already. Generated iOS projects ship one more Swift file, `MobileOverlayView.swift`.

#### macOS shows Modal as a sheet and Popover as NSPopover

_Owner:_ Ifiok Jr. · _Introduced in:_ [4cba9d4](https://github.com/leptoskit/leptoskit/commit/4cba9d4cc9bafbfdd824dfca3ce310b3ad3cda0d)

On macOS a `Modal` is now a sheet on the app's window: it dims the window, keeps clicks and the keyboard to itself, and gives focus back when it closes. A `Popover` is an `NSPopover` pointing at its anchor. Escape asks to dismiss either one, and a press outside asks to dismiss a popover. `initial_focus` moves focus into the overlay when it opens. This holds for generated macOS apps (the Swift renderer) and for Rust hosts on `AppKitBackend`.

What differs from the web: a sheet is as large as what the modal holds and the modal's own background is not drawn; a sheet has no outside to press; and a popover is centred on its anchor, so `placement` picks above or below only.

`AppKitBackend` gains `overlay_shown`, `overlay_window` and `perform_dismiss`, and `open_window` shows overlays that were already open. Generated macOS projects ship one more Swift file, `MacOverlayView.swift`.

#### Smaller web Wasm, lazy listeners & precompressed serving

_Owner:_ Ifiok Jr. · _Introduced in:_ [4001dbb](https://github.com/leptoskit/leptoskit/commit/4001dbb8ed91ecbdcb3caffdc86690b17607ba9c)

- Remove `sha2` from `leptoskit_native`'s `web` feature in favor of a deterministic 64-bit `ScopeHasher`, emit non-default base CSS style deltas directly with `styles::delta_for`, fuse tree pre-scans into one `scan_tree` pass, attach DOM event listeners lazily via `Either` only when handled, and make `NodePath` zero-sized without `devtools`.
- Inject `<link rel="modulepreload">` and `<link rel="preload" as="fetch" type="application/wasm" crossorigin>` hints into built `index.html` files and enable `--strip-producers --low-memory-unused` in `leptoskit_cli`'s `wasm-opt` pipeline.
- Negotiate precompressed `.br`/`.gz` `index.html` in `leptoskit_axum`'s `SpaFallback` and serve precompressed static site/docs assets with `Vary: accept-encoding` in `leptoskit_app`.

#### Binary LKW1 guest wire, smaller guests, and fast Tailwind

_Owner:_ Ifiok Jr. · _Introduced in:_ [42c3217](https://github.com/leptoskit/leptoskit/commit/42c32178289561103166d9067be713926265efb2)

- Stream native guest trees to the host with a compact binary wire protocol (`LKW1`) into a reused staging arena instead of per-node JSON serialization, while keeping single-pass JSON decoding for legacy and JS guests.
- Strip dead browser `wasm-bindgen` custom sections and unreachable exports before `wasm-opt -Oz` in `leptoskit_cli`'s guest optimizer, reducing native guest `.wasm` sizes by ~45–52% raw (~29–35% brotli).
- Defer `For`/`Show` region evaluation until expansion (avoiding duplicate subtree construction) and replace heavy sorting networks in `Tailwind::parse` with a compact insertion sort and bitflag fast paths.

#### Plugin options the app would crash on stop the build instead

_Owner:_ Ifiok Jr. · _Introduced in:_ [da4763e](https://github.com/leptoskit/leptoskit/commit/da4763e983262b2144684984245a2bf27b5a61f5)

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

- **Always run the repository's full pre-push gate, including pushes whose commits change no files.** The gate no longer depends on the hook runner selecting at least one changed file. _Owner:_ Ifiok Jr. · _Introduced in:_ [2c909d1](https://github.com/leptoskit/leptoskit/commit/2c909d10b5be31f521b4ea361a62a4f685538894)

#### Publish cloud CLI crates and prebuilt release binaries

_Owner:_ Ifiok Jr. · _Introduced in:_ [e47b3de](https://github.com/leptoskit/leptoskit/commit/e47b3de2192792651b71741c9094b14cf49492be)

- Registers `leptoskit_cloud_platform` (`leptoskit-cloud-platform`), `leptoskit_cloud_worker` (`leptoskit-cloud-worker`), and `leptoskit_cloud_app` (`leptoskit-cloud-control`) in `monochange.toml` and removes `publish = false` so all three crates version and publish with the main release group.
- Adds `[package.metadata.binstall]` to all three crates and extends `.github/workflows/publish.yml` to build, archive, upload, and provenance-attest prebuilt binaries for all release targets alongside `leptoskit` and `lk`.
- Bundles the console's two Latin font subsets inside `crates/leptoskit_cloud_app/assets/fonts/` so `leptoskit_cloud_app` packages cleanly for crates.io.
- Updates `lk cloud <name>` missing-binary hints to recommend `cargo binstall` and `cargo install`.

- **Harden native-wave plugin correctness.** Live Activities on iOS cancel push-token waits without freezing the per-id busy lock, replace duplicate ActivityKit ids on start, and reject duplicate content keys. Picture-in-picture leases best-effort exit system PiP and observe user exit; `request_overlay` docs match launch-time status. Home widgets stop reporting unrelated pins for unknown kinds, commit preferences before success, and reject dangerous URL schemes. Device-policy and Live Activities stubs validate payloads before Unavailable. Wear/watchOS hosts return errors instead of panicking on viewport kind mismatch. New Android hosts declare `supportsPictureInPicture` on MainActivity. _Owner:_ Ifiok Jr. · _Introduced in:_ [879db6b](https://github.com/leptoskit/leptoskit/commit/879db6b2334d0c5bd8519f9a26f7a3eb92fca7f0)

#### Add the recorder plugin: continuous microphone segments

_Owner:_ Ifiok Jr. · _Introduced in:_ [0036fe4](https://github.com/leptoskit/leptoskit/commit/0036fe4b9d010f4c568d160e88d4550b2aa5e38a)

Add a `leptoskit_plugin_recorder` crate with a typed binary guest client, Swift and Kotlin handlers, and automatic native registration under the runtime name `recorder`. It records the microphone continuously in the background as timed audio segments: AAC-LC, mono, `.m4a`, rotated at each boundary with no gap between segments, for apps that upload and transcribe each segment themselves.

`start` takes a directory inside the app's own data or cache directories and a segment length (default 300 s, clamped 5–3600 s), sample rate (default 16 kHz), AAC bitrate (default 24 kbps) and activity threshold (default −45 dBFS). `next_segment` is a cancellable wait that returns each finished segment with its path, start time, duration, size, peak level and `active_ms` — the milliseconds of 30 ms frames above the threshold — so the app can skip uploading quiet segments without decoding them. The plugin never deletes a delivered segment and never decides whether one is silent; the app owns both choices. `status` reports idle, recording, interrupted (a call or Siri), silenced (Android: another app holds the microphone) or failed with a reason, plus the open segment's start time.

A small append-only journal beside the segments survives a crash: the next `start` (or a `recover` call) delivers leftover finalised files as `ended_by = recovered` segments and deletes the truncated files that cannot be read, reporting each deletion once. iOS records through an `AVAudioEngine` input tap into an AAC `AVAudioFile`, configures `.playAndRecord` with `.mixWithOthers`, `.allowBluetooth` and `.defaultToSpeaker` so the owner's music and calls keep working, closes the segment on an interruption and resumes when the system allows it, and rebuilds the tap on route changes and after a media services reset; recording can only start in the foreground, and the `audio` background mode keeps it running. Android records `AudioRecord` PCM into a `MediaCodec` AAC encoder and `MediaMuxer` on its own thread, reports `silenced` through the audio recording callback, and requires the `background` plugin's microphone-typed foreground service for background recording — which Android 14 cannot start from `BOOT_COMPLETED`, so the owner must open the app once after a reboot. The `background` plugin now exposes the running service's type mask so `start` can fail with a clear error instead of recording into a process the system will kill.

- **Release web builds minify the JavaScript glue, its snippets, and the chunk loader with esbuild, next to `wasm-opt`.** Export names and `import.meta` stay, so the page's imports keep working. `esbuild` comes with the development environment; a machine without it can set `bundle.wasm_opt = false` to skip both. _Owner:_ Ifiok Jr. · _Introduced in:_ [f5104c6](https://github.com/leptoskit/leptoskit/commit/f5104c6f382ac2e42b512fb160280ee8d375cb9e)
- **Configured RPC endpoints now work for clients alongside the server configuration.** The clients previously called the compile-time defaults: the app's own client posted every call to `/rpc/<op>`, the browser's server-sent events fallback opened `/live/events` and posted frames to `/live/frames`, and the exported spec described every operation at `/rpc/<op>` however the app was configured. The spec export now carries both mounts (`rpc_path` and `live_path`, spec version 4), composes every rpc operation's route from the configured prefix — a service's spec under the service's own prefix — and the foreign and generated SDK clients call those routes and open that socket, so `leptoskit rpc generate` output follows the configuration. The app's own client follows it too: `leptoskit dev` and `leptoskit build` compile the browser and native clients with `LEPTOSKIT_RPC_PATH` and `LEPTOSKIT_RPC_LIVE_PATH` from `leptoskit.toml`, and `ClientConfig::rpc_path`/`live_path` name the mounts by hand for builds that don't go through the CLI. A service's operations keep the default mounts under the service's prefix: `[rpc].path` moves the app's own. Defaults are unchanged (`/rpc`, `/live`, legacy `/api`), as are scope lookup, auth headers, service credentials, and service origin isolation. _Owner:_ Ifiok Jr. · _Introduced in:_ [8bd1891](https://github.com/leptoskit/leptoskit/commit/8bd1891b8d987c9abe19128343e7107d706ecd88)

#### `[rpc].path` and `[rpc].live_path` decide where routes serve

_Owner:_ Ifiok Jr. · _Introduced in:_ [8bd1891](https://github.com/leptoskit/leptoskit/commit/8bd1891b8d987c9abe19128343e7107d706ecd88)

`RpcConfig.path`, `live_path`, and `transports` were accepted and validated but no server read them: every app served `/rpc` and `/live` whatever the config said. Now they decide the mounts, once, at the app's entrypoint.

The generated app reads `[rpc]` from `leptoskit.toml` and applies it: operation routes serve at `path` (with the JSON routes kept on the legacy `/api` prefix while JSON is a transport), and `Server::rpc_config` carries the resolved live mount through `Server::live`. `Live` takes the same config, so a router built by hand mounts the socket at `live_path` — `/live` while the config stays at its defaults — with its server-sent events and frame posts right next to it. `path` without `live_path` moves the socket to `{path}/live`; an explicit `live_path` mounts it anywhere.

`leptoskit rpc spec` exports the configured live mount, so regenerated clients open their socket where the app serves it. Generated HTTP calls still name the compile-time `/rpc/<op>` route: keep the default `path` when generated clients call the app directly, and use a custom one behind a proxy or gateway that remounts the prefix.

#### Preserve null, empty collections and omitted arguments in generated SDK requests.

_Owner:_ Ifiok Jr. · _Introduced in:_ [51224bf](https://github.com/leptoskit/leptoskit/commit/51224bf974470ffc7ad6154c57e515f4f83556d3)

Query and form encoders now match the server's serde_qs 1.1 protocol: explicit null is a bare key and an empty collection is `key=`. JSON and live encoders keep explicit null, and required nullable arguments in Go and Rust keep their keys. Python optional arguments use the exported `UNSET` for omission and `None` for explicit null. Generated Rust and CLI packages need `serde_qs` 1.1.3; update an existing package's manifest when regenerating its code.

Clients, OpenAPI, AsyncAPI and compatibility checks now follow each argument's `required` flag rather than inferring it from nullability. Specs from `#[api]` functions mark `Option` arguments optional, because the server reads a missing key as `None`. The spec format is now version 4; older specs still load, and their nullable arguments stay optional. A spec written by this version needs an equally new CLI to generate clients.

In regenerated Dart clients, `Option` arguments become optional `Presence<T?>` parameters instead of required `T?` ones. Omit them, or pass `Presence.present(value)`.

Strict diagnostics report a Go or Rust optional nullable argument only when its schema declares a default, and no longer report a Dart query/form limitation that the encoder now handles. Mandatory SDK acceptance decodes generated HTTP requests with the server codec.

- **New apps write builder lint to match the starter screens.** `leptoskit new` now sets `[lint] markup = "builder"` with `markup_style` and `component_attribute` denied, so a fresh scaffold's builder home screen is clean under `leptoskit lint --deny-warnings`. The skill, creating guide, linting guide, and small-bundles guide match: omitted `render` still defaults to csr in the schema, but new projects keep the explicit ssr they already write. _Owner:_ Ifiok Jr. · _Introduced in:_ [5d86470](https://github.com/leptoskit/leptoskit/commit/5d86470b55f2b640a68a13c6097503dc23cfea84)
- **Generated servers find `LeptoskitConfig` again.** The database and auth server templates read `[rpc]` through `leptoskit::LeptoskitConfig`, which `leptoskit` exports only from `leptoskit::config` and its prelude, so a newly generated project didn't compile. They now name `leptoskit::config::LeptoskitConfig`. _Owner:_ Ifiok Jr. · _Introduced in:_ [ef5e7c5](https://github.com/leptoskit/leptoskit/commit/ef5e7c58952db616d3de126653148ae14add82a0)
- **Scaffold uses shared Health; skill documents web-only trim.** New apps re-export `Health` from the shared crate in the app library so `leptoskit lint --deny-warnings` no longer fails on an unused `*_shared` dependency before any RPC types are used. The coding-agent skill says there is no frontend-only `new` flag: after scaffolding, set `platforms = ["web"]` and optionally trim unused host workspace members. _Owner:_ Ifiok Jr. · _Introduced in:_ [fb5e435](https://github.com/leptoskit/leptoskit/commit/fb5e4353ff93ff3cfed5eab0a9d01997609c2bad)
- **New apps pin rustc; skill matches the scaffold.** `leptoskit new` now writes a `rust-toolchain.toml` that pins the SDK channel and the `wasm32-unknown-unknown` target, so `edition = "2024"` projects build even when rustup's default toolchain is older. The coding-agent skill no longer claims the default starter uses regenerating file routes, tells agents to keep the HTML `shell` in leptos `view!` (with a line-local lint allow) when enforcing builders, and shows `#[component]` helpers returning `Element` for `.child(…)` composition. The creating guide notes the SSR default and the toolchain pin. _Owner:_ Ifiok Jr. · _Introduced in:_ [53ca740](https://github.com/leptoskit/leptoskit/commit/53ca7405fac9fb680cd7468429cf09d9b004e76e) · _Last updated in:_ [442b38b](https://github.com/leptoskit/leptoskit/commit/442b38b81352231ebb9810f646247bd548ed353c)
- **Web-only lint skips unused hosts; drop an import.** `leptoskit lint`'s clippy pass excludes workspace host crates whose platforms are not listed in `app.platforms`, so setting `platforms = ["web"]` is enough for a green deny-warnings run without GTK or native prepare. `--db` / `--auth` scaffolds no longer put an unused `use leptos::prelude::*` in `App()`. The skill documents the platforms-based clippy skip. _Owner:_ Ifiok Jr. · _Introduced in:_ [44be954](https://github.com/leptoskit/leptoskit/commit/44be9542fedde87deb6c823b8c7f3f5ad44cc49d) · _Last updated in:_ [442b38b](https://github.com/leptoskit/leptoskit/commit/442b38b81352231ebb9810f646247bd548ed353c)

#### Native scenes load models on the first launch

_Owner:_ Ifiok Jr. · _Introduced in:_ [7237130](https://github.com/leptoskit/leptoskit/commit/723713098b125431f6e762cbf4b6f3ebe5506a23)

Android, iOS and macOS install the scene asset reader before starting the guest. Models queued before the first native scene view exists now load from the app bundle on a cold launch.

Generated Axum route lists also return their owned paths directly, keeping generated apps free of redundant conversion warnings.

#### Service workers and stronger extension messaging

_Owner:_ Ifiok Jr. · _Introduced in:_ [363ebad](https://github.com/leptoskit/leptoskit/commit/363ebadd2effe51d9b954c31ab0b6bcbfdc46ab8)

Web apps can enable the `service-worker` feature to register a worker script, exchange messages, describe caching strategies, open Cache Storage, and manage Web Push subscriptions. Calls return `Unavailable` outside a secure browser context.

Browser extensions gain long-lived `runtime::connect` ports (with `Background::on_connect`), storage `clear` / `keys` / `get_bytes_in_use`, and permissions `get_all` / `on_added` / `on_removed`. The generated background worker keeps ports alive so the wasm side can post and disconnect by id.

- **SSR apps now register every file route with `leptos_axum`, and the server render reads the request URL instead of always opening on Home.** _Owner:_ Ifiok Jr. · _Introduced in:_ [363c36e](https://github.com/leptoskit/leptoskit/commit/363c36ed87c461f093d2a4c53a090ef8606f9228)

#### Server-rendered queries hydrate without a mismatch

_Owner:_ Ifiok Jr. · _Introduced in:_ [c451911](https://github.com/leptoskit/leptoskit/commit/c451911f920e3f81df7154ede1e9cc0bff52ff10)

A page that rendered a query on the server now hydrates as the same page in the browser. Before, only a query's data was carried, so a query that failed on the server started from "loading" in the browser, leptos reported a hydration mismatch, and the page stopped updating.

- `QueryScope::with_ssr_errors` (and `with_error_codec`) streams a server render's error alongside the data, so the browser shows the same error first. RPC queries and paginated queries opt in, and `FnError` is now `Serialize` and `Deserialize`.
- A query seeded from the server doesn't fetch while the page hydrates. Its mount refetch — stale data in the background, or a retry after an error — starts once hydration has finished, so the first render matches the server's HTML.
- On the server, reading a query that is still loading inside `<Suspense>` holds the boundary until it settles, as a leptos resource does, so the server renders the outcome it streams.
- Streamed payloads escape `<`, `>`, `&`, and U+2028/U+2029, so data and errors can't close the `<script>` they're embedded in.

Apps using `render = "ssr"` should read queries inside `<Suspense>`; outside a boundary the server renders whatever the query holds at that moment.

New apps put their web adapter inside that boundary, including the database and sign-in scaffolds. Existing portable apps should wrap the closure that calls `leptoskit::web::render` in the same way.

Generated browser entrypoints configure the RPC and auth clients inside the mount callback, after leptos has set up its executor and owner, so the live socket starts on the first page load. A query observer reads `enabled` without subscribing while it binds or runs its refetch interval, so an `enabled` closure that reads another query no longer logs a reactive-tracking warning on every page.

#### Accept managed registries with explicit ports

_Owner:_ Ifiok Jr. · _Introduced in:_ [b47bc2e](https://github.com/leptoskit/leptoskit/commit/b47bc2e41920c89e7382665a7fccab1c9508de9d)

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
