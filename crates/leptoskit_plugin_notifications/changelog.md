# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-28)

### Features

- **Android FCM registration on the plugin side.** `REMOTE_TOKEN` and `REMOTE_REGISTER` work on Android hosts, not iOS alone: a process-wide broker — the same shape as the Swift one — keeps the latest answer, fed by the registration task's own reply and by the messaging service's `onNewToken`, and the Android `REMOTE_REGISTER` asks `FirebaseMessaging` for its token reflectively (the generated app carries `firebase-messaging` only when push is opted into), coalesces OS requests at one, answers `Unconfirmed` on silence, never succeeds after close, and keeps the token out of every log and description. The stable failure codes gain `firebase-not-configured` — the Firebase classes are absent, or the app has no `google-services.json` values — beside `os-refused` and `token-shape`; tokens validate as 1–4096 bytes of printable ASCII and the wire gains state 4 for them. The Android instrumentation suite covers the bounded wait, unconfirmed silence, token-shape refusal, close-while-waiting, and the one-ask convergence with an injected registration boundary. _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)

#### Development and store builds share one service

_Owner:_ Ifiok Jr. · _Introduced in:_ [8980de9](https://github.com/leptoskit/leptoskit/commit/8980de9af6db3ed62cd57c2e36e8c461d5a0cea5)

Apple runs two push servers, and a device token is known to one of them. Until now the service used one for every device, so a build run from Xcode and a TestFlight build could not enroll with the same service. Now each iOS and macOS device says which server its build is signed for, and the service pushes it through that one.

- The iOS and macOS hosts answer a new plugin method, `APS_ENVIRONMENT` (method 11): the simulator answers development, a build with an embedded provisioning profile answers what the profile's `aps-environment` entitlement says, and an iOS build without a profile, which came through TestFlight or the App Store, answers production. Android does not answer it.
- `native::enroll` and the launch `reconcile` send the answer with the device's other details, as `DeviceDetails::apns_environment`. Your app writes no code for it, and a change reaches the service at the next launch.
- The service stores it per endpoint and picks Apple's server from it. `LEPTOSKIT_NOTIFICATIONS_APNS_ENVIRONMENT` is now the default, for a device that reported nothing: an older app version, a Mac build without a profile, or an imported token.
- `import_push_endpoints` and a direct `register_push_endpoint` take the same field, so imported APNs tokens can name their environment.
- `person_endpoints` and `push_endpoints` show it under `device`. Conditions do not read it, and it is dropped for an FCM or Web Push endpoint.

The `.p8` key, the team id, and the topic are still one of each, so both builds need the same bundle id and a key that may use both servers.

This release has one migration per engine, `endpoint_apns_environment`, which adds a nullable column to `push_endpoints`. Run `leptoskit db --service notifications migrate` before you start the new version.

Breaking for code that builds these by hand: `DeviceDetails` has a new public field, so a struct literal needs `..DeviceDetails::default()`. `ApnsEnvironment` now lives in `push` and is re-exported from `service::config`, where it was. A host that predates method 11 answers "unavailable" for it, and the app enrolls as before.

The Swift code that reads the provisioning profile was not compiled or run on a device for this release.

#### Device details on push endpoints, and targeting by device

_Owner:_ Ifiok Jr. · _Introduced in:_ [d0ea555](https://github.com/leptoskit/leptoskit/commit/d0ea555a8701f661e4bc72a6d697ad6dc59f062a)

Every push enrollment now carries what the device says about itself — its preferred locale, its time zone, and the app's version — and a send's condition can target devices with them.

- **Device details.** `register_push_endpoint` takes an optional `device` (`DeviceDetails { locale, time_zone, app_version }`), defaulting to none, so existing callers keep working. The service stores the locale as a canonical BCP 47 tag (`en_gb` becomes `en-GB`, at most 35 bytes), the time zone as an IANA-shaped name (at most 64 bytes, checked for shape only), and the app version as free text (at most 64 bytes), refusing a bad detail by name. Re-enrolling replaces them; a change to the details alone never rotates the endpoint's credential or disturbs its queued deliveries. Only the endpoint's owner reads them back, as `EndpointSummary::device`. A new `device_details` migration (SQLite and Postgres) adds the three nullable columns.
- **No app code.** The web client sends `navigator.language` and the `Intl` time zone, and the app's version when `EnrollOptions::app_version` names one. The `native` module reads the locale, time zone, and app version through the notifications plugin's new `DEVICE` method on iOS, macOS (Foundation's preferred language, `TimeZone.current`, `CFBundleShortVersionString`), and Android (the default locale and time zone, the package's version name). `native::reconcile` now sends only when the token or the details changed since the enrollment last sent them — `Enrollment` remembers a digest of what it sent in its new `sent` field — so an app update or a language change reaches the service at the next launch and an unchanged launch makes no service call.
- **Device predicates.** Conditions gain `device.platform` (`'apns'`, `'fcm'`, or `'web_push'`), `device.locale`, `device.language` (the locale's first subtag, so `'fr'` matches `fr-CA`), `device.app_version`, and `device.time_zone`, with the operators `user.` predicates have. `device.app_version` compares as a version: everything from the first `-` or `+` is ignored, the rest must be dot-separated numbers compared left to right with missing parts as 0 (`2.10 > 2.9`, `2.1 == 2.1.0`); an ordering is false for a value that isn't a version. Any other key, or a platform that isn't one, is refused at send time and under `validate_only`, naming what there is.
- **Judged per device.** A person is in the audience when the condition holds with no device — where a device predicate answers false, as an absent attribute does — or on at least one of their live endpoints. They get one inbox row, and the push goes only to the endpoints the condition holds for, decided when the fanout writes the batch's deliveries. "Everyone on Android" is `device.platform == 'fcm'`, and it walks the same registry `SendTarget::Everyone` does.

The plugin's `DEVICE` method (9) answers three length-prefixed strings, empty for unknown, as the new `DeviceInfo`; `guest::Notifications::device_info` reads it, and a host that predates it answers `Unavailable`. The Android host's remote-registration snapshot is now encoded little-endian, as the bridge codec reads it.

- **iOS APNs registration, second patch: the answers are now honest about silence and tight about what crosses.** `REMOTE_REGISTER` bounds OS requests at one — later callers join the wait instead of asking again — and answers `Unconfirmed` (state 3) when its wait bound passes with no new answer, so a prior token is never handed back as though it were fresh; the broker keeps the last answer readable through `REMOTE_TOKEN`, and a plugin closed while waiting never answers a token. Registration refusals cross as the contract's stable codes — `os-refused`, `aps-environment` (matched on domain and code), `token-shape` — never the OS's own error text, which can carry device detail and previously clipped to invalid UTF-8. The token is an even count of hex characters (whole bytes, whatever length the OS chose), the revision is a consistent full `u64` with `0` meaning no answer on both constructor and decoder, the Swift broker's counter saturates instead of trapping, and no `Debug` or description of token, state, or snapshot prints its bytes. The browser implementation moved behind a default `browser` feature (as the HTTP plugin's did), so interpreter guests turn it off and their dependency tree stays free of browser imports. _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)
- **macOS joins iOS on the remote-registration wire: the Swift plugin's `REMOTE_TOKEN` and `REMOTE_REGISTER` are advertised on macOS hosts too, where registration asks `NSApplication.registerForRemoteNotifications` instead of `UIApplication`'s. Everything else is the same contract the iOS slice shipped — the process-wide broker the app delegate feeds, one OS request at a time with later callers joining the wait, `Unconfirmed` on silence, the stable refusal codes, lower-case hex tokens of no assumed length, and no token in any log, `Debug`, or description.** No new wire states. _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)

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

### Fixes

- **Android push registration replies in the right byte order.** On Android the plugin wrote a remote registration's revision, state and string length big-endian, while every other reply and the Rust side that reads it are little-endian. An app asking for its push token read a state of `50331648` instead of `3`, or a length of hundreds of megabytes, and failed to decode the reply. The snapshot is now written little-endian like the rest. _Owner:_ Ifiok Jr. · _Introduced in:_ [1cf8d42](https://github.com/leptoskit/leptoskit/commit/1cf8d4299a5e4daa2822eaa90ba30b2d327b0e78)

#### Push-enabled hosts now work the way the notifications guide describes.

_Owner:_ Ifiok Jr. · _Introduced in:_ [7f60843](https://github.com/leptoskit/leptoskit/commit/7f60843eba43202f72117226374f71d7b4f6528d)

**Android.** The scaffold's Gradle script applies Google's services plugin by id (`apply(plugin = plugin)`); the earlier `apply(plugin)` asked Gradle for a script file of that name and failed every push-enabled build. An app scaffolded with the earlier line gets a generation error that names the one-word change. The generated messaging service is declared with the `com.google.firebase.MESSAGING_EVENT` intent filter, so Firebase delivers token refreshes to it instead of to its own default service. The manifest names a default notification channel, `leptoskit.notifications.default`, which the notifications plugin creates at startup, and the notification icon when `[app.icons] notification` generated one. Release builds get R8 keep rules for the Firebase names the plugin looks up by reflection, the plugin finds the task's listener methods on the public `Task` type, and Firebase without configuration answers `firebase-not-configured` even when its exception arrives wrapped by the reflective call.

**Android channels.** The FCM transport sends `android.channel_id` under the `leptoskit.notifications.` prefix the plugin creates channels with, so a send that names `inbox` reaches the channel the app configured as `inbox`. An id that already carries the prefix is sent unchanged.

**iOS and macOS.** The permission request asks for alerts, badges, and sounds, so a push's `aps.badge` and `aps.sound` take effect where the person allows them; it asked for alerts only.

**macOS.** A push-enabled Mac app (`[native.macos] aps-environment`) now needs `[native.macos] provisioning-profile`, a new setting naming the profile to embed: Apple's push entitlement is restricted, and macOS refuses to launch an app that claims it without a matching embedded profile. Generation stops without the profile. `leptoskit build --platform macos` embeds it at `Contents/embedded.provisionprofile` and signs with the identity in `LEPTOSKIT_MACOS_IDENTITY`, refusing to ad-hoc sign; `--release --unsigned` builds without the push entitlement. `leptoskit publish macos` embeds the profile before signing with the Developer ID identity. This follows Apple's signing rules and has not been run against a real Apple developer account. Apps without `aps-environment` generate and sign exactly as before.

**Web.** The receiver worker claims the pages already open when it activates, so a click can route the tab a person enrolled from, and a click on an action button adds the button's id to the opened address as `lka`.

The guide's samples no longer stop on iOS and macOS at `configure_channel` or enroll after a refused permission, and it now covers Web Push on iPhone and iPad needing a Home Screen install, combining the receiver with an app's own service worker, which `aps-environment` value an iOS build wants, and the entitlement a time-sensitive push needs.

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_plugin_notifications: local notifications

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_notifications` is the leptoskit plugin for immediate local system notifications with explicit permission requests, bounded messages, and cancellation.

Add `leptoskit_plugin_notifications` as a direct dependency and select the `notifications` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Notifications` rather than resolving the plugin on every call. iOS, macOS, and Android post through the system (Android also configures channels), while browser and Linux apps call the `web` and `linux` modules directly; Windows has no backend yet. A successful call means the OS accepted the request, not that a banner appeared, and scheduled notifications, push, and tap routing are not implemented.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
