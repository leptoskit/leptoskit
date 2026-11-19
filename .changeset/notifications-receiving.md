---
"leptoskit_notifications": minor
"leptoskit_plugin_notifications": minor
"leptoskit_cli": minor
---

# Taps, foreground pushes, and data messages reach app code

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
