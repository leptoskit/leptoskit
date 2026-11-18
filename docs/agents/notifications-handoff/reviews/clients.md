# Review: push clients on iOS, macOS, Android and the web

Read-only review of `main` at `93e2a2af0` on 18 November 2026. The reviewer built nothing and ran nothing, so every item is a claim: confirm it with a failing test before fixing it. Line numbers are from that commit. [The handoff](../readme.md) says which findings are already being fixed.

I found 13 problems. Two stop Android push completely, and one stops the macOS app from launching. Nothing was edited, built or run. Every finding below comes from reading the code. Where a claim depends on how a platform behaves rather than on something in the repository, I say so.

## Findings, most severe first

**1. Critical (high confidence). Android: every token reply is encoded in the wrong byte order, so every Android enroll and reconcile fails.**

- **Where:** `crates/leptoskit_plugin_notifications/android/NotificationsPlugin.kt:415-446`. `encodeSnapshot` uses `ByteBuffer.allocate(...)` without `.order(ByteOrder.LITTLE_ENDIAN)`, and Java's default is big-endian. The permission reply at line 476 does set little-endian. Swift and the Rust codec (`leptoskit_bridge/src/codec.rs:26,57`) are little-endian, and the readme says "All integers are little-endian".
- **Scenario:** Firebase hands over a token, so the reply should be revision 1, state 4. Rust reads it as revision `0x01000000` and state `0x04000000`, which is not a valid state. `RemoteRegistration::decode` returns `InvalidArgument`, and `native::enroll` / `reconcile` return `EnrollError::Register(...)` every time. `remote_snapshot()` fails the same way for any answered state. Only the empty `None` reply decodes.
- **Why tests missed it:** `tests/android/NotificationsPluginTest.kt:318` decodes little-endian, so those tests should fail. As far as I could find, CI never runs them: `connectedDebugAndroidTest` only runs the scaffold's `NativeAppTest`, and nothing copies plugin `tests/android` in.
- **Fix:** `ByteBuffer.allocate(16 + MAX_TOKEN_CHARACTERS).order(ByteOrder.LITTLE_ENDIAN)`, and wire the plugin's instrumentation tests into CI.

**2. Critical (medium-high confidence; based on how Gradle's Kotlin build scripts work). The Android build template applies the Google services plugin as if it were a script file, so push-enabled builds fail.**

- **Where:**
  - `crates/leptoskit_cli/src/native/templates/build.gradle.kts:8-12` has `?.forEach { plugin -> apply(plugin) }`.
  - The snapshot `snapshots__new_native__android_app_build.gradle.kts.snap:12-16` contains the same line.
- **Why it fails:** in Gradle's Kotlin DSL the signature is `PluginAware.apply(from: Any? = null, plugin: String? = null, to: Any? = null)`. The first positional argument is `from`, so the line becomes `apply(from = "com.google.gms.google-services")`.
- **What the developer sees:** Gradle fails while configuring the project with "Could not read script '…/app/com.google.gms.google-services' as it does not exist". The CLI's own message for older apps (`project.rs:206-208`) correctly says `apply(plugin = it)`. Tests only check that the text mentions `gradle-plugins.txt`.
- **Fix:** `apply(plugin = plugin)` in the template, then update the snapshot.

**3. High (high confidence; based on Apple's code-signing rules). Turning on `[native.macos] aps-environment` stops the macOS app from launching.**

- **Where:**
  - `crates/leptoskit_cli/src/native/desktop.rs:135-149,166-184` signs ad-hoc (`--sign -`) with `com.apple.developer.aps-environment`.
  - `crates/leptoskit_cli/src/publish/macos.rs:92-106` signs with Developer ID and the same entitlements.
  - Nothing anywhere embeds `Contents/embedded.provisionprofile`.
- **Why:** `com.apple.developer.aps-environment` is a restricted entitlement. macOS rejects a binary that claims it without a matching embedded provisioning profile ("No matching profile found") and kills it at launch.
- **What the developer sees:** opting into push makes the whole app crash immediately, for both `leptoskit build macos` and published builds. The guide (`41-notifications.md:678`) says both signatures carry the entitlement as though that works.
- **Fix:** with push on, require a provisioning profile and embed it at `Contents/embedded.provisionprofile` before signing (a development profile for local builds, a Developer ID profile for publish). Refuse ad-hoc signing in that case, and document it.

**4. High (high confidence). iOS and macOS install no notification-centre delegate, so nothing shows while the app is open and taps go nowhere.**

- **Where:**
  - `templates/App.swift:10-50` has no `didFinishLaunching` or `willFinishLaunching` and never sets `UNUserNotificationCenter.current().delegate`.
  - `templates/MacApp.swift:27-45` doesn't set one either.
  - A repository-wide search finds no `UNUserNotificationCenterDelegate`.
- **Scenario 1 (pushes):** a remote alert that arrives while the app is in the foreground is suppressed by the OS.
- **Scenario 2 (local banners):** local banners from `show()` are also suppressed while the app is in the foreground. That defeats the guide's advice to use the local plugin "where you want a banner while the app runs" (`41-notifications.md:485-526`) on Apple. The guide's iOS and macOS sections never mention foreground behaviour; only the plugin readme (line 56) does.
- **Scenario 3 (taps):** tap responses are never delivered to the app.
- **Fix:** install a host-owned delegate in `application(_:willFinishLaunchingWithOptions:)` on iOS and `applicationWillFinishLaunching` on macOS:
  - `willPresent` returns `[.banner, .list]`, plus `.sound`/`.badge` once those are authorized (finding 8).
  - `didReceive` forwards the tap into a broker (see the tap routing section).

**5. High (high confidence on the code; the browser behaviour is platform knowledge). Silent data messages (`kind: "data"`) reach no app code on any platform, and on the web they cause harm.**

- **iOS and macOS:**
  - No `remote-notification` entry in `UIBackgroundModes` (the plugin's `Cargo.toml` declares no `apple-background-modes`).
  - No `application(_:didReceiveRemoteNotification:fetchCompletionHandler:)` in `App.swift` or `MacApp.swift`.
  - Result: a `content-available` push never wakes or reaches the app.
- **Android:**
  - Nothing overrides `onMessageReceived`, and the generated service isn't even registered for messages (finding 6).
  - Result: data-only messages are dropped.
- **Web:**
  - `receiver.js:300-309` returns without showing anything.
  - Chrome then shows its own generic "This site has been updated in the background" notification.
  - Safari revokes the push subscription after repeated silent pushes. Push sends require `userVisibleOnly`.
  - The `setBadge` promise at lines 228-242 isn't awaited, so the worker can be stopped before the badge is set.
- **Guide mismatch:** `41-notifications.md:407` says "The send's `data` reaches your app"; line 411 says only that Chrome "may stop delivering".
- **Fix:**
  - Web: refuse data sends at the service, or show a notification for them; await the badge promise.
  - Native: add the background mode, the delegate handler, `onMessageReceived`, and a broker or guest method to read the data. Until then, document data messages as server bookkeeping only.

**6. Medium (high confidence; based on how Firebase routes messages). The generated Android messaging service never receives anything.**

- **Where:** `project.rs:379-388` declares `LeptosKitMessagingService`, but `manifest/services.rs:85-90` writes `<service … android:exported="false"/>` with no `<intent-filter><action android:name="com.google.firebase.MESSAGING_EVENT"/></intent-filter>`. The string `MESSAGING_EVENT` appears nowhere in the repo.
- **Scenario:** Firebase routes events to its own default service (declared with priority -500), so `onNewToken` (`project.rs:596-606`) never runs. A token that rotates mid-session isn't seen until the next launch's `reconcile`.
- **Docs claim otherwise:** the guide (lines 649 and 659), the readme (line 72) and the changesets all say token refreshes are forwarded. Even with the filter, foreground visible pushes would still be dropped because there is no `onMessageReceived`.
- **Fix:** give the generated service component that intent filter.

**7. Medium (low-medium confidence; depends on how R8 shrinks Firebase classes, which I did not verify). Release builds may break Firebase registration.**

- **Where:** `NotificationsPlugin.kt:162-216` reaches Firebase and Google Play services by name through reflection: `Class.forName(...)`, `getMethod("getInstance"/"getToken")`, `task.javaClass.getMethod("addOnSuccessListener", …)`, and a proxy that matches `method.name == "onSuccess"`.
- **Why it matters:** release builds minify (`build.gradle.kts:77-86`) and there are no keep rules for these names (`leptoskit_bridge_ffi/android/proguard-rules.pro`).
- **Possible symptom:** release-only `os-refused` or `firebase-not-configured`, or a hang that ends in `unconfirmed` because a renamed `onSuccess` falls through to `else -> null`.
- **Smaller bug in the same code:** when Firebase isn't initialised, the `IllegalStateException` arrives wrapped in `InvocationTargetException`, so the `catch` at line 211 never sees it. It is reported as `os-refused`, not `firebase-not-configured`.
- **Fix:** push-enabled builds already compile against `firebase-messaging`, so generate a static registrar next to `LeptosKitMessagingService.kt` and hand it to the plugin. Otherwise, emit `-keep` rules for those members when push is on.

**8. Medium (high confidence; based on Apple's authorization rules). Apple only asks for alert permission, so push badges and sounds do nothing.**

- **Where:** `apple/NotificationsPlugin.swift:154-156` calls `requestAuthorization(options: [.alert])`.
- **Scenario:** `aps.badge` (including `BadgeSetting::Unread`) and `aps.sound` from pushes are ignored on iOS and macOS. The guide's options table (lines 387-388) and the Badges section (lines 413-424) say they take effect.
- **Fix:** request `[.alert, .badge, .sound]`, or let the caller choose, and update the docs.

**9. Medium (high confidence). Android notifications shown by Firebase land in the wrong channel with the wrong icon.**

- **Channel mismatch:**
  - The plugin creates channels as `leptoskit.notifications.<id>` (`NotificationsPlugin.kt:604`).
  - The guide (line 394) says `android.channel_id` "posts through an Android channel your app must have created (the local-notifications plugin does)".
  - The transport sends the raw id (`fcm.rs:665-676`).
  - Firebase finds no such channel and falls back to its "Miscellaneous" channel.
- **Icon:** there is no `com.google.firebase.messaging.default_notification_icon` metadata, so Android shows a filled launcher-icon silhouette (often a white square), even though `ic_notification` exists for local banners.
- **Default channel:** there is no `default_notification_channel_id` metadata either.
- **Fix:**
  - Channel: add the prefix in the transport, or document the full id.
  - Icon and default channel: when push is on, add both metadata entries pointing at `@drawable/ic_notification` and a configured channel.

**10. Medium (high confidence on the code). On a shared device, the next person can't enroll if the previous one never disabled.**

- **Where:** `service/push.rs:204-206` returns `Conflict` ("this device belongs to another account") when another person holds the same token. APNs and FCM tokens don't change on sign-out.
- **Scenario:** account A's session expires, or A signs out offline, so `disable` never runs. Account B on the same device gets `EnrollError::Enroll(None, Conflict)` from `enroll` on every attempt, with no way to recover. The same holds in browsers.
- **Code and guide are misleading here:**
  - `Retirement::Stale` (`native.rs:238-243`) can only happen if the token also changed.
  - The guide's "previous account's leftover" bullet (line 612) describes a case that actually ends in `Conflict` when the token is unchanged.
- **Fix:** have the service hand ownership to the new person when they present the same device credential, or call Firebase `deleteToken()` on Android sign-out. At minimum, add a typed `EnrollError::HeldByAnotherAccount` and document it.

**11. Medium-low (high confidence). The ownership check reads only the first page of endpoints, so old endpoints can be left live.**

- **Where:** `native.rs:421-435` reads one page of at most 20 rows, oldest first (`service/push.rs:288-293`). Invalidated endpoints are kept with no limit and don't count toward the 10-endpoint quota (`push.rs:248-252`).
- **Scenario:** a person with more than 20 endpoints rotates their token. `owned` wrongly returns false, so the old live endpoint is marked `Stale` and never revoked. If they are at the quota, enrollment fails and the handle is dropped.
- **Fix:** skip the listing. Call revoke directly and treat 404 as stale; the service already answers 404 for endpoints the caller doesn't own. Otherwise, page through with `has_more`.

**12. Low-medium (high confidence on the code; Safari specifics are platform knowledge). Web: clicks often focus a tab without routing.**

- **Where:** `receiver.js:366-416,421-433`.
- **Problems:**
  - The worker never calls `clients.claim()`. `navigate()` rejects for a tab this worker doesn't control, which is always the case for the tab where the person first enrolled until it reloads, so the click focuses the tab and doesn't route.
  - When `navigate` does work, it reloads the whole Wasm app.
  - Action buttons are ignored (`event.action` is never read), so they behave like a body click.
  - `browser::ready()` resolves when a worker is active, not when it controls the page, but the guide (line 723) says "wait for it to control the page".
- **Fix:** see the tap routing section. At minimum, add `activate` → `clients.claim()` and handle `event.action`.

**13. Low (medium confidence). The guide leaves out steps and in places contradicts the code.**

- **Local-notification sample:** the sample calls `configure_channel` unconditionally with `?` (lines 503-511). It answers `Unavailable` on Apple, so the sample stops on iOS and macOS. The readme (line 54) says not to do this.
- **Permission samples:** the samples go on to enroll even when permission is denied (lines 599-606).
- **Web Push on iOS:** it works only in an app installed to the home screen with a manifest. The guide doesn't say so, and `availability()` reports "this browser has no service worker or Push API".
- **Service-worker conflict:** the receiver must be the only worker for its scope. The web platform page has apps write their own caching `/sw.js`, and there's no guidance on combining the two (for example with `importScripts`).
- **`aps-environment = "production"`:** suggesting it for TestFlight builds likely conflicts with development signing. Normally the entitlement stays `development` and the export step changes it. This rests on Xcode behaviour I didn't verify.
- **Time-sensitive alerts:** `time-sensitive` needs Apple's time-sensitive entitlement, and the guide doesn't say so.

## Tap routing today, platform by platform

**iOS**

- **Today:** a tap launches or foregrounds the app and nothing else happens. The payload carries top-level `lkn` (the notification's UUID), `lklink` (dropped by payload trimming) and `lk` (the send's `data`), but nothing reads them. On a cold launch with scenes, `connectionOptions.notificationResponse` in `scene(_:willConnectTo:)` isn't read either. Local notifications from `show()` carry no extra data at all.
- **Missing:**
  - A delegate installed before `didFinishLaunching` returns, so cold-launch taps aren't lost.
  - A `didReceive response` handler that takes `lkn`, `lklink` and `actionIdentifier`, calls the completion handler, and puts the result in a static broker like `RemoteRegistration`, because the guest may not exist yet.
  - A plugin method (for example 9, "take pending open") plus a wake-up so the guest learns about taps while running.
  - A guest-side rule for `lklink`: only paths starting with `/`, or the app's own domain, then fall back to the inbox route with `lkn`.
  - A link field on `show()` for local notifications.

**macOS**

- **Today:** the same gap as iOS, with no delegate on `MacApp`.
- **Missing, beyond iOS:**
  - Install the delegate in `applicationWillFinishLaunching`.
  - The app quits when its last window closes (`MacApp.swift:156`), so a tap is usually a cold launch.
  - The menu-bar host has no window at launch and has to open one before routing.

**Android**

- **Background or killed:** Firebase posts the notification. A tap fires the launcher intent with `FLAG_ACTIVITY_CLEAR_TOP`, and the `data` map arrives as string extras (`lkn`, `lklink`, the caller's keys, and `google.message_id`). `MainActivity` is `singleTop`, so it arrives in `onCreate` or `onNewIntent`, but those only call the shortcut and deep-link helpers. Nothing reads `lkn` or `lklink`.
- **Foreground:** the message goes to Firebase's default service and is dropped, so there is nothing to tap.
- **Local notifications:** they have no content `PendingIntent` (`NotificationsPlugin.kt:502-510`), so a tap does nothing, not even open the app.
- **Missing:**
  - `MainActivity` reads and clears those extras into a broker. It should only act when `lkn` and `google.message_id` are present, and skip intents with `FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY`.
  - The messaging-service intent filter (finding 6), plus `onMessageReceived` to forward foreground messages or post a local notification.
  - `show()` gains a link and sets an explicit content intent to `MainActivity` using `PendingIntent.FLAG_IMMUTABLE | FLAG_UPDATE_CURRENT` and a unique request code per notification.
  - The same guest API as iOS.

**Web**

- **Today:** the worker focuses the first same-origin window and calls `navigate`, or calls `openWindow` when there is none. The target is `lklink` if it is on the same origin, otherwise the inbox route with `?lkn=`. This works for a closed app or a background tab the worker controls.
- **Gaps:**
  - A tab the worker doesn't control isn't routed (finding 12).
  - Routing is a full page reload, not an in-app route change.
  - An `lklink` target doesn't carry `lkn`.
  - Action buttons are ignored.
- **Missing:**
  - Call `clients.claim()` on activate.
  - `postMessage({ lkn, link, action })` to the focused window, and fall back to `navigate` or `openWindow` if the page doesn't acknowledge it.
  - A page-side API in the `browser` module, built on the service-worker message support, that delivers these messages and reads `lkn` from the launch URL.
  - Append `lkn` to `lklink`.

## Examined and sound

- APNs token handling: lower-case hex of any length within 32-200 characters, matching Rust's `DeviceToken`.
- The Swift snapshot encoding is little-endian and matches the Rust codec.
- iOS and macOS remote registration runs on the main actor, one request at a time, with a bounded wait. A silent OS answers `unconfirmed`, and late callbacks are kept in the broker.
- The `aps-environment` failure code is matched on Cocoa error 3000 rather than error text.
- The iOS entitlement string and the macOS key name (`com.apple.developer.aps-environment`) are correct.
- The Android 13+ permission request checks the manifest and uses the host's broker; `Unknown` and `Denied` follow the rationale API.
- FCM token validation matches Rust's `FcmToken` and the service's bounds.
- In `native.rs`: the snapshot-to-credential mapping is right, and `reconcile` without a saved enrollment never registers (no silent re-enabling). The quota-retry path matches the service's message. The `disable` idempotency key is stable and shared with the browser module.
- Web subscribe options: `userVisibleOnly: true` is set and the VAPID key is converted from base64url correctly.
- Web enrollment safety: the key-mismatch guard works, the exact-scope check works, and `availability()` checks conditions in the right order.
- In the receiver: `waitUntil` covers both the push and click handlers. The size guard and "New notification" fallback work, links are kept to the app's own origin, and the inbox route is safely embedded as a JSON string.
- The transports: APNs background push type with priority 5 for data sends, collapse ids on every provider, and the TTL and expiration mapping. FCM sends a string-valued data map with reserved keys filtered out. Web Push sets `Topic`, `Urgency` and `TTL`.
- `MainActivity` (`singleTop`, `onNewIntent` calling `setIntent`) is a sound starting point for tap routing.

## What I could not review

- Anything at runtime: no device, simulator or emulator run, and no real Firebase or APNs credentials.
- Whether the pinned `google-services` 4.6.0 and `firebase-messaging` 26.0.2 exist and work with AGP 9.0.1.
- Whether Firebase and Google Play services AARs ship R8 rules that would avoid finding 7.
- How Xcode automatic signing (`-allowProvisioningUpdates`) handles the push capability, and what entitlements the iOS export step produces.
- Whether the host policy grants methods 7 and 8 by default.
- Whether Safari keeps the user gesture across the awaits in `browser::enroll`.

No pull requests were opened or touched.
