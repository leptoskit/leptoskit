<h1 align="center">leptoskit_plugin_notifications</h1>

<p align="center">
  <strong>leptoskit_plugin_notifications</strong> is the leptoskit plugin for immediate local system notifications with explicit permission requests, bounded messages, and cancellation.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_notifications"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_notifications"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_notifications"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_notifications.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_notifications"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_notifications/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Immediate local notifications with explicit permission requests, bounded binary messages and cancellation. A successful call means the platform accepted the request. It does not prove that a banner appeared or the user saw it: Focus modes, channel settings, foreground presentation policy and desktop services control delivery.

| Platform    | Implementation                          | Scope                                                                                                                                                                                 |
| ----------- | --------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| iOS / macOS | Swift `UNUserNotificationCenter`        | Permission query/request, immediate posting, named cancellation, namespace cancellation; both also APNs token acquisition and registration, device details, and what the app received |
| Android 26+ | Kotlin `NotificationManager`            | The above, plus channels and Android 13 notification permission; FCM registration-token acquisition and registration, device details, and what the app received                       |
| Browser     | Rust `web` module using `Notification`  | Explicit permission request, posting and cancellation through retained handles                                                                                                        |
| Linux       | Rust `linux` module using session D-Bus | Immediate posting and cancellation through retained handles                                                                                                                           |
| Windows     | Unavailable                             | No Windows toast backend yet                                                                                                                                                          |

Scheduled notifications, service-worker notifications, badges, attachments and delivery receipts are not implemented. Taps, notifications that arrive while the app is open, and silent data messages are handed to the guest on iOS, macOS and Android ([Receiving](#receiving-ios-macos-and-android)); the browser and Linux backends hand over nothing. Remote push registration is iOS, macOS and Android only, and it acquires a device credential: it does not send anything to a push provider. Unsupported operations return `Unavailable` where the backend exposes them. Linux and browser APIs are called directly from Rust; they are not native bridge registrations.

## Native registration and use

The crate metadata provides Apple and Android registration information for the LeptosKit generator. Android requests `android.permission.POST_NOTIFICATIONS` in the generated manifest and uses the generated activity's `NativePermissionHost` broker. A custom Android host must implement that interface to request runtime permission. Querying permission and posting do not need the permission broker.

Enable the `guest` feature in Wasm guest code and grant the methods the app needs in the host policy. The guest client caches its numeric plugin ID.

Channel configuration is Android-only: every other platform answers `Unavailable`, so treat that answer as "no channels here", not as a failure. Apple validates the portable channel identifier but does not use Android channels. Android channel importance and sound become user-controlled after channel creation; configuring the same ID does not override their choices. Notifications this plugin posts are silent on Apple, browser and Linux; Android sound is an explicit channel option.

On iOS and macOS the permission request asks for alerts, badges and sounds together. The plugin's own posts use only alerts; the badge and sound are for pushes from the notifications service (`aps.badge`, `aps.sound`), which the system applies only where the person allowed them. The system asks once, so a person who answered an earlier build's alert-only request may keep badges and sounds off until they change it in Settings.

On Android a channel configured with the ID `inbox` is `leptoskit.notifications.inbox` on the device. The notifications service adds that prefix to a send's `android.channel_id`, so sends name the same ID the app configured. A push-enabled app's manifest names `leptoskit.notifications.default` as Firebase's default channel; the plugin creates that channel when it is constructed and the manifest names it, called "Notifications" at default importance, and never touches it again. Configure the channel `default` to give it the app's own name and importance. The Android constructor accepts a small-icon resource; its default is the system information icon. Apps should supply a suitable monochrome notification icon when registering a custom instance.

Apple requires a properly packaged application with a bundle identity. The plugin never sets the notification centre's delegate itself: the generated iOS and macOS apps install the plugin's `NotificationCenterDelegate` before launch finishes, and with it a notification shows while the app is open, a local one included. An app with its own delegate forwards `willPresent` and `didReceive` to `ReceivedNotifications`. On Android a posted notification carries a content intent to the app's launcher activity, so a tap opens the app and names the notification.

Android permission `Unknown` means the OS cannot distinguish an initial unrequested permission from a denied permission with no rationale. It is not a claim that another prompt is possible. Apple distinguishes not-determined, provisional and ephemeral authorization.

## Lifetime and cancellation

Native IDs use the `leptoskit.notifications.` namespace. A completed post with the same ID replaces the previous notification where the OS supports replacement. Use one registered plugin instance per application and distinct IDs for independent callers; cancellation is application-wide within that namespace. Cancel-all leaves notifications from other providers untouched.

Apple limits in-flight posts to 64. A concurrent post with the same ID returns `Busy` until the prior OS call completes. Cancelling a post, cancelling its bridge future or closing the plugin retracts pending work, including a late result from an OS call that cannot be aborted. Once posting has completed, closing the plugin does not dismiss delivered notifications. Cancelling a permission request cannot close the OS permission dialog or undo an authorization choice.

Browser permission requests need a secure context and usually a user gesture. Browsers that require service-worker delivery report `Unavailable` from `show`; this module does not install a service worker. Retain the returned `web::Handle` to cancel it. Dropping a handle preserves delivery. Browser tags provide replacement semantics subject to browser support.

Linux `Notifications::new` connects to the session bus without posting anything. The service assigns a numeric ID returned inside the cancellation handle; the portable ID is not used to replace notifications. Dropping a handle preserves delivery. Calls can block up to five seconds and belong off the rendering thread. The body is escaped for the desktop notification protocol's markup. App display names are metadata, not trusted application identity. Linux desktop permission prompting is not standardized, so this module does not manufacture a permission state.

## Remote registration (iOS, macOS, and Android)

On iOS and macOS hosts the generated app delegate forwards `didRegisterForRemoteNotificationsWithDeviceToken` and `didFailToRegisterForRemoteNotificationsWithError` into a process-wide broker; on Android hosts the generated app — with `[native.android] push` on — declares a messaging service, with the `com.google.firebase.MESSAGING_EVENT` intent filter Firebase finds it by, whose `onNewToken` forwards FCM registration-token refreshes into the same broker, and `REMOTE_REGISTER` asks `FirebaseMessaging` for its token reflectively, so an app that never opted in answers `firebase-not-configured` instead of failing to compile with a dependency it does not carry. Firebase without initialization values answers `firebase-not-configured` too, including when the SDK's exception arrives wrapped by the reflective call. A push-enabled app's generated R8 rules keep the Firebase and Google Play services names those lookups use, so a minified release build registers the same way a debug build does; an app with its own Gradle setup that selects this plugin and shrinks its code needs the same rules. The plugin's `REMOTE_TOKEN` reads the broker's latest answer while `REMOTE_REGISTER` asks the OS to register — one OS request at a time, later callers joining the wait — and waits a bounded time for a **new** answer. The broker is static on purpose: answers that arrive before the plugin or its guest exists are not lost, a closed plugin leaves the OS's answer readable, and no closed guest is retained; a plugin closed while waiting never answers a token. The token is held in memory only — the OS hands it out again on the next launch — and its bytes are never logged, never printed by a `Debug` or description, and never re-issued as a fresh answer when the OS stays silent: silence at the bound answers `Unconfirmed`, and the broker's previous answer stays readable through `REMOTE_TOKEN`. Registration refusals cross as the contract's stable codes — `os-refused`, `aps-environment`, `firebase-not-configured`, `token-shape` — never the OS's or SDK's own error text. Every other platform answers `Unavailable` for these methods, and so does a host that predates `DEVICE`. `DEVICE` reads what the device says about itself for push targeting, which the notifications service's enrollment sends with no app code: the preferred locale (Foundation's first preferred language on Apple, the default locale's tag on Android), the current time zone's identifier, and the app's version (`CFBundleShortVersionString` on Apple, the package's version name on Android), read fresh on every call and answered as three length-prefixed strings, empty for unknown. A value past 256 bytes or with a control character is answered as unknown rather than refusing the rest. `APS_ENVIRONMENT`, on iOS and macOS hosts only, names the APNs environment the running build is signed for, which the same enrollment sends so the service pushes each device through the server that knows its token: a simulator answers development, a build with an embedded provisioning profile answers what the profile's `aps-environment` entitlement says, an iOS build without a profile — one that came through TestFlight or the App Store — answers production, and a macOS build without one answers unknown. Android, and a host that predates the method, answer `Unavailable`. Registration is not permission: alerts still need `request_permission`, and without the app's `aps-environment` entitlement (iOS, or `com.apple.developer.aps-environment` on macOS) or its `google-services.json` values (Android) the platform refuses.

## Receiving (iOS, macOS, and Android)

`RECEIVED` hands the guest what the device received, oldest first: a tap on a notification or one of its action buttons (kind 0), a notification that arrived while the app was open and was shown (kind 1), and a message handed to the app without a tap, which is a silent data message (kind 2). Each item is the kind, the pressed action's identifier (empty for a tap on the notification itself), the identifier of the local notification when it is one this plugin posted, and the provider's payload as JSON text. The host interprets nothing: `leptoskit_notifications` reads the notification's id, the link and the send's data out of the payload, and its `native::Receiver` is the API an app uses.

The request names how long the host may wait for something to arrive, at most ten seconds; `0` never waits. A waiting call ends as soon as something arrives, an empty wait answers an empty batch, and a cancelled wait loses nothing. Like remote registration, the items live in a process-wide broker, because a tap that starts the app arrives before the plugin and its guest exist. It keeps the latest sixteen, and a payload over 8 KiB or an action over 256 bytes is handed over as empty rather than cut.

On iOS and macOS the items come from the notification-centre delegate the generated apps install (`willPresent`, `didReceive`) and from the app delegate's `didReceiveRemoteNotification`, which a push-enabled iOS app is woken for through the `remote-notification` background mode generation adds. Dismissing a notification is not recorded. A notification that arrives while the app is open is shown with its banner, list entry, sound and badge. On Android the plugin reads a tap from the activity's own intent, at construction and on every read, and takes its marks off the intent so it is read once; the generated messaging service forwards `onMessageReceived`, and for a message with a title or body, which Firebase hands over only while the app is open, the plugin posts the notification Firebase would have posted. Android action buttons are not sent by the service, so an Android item never names an action.

## Binary protocol

All integers are little-endian `u32`. Strings are `u32` byte lengths followed by strict UTF-8. Every decoder rejects trailing bytes, truncated fields, invalid UTF-8 and values outside their domain before calling a platform API.

| ID | Method             | Request                                                   | Response                            |
| -- | ------------------ | --------------------------------------------------------- | ----------------------------------- |
| 0  | Permission         | Empty                                                     | Permission `u32`                    |
| 1  | Request permission | Empty                                                     | Permission `u32`                    |
| 2  | Show               | ID, title, body, channel strings                          | Empty                               |
| 3  | Cancel             | ID string                                                 | Empty                               |
| 4  | Configure channel  | ID, name, description strings, importance and sound `u32` | Empty                               |
| 5  | Delete channel     | ID string                                                 | Empty                               |
| 6  | Cancel all         | Empty                                                     | Empty                               |
| 7  | Remote token       | Empty                                                     | Revision `u64`, state `u32`, string |
| 8  | Remote register    | Empty                                                     | Revision `u64`, state `u32`, string |
| 9  | Device             | Empty                                                     | Three strings                       |
| 10 | Received           | Wait bound in milliseconds `u32`                          | Count `u32`, then items             |
| 11 | APNs environment   | Empty                                                     | Environment `u32`                   |

IDs contain 1–64 ASCII letters, digits, `.`, `_` or `-`. Titles have at most 256 UTF-8 bytes, bodies 4096, channel names 128 and descriptions 1024. Titles and names must contain non-whitespace text. Control characters are rejected except line feeds and tabs in bodies/descriptions. Sound is exactly 0 or 1; importance is Low=1, Default=2 or High=3. Permission is Unknown=0, NotDetermined=1, Denied=2, Granted=3, Provisional=4 or Ephemeral=5. Remote state is None=0, APNs token=1, Failed=2, Unconfirmed=3 or FCM token=4; the APNs token is an even 32–200 count of lower-case hex characters, the FCM token 1–4096 bytes of printable ASCII, neither an assumed shape, and failure reasons are the contract's stable codes, never the OS's own text. The revision is the host's monotonic answer counter; 0 means no answer has arrived, and only `REMOTE_REGISTER` answers Unconfirmed — silence at its wait bound, not a stale token. A received item is a kind `u32` (Opened=0, Presented=1, Message=2) and three strings: the action, at most 256 bytes without control characters; the local identifier, empty or an ID; and the payload, at most 8192 bytes. A reply carries at most 16 items, and a wait bound over 10000 is read as 10000. The APNs environment is Unknown=0, Development=1 or Production=2.

## Verification and references

Rust tests cover binary validation and truncation. Apple tests use an injected service to verify late completion, cancellation and close without posting system notifications. The browser test replaces `Notification` with a controlled fake and exercises explicit permission, payload fields, cancellation and unsupported service-worker-only behavior. The Android instrumentation suite posts silent notifications only in a disposable emulator fixture and checks replacement, namespace cancellation and close against `NotificationManager.activeNotifications`, and, with an injected registration boundary, bounded registration waits, unconfirmed silence, token-shape refusals, close-while-waiting and the one-ask convergence. It also covers the default push channel (created once in the plugin's namespace, never overwritten, absent in an app whose manifest names none), the stable code for a wrapped Firebase failure, and the real registrar's answer in an app without Firebase — no real Firebase run, which needs a real `google-services.json` and a device, and no minified build with Firebase, so the keep rules are not exercised by a test. The `DEVICE` reply is covered on both hosts with an injected source — the three strings, unknown as empty, an unshippable value answered as unknown — beside the Rust codec's own bounds. The `APS_ENVIRONMENT` reply has the Rust codec's test, a compiled guest that enrolls with a scripted answer, and an Apple test that reads made-up provisioning profile bytes; no test reads a real build's profile. Receiving is covered the same way: the Rust codec's bounds and truncation, a compiled guest that takes scripted batches through `native::Receiver`, and on both hosts the broker's order, bounds and once-only reads, the bounded wait, and close while waiting. The Android suite also reads a tap from an intent and posts a push that arrived while the app was open. No test here taps a real notification or receives a real push: the delegate callbacks on Apple and the Firebase service on Android run only on a device.

API scope is informed by [Expo Notifications](https://docs.expo.dev/versions/latest/sdk/notifications/), [Tauri Notification](https://v2.tauri.app/plugin/notification/) and [Electron Notification](https://www.electronjs.org/docs/latest/api/notification). Platform contracts come from [Apple authorization](https://developer.apple.com/documentation/usernotifications/asking-permission-to-use-notifications), [Android notification permission](https://developer.android.com/develop/ui/views/notifications/notification-permission) and the [Freedesktop notification protocol](https://specifications.freedesktop.org/notification/latest/protocol.html).

## How it fits

<!-- {=p_notificationsSummary} -->

Add `leptoskit_plugin_notifications` as a direct dependency and select the `notifications` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Linux hosts register it where it has a Linux Rust implementation; generated Windows and terminal hosts currently omit runtime plugins, so use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::Notifications` rather than resolving the plugin on every call; web builds keep the default `browser` feature, and interpreter guests turn it off so their dependency tree stays free of browser imports. A successful call means the OS accepted the request, not that a banner appeared; focus modes and user settings can still suppress it. On iOS and macOS the generated app delegate forwards APNs device tokens into the plugin's broker, and on Android a push-enabled generated app forwards FCM registration tokens into the same broker; `REMOTE_REGISTER` turns an explicit opt-in into a bounded, typed registration answer, answers `unconfirmed` when the OS stays silent rather than replaying a prior token, and carries refusal as stable reason codes instead of the OS's own text. Every other platform reports `Unavailable` for it.

<!-- {/p_notificationsSummary} -->

## Use it

```rust,ignore
use leptoskit_plugin_notifications::{Channel, Identifier, Importance, Notification};
use leptoskit_plugin_notifications::guest::Notifications;

let client = Notifications::new()?;
let channel_id = Identifier::new("downloads")?;
// Android only: create the channel before requesting permission or posting.
// Every other platform has no channels and answers `Unavailable`.
let channel = Channel::new(channel_id.clone(), "Downloads", "", Importance::Low, false)?;
match client.configure_channel(&channel).await {
    Ok(()) | Err(leptoskit_bridge::Error::Unavailable) => {}
    Err(error) => return Err(error.into()),
}
// Requesting can be denied: post only when it was granted.
if client.request_permission().await? != leptoskit_plugin_notifications::Permission::Granted {
    return Ok(());
}
let notification = Notification::new(
    Identifier::new("download-1")?,
    "Download complete",
    "Your file is ready.",
    channel_id,
)?;
client.show(&notification).await?;
client.cancel(notification.id()).await?;
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_notifications)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
