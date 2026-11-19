# Push setup

The inbox reaches a person only while your app is open on a screen. To show a banner when it is closed, the service has to hand the notification to the platform's own push service: Apple's (APNs) for iOS and macOS, Google's (FCM) for Android, and the browser's own for the web (Web Push). This page sets that up in two halves. First the operator gives the service its credentials for each platform. Then your app asks the person, gets the device's token or subscription, and enrolls it. After that every send that writes an inbox row also pushes to that person's enrolled devices, with nothing to change in your sending code.

Push works on the web, iOS, Android, and macOS. Windows, Linux, the terminal, HarmonyOS, and the watch surfaces have no enrollment: their apps read the inbox and receive no push.

A push never stands in for the inbox row. The row is stored first; the push is a best-effort signal that it is there.

## Configure a transport

Each platform's push service wants proof that a push comes from you, and each has its own form of it:

- **APNs** takes a signing key: a `.p8` file you create under Keys in your Apple developer account, with its key id and your team id. One key serves all your apps.
- **FCM** takes a service account: a JSON file you create for your Firebase project under Project settings, Service accounts. Android delivery goes through FCM, so an Android app needs a Firebase project even though you use nothing else of Firebase.
- **Web Push** takes a VAPID key pair, which you generate yourself (below). The browser subscribes with the public half, and the service signs each push with the private half.

Each transport is optional and reads its credentials from the service's environment. Provider credentials are secrets: they belong in your secret manager or deploy environment, never in the repo or in client code. A service starts with a cleared environment and receives only the variables its `[services.notifications]` table names under `secrets`, passed through from the environment `leptoskit dev`, `leptoskit serve`, or your deploy runs in — a variable you set but don't list never reaches it, and push stays off. List the variables of the transports you use, and leave out the others:

```toml
[services.notifications]
crate = "leptoskit_notifications"
version = "0.3"
trust = "first-party"
secrets = [
	# APNs: iOS and macOS
	"LEPTOSKIT_NOTIFICATIONS_APNS_KEY_P8",
	"LEPTOSKIT_NOTIFICATIONS_APNS_KEY_ID",
	"LEPTOSKIT_NOTIFICATIONS_APNS_TEAM_ID",
	"LEPTOSKIT_NOTIFICATIONS_APNS_TOPIC",
	"LEPTOSKIT_NOTIFICATIONS_APNS_ENVIRONMENT",
	# FCM: Android
	"LEPTOSKIT_NOTIFICATIONS_FCM_SERVICE_ACCOUNT",
	# Web Push: browsers
	"LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_VAPID_PRIVATE_KEY",
	"LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_VAPID_PUBLIC_KEY",
	"LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_VAPID_SUBJECT",
]
```

`leptoskit dev` warns about a listed variable that isn't set, and a generated [`compose` deploy](../24-deploying.md#the-targets) refuses to start without each one. What each holds:

| Variable                                             | What it holds                                                                                                                                      |
| ---------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| `LEPTOSKIT_NOTIFICATIONS_APNS_KEY_P8`                | the contents of the `.p8` signing key from your Apple developer account, line breaks included                                                      |
| `LEPTOSKIT_NOTIFICATIONS_APNS_KEY_ID`                | that key's id                                                                                                                                      |
| `LEPTOSKIT_NOTIFICATIONS_APNS_TEAM_ID`               | your team id                                                                                                                                       |
| `LEPTOSKIT_NOTIFICATIONS_APNS_TOPIC`                 | your app's bundle id                                                                                                                               |
| `LEPTOSKIT_NOTIFICATIONS_APNS_ENVIRONMENT`           | the default: `sandbox` or `production`, for a device whose build did not say which it is signed for                                                |
| `LEPTOSKIT_NOTIFICATIONS_FCM_SERVICE_ACCOUNT`        | the contents of the service-account JSON file the Firebase console issues                                                                          |
| `LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_VAPID_PRIVATE_KEY` | the VAPID private key: 32 bytes, base64url                                                                                                         |
| `LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_VAPID_PUBLIC_KEY`  | the VAPID public key: 65 bytes, base64url — the same key your web app passes to `browser::EnrollOptions::new`                                      |
| `LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_VAPID_SUBJECT`     | a `mailto:` or `https:` contact the push services can reach you at                                                                                 |
| `LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_EXTRA_HOSTS`       | optional: comma-separated push services beyond the [shipped ones](#which-push-services-a-browser-may-use); add it to `secrets` too when you use it |

The `.p8` key and the service account are files. Set the variables from the files rather than retyping them, so the key's line breaks survive:

```sh
export LEPTOSKIT_NOTIFICATIONS_APNS_KEY_P8="$(cat AuthKey_ABC123DEFG.p8)"
export LEPTOSKIT_NOTIFICATIONS_FCM_SERVICE_ACCOUNT="$(cat my-project-firebase-adminsdk.json)"
```

[Monosecret](../secrets.md) holds the same values: declare the names in your manifest's profile and store each file's contents as its value.

leptoskit has no command for a VAPID pair yet. The Web Push ecosystem's own generator prints one in exactly the form these variables take, both halves base64url:

```sh
npx web-push generate-vapid-keys
```

Keep the pair once your app has subscribers: a subscription is made for one public key, and a new pair can't push to it.

Your web app needs the public key to subscribe. It is not a secret: compile it into your client, or serve it from your own server. The service has no operation that hands it out.

A transport with all of its variables set is on; a transport with none is off (its platform is refused at enrollment, and the inbox is untouched); a transport with some set refuses to start, naming every missing variable in full, because half-configured looks like push works until the first send fails. `LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_EXTRA_HOSTS` without the VAPID variables counts as some.

The APNs topic and environment, the FCM project, and the VAPID identity are all yours on the server; an enrollment carries only the device credential, so nothing a client sends can steer where a push goes. The FCM service account must be the JSON Google issues — an **RSA** private key, which OAuth signs RS256; the exchange endpoint is fixed to Google's own, and the `token_uri` inside the JSON is ignored. The VAPID pair is cross-checked at startup, so a mistyped half refuses to start rather than failing at every push service at once. An extra-hosts entry that isn't a plain host name also refuses to start, named.

## Enroll a device

The signed-in person enrolls through the service's RPC — `register_push_endpoint` with one typed credential. What the credential is and how your app obtains it is platform work: request the OS's push permission, get the token or subscription, and hand it over. On iOS it is [this crate's `native` module](#enroll-an-ios-device), on Android the same module answers with an FCM token ([enroll an Android device](#enroll-an-android-device)), on macOS the same module answers with an APNs token ([enroll a macOS device](#enroll-a-macos-device)), and on the web it is [the `browser` module](#enroll-a-browser), further down.

```rust
use leptoskit_notifications::push::DeviceDetails;
use leptoskit_notifications::push::PushCredential;
use leptoskit_notifications::push::RegisterPushEndpoint;

let endpoint = leptoskit_rpc::client::call(RegisterPushEndpoint {
    credential: PushCredential::Apns {
        device_token: token_from_the_os,
    },
    device: DeviceDetails::default(),
    renews: None,
})
.await?;
```

You rarely write this call yourself: `native::enroll` and `browser::enroll` make it for you, with the device's details filled in. The service answers an opaque endpoint id — the credential itself never comes back, and never appears in a list, a status row, or an error. Registering a destination that account already holds answers the same endpoint again, so a retried enrollment makes no duplicate; a Web Push subscription whose browser rotated its keys updates the stored credential in place, keeping the endpoint and its queued deliveries. Credential rotation and reactivation advance a version: queued deliveries follow the new credentials, and late responses to older credentials cannot invalidate the endpoint or change the new queue.

A person holds at most ten live endpoints, and the service keeps at most ten more that a provider has invalidated — an uninstalled app, an expired subscription — so their device list can explain them. Past that, enrolling drops the oldest invalidated ones. Only the owner can bring an invalidated endpoint back, by enrolling it again within the live limit.

## A shared device

Two people use the same phone or the same browser. The first never signed out properly — their session expired, or they signed out with no connection — so the service still thinks the device is theirs. Without a rule for this, the second person couldn't turn notifications on, and the first person's notifications would keep appearing on a device they no longer hold.

The rule: **whoever presents the device's own credential holds the device.** When the second person turns notifications on, their app hands over the same token (or the same browser subscription, keys included) the first person enrolled. The service gives the device to them under a new endpoint id and revokes the first person's: it leaves their device list, and anything still queued for it is skipped as `endpoint_revoked`. Nothing for you to write — `native::enroll` and `browser::enroll` already do this.

It only happens when a person turns notifications on. A launch never takes a device over (see below), and a Web Push address presented with different keys is refused with a conflict: knowing the address isn't holding the browser.

## When the server revokes a device

An enrollment can end without the device's help: the person removes the device from a list on another device, the next person takes it over, your server revokes it, or `ForgetPerson` erases the person. The device finds out at its next launch — `reconcile` asks the service about the enrollment the app saved — and answers that it is gone (`native::Outcome::Gone`, or `None` in a browser). Clear what you saved and show the switch as off. **A launch never enrolls again in its place**: a revoke made on the server stays made until the person turns notifications back on themselves.

When your server ends someone's session without the device — "sign out everywhere", a suspended account, an expired session it cleans up — it can release their devices too, so their notifications stop arriving on devices they're signed out of:

```rust
let revoked = call_as_app(leptoskit_notifications::push::RevokePersonEndpoints {
    person: alice_id,
})
.await?;
```

That removes every endpoint the person holds and answers how many. Their inbox, preferences, topics, and attributes stay; [`ForgetPerson`](audiences.md#erase-a-person) is the call that erases those.

List and revoke are the person's own. The list is one bounded page at a time, oldest first, and it includes invalidated endpoints with their state — a device list that hides them can't explain them:

```rust
let page = leptoskit_rpc::client::call(leptoskit_notifications::push::PushEndpoints {
    page: Default::default(),
}).await?;
leptoskit_rpc::client::call(leptoskit_notifications::push::RevokePushEndpoint { endpoint: id }).await?;
```

Revoking deletes the endpoint and skips everything still queued for it as `endpoint_revoked`. Each attempt checks endpoint liveness immediately before sending: an attempt that passed that check before revocation committed may still reach the provider. Revocation does not recall provider delivery already in flight or visible banners. Another person's endpoint — or an id nobody holds — is `404`, exactly as if it didn't exist.

## Enroll an iOS device

On an iOS device the whole enrollment is this crate's `native` module, behind the feature of the same name. The Android flow is the same module with an FCM token ([the next section](#enroll-an-android-device)), and the macOS flow is the same module with an APNs token again ([enroll a macOS device](#enroll-a-macos-device)). Depend on the crate in your app, select the notifications plugin, and opt the iOS build into push:

```toml
[dependencies]
leptoskit_notifications = { version = "0.3", features = ["native"] }

[native]
plugins = ["notifications"]

[native.ios]
aps-environment = "development"
```

The `aps-environment` line is the opt-in. It is a string — Apple's entitlement takes exactly `development` or `production`, not a boolean — and generation writes it into `.leptoskit/ios/App.entitlements`. The value has to agree with the provisioning profile the build is signed with. With Xcode's automatic signing, which is what a leptoskit iOS build uses unless you configured manual signing, **keep `development`, for TestFlight and store builds too**: Xcode signs the archive with a development profile, and the export step re-signs it with your distribution profile, which is what puts the production environment into the build people install. Set `production` only when your own manual signing uses a distribution profile for the build itself. If a release build stops with a message that the provisioning profile doesn't match the `aps-environment` entitlement, this line is the one to change.

What decides where a device registers is the signature of the build that is installed, not this line: a build run from Xcode registers with Apple's sandbox, and a TestFlight or App Store build registers with production. The app tells the service which one: at enrollment the iOS and macOS hosts read the environment the installed build is signed for, and the service pushes each device through that environment's server. So one service can serve a build run from Xcode and a store build at once; see [development and store builds on one service](#development-and-store-builds-on-one-service). An app without the setting generates exactly as before, with no push entitlement: selecting the notifications plugin for local banners never turns an app into a push-enabled one. No provisioning or certificate access happens in the build.

The generated app delegate already forwards the OS's answers: `didRegisterForRemoteNotificationsWithDeviceToken` and `didFailToRegisterForRemoteNotificationsWithError` land in the notifications plugin's in-memory broker, which survives the early callbacks that can arrive before the plugin and its guest exist. The token is held in memory only — the OS hands it out again on the next launch — and Apple's bytes are forwarded as exact lower-case hex, of whatever length the OS chooses; nothing assumes the historical 32 bytes, and nothing is invented on failure.

Then the app's own flow, in order:

```rust
use leptoskit_notifications::native;
use leptoskit_plugin_notifications::guest::Notifications;
use leptoskit_plugin_notifications::Permission;

// From an explicit gesture — a "turn on notifications" switch, never a
// mount or a render: the OS permission question.
let notifications = Notifications::new()?;
if notifications.request_permission().await? != Permission::Granted {
	// Show your own "notifications are off" state and stop: enrolling a
	// device that can show nothing only earns pushes nobody sees.
	return Ok(());
}

// Still in the gesture: register, wait the OS's bounded answer, enroll.
// The answer is an opaque endpoint id — persist it, with the opt-in flag,
// beside the session. Never persist the token itself.
let enrollment = native::enroll().await?;
```

At **launch**, once the person is signed in, call `native::reconcile(Some(&enrollment))` — with the enrollment you saved, and only with one: it is the record that this person opted in on this device. Reconciliation registers with APNs again — registration prompts nothing, Apple expects it every launch, and a device that never opted in is never registered, so a person who turned notifications off is not silently re-enabled. It also reads the [device's details](audiences.md#what-a-device-says-about-itself) again. The answer is an explicit transition you persist:

- `Outcome::Kept` — the OS still issues the same token. When the device's details are unchanged too, nothing is registered: the module only asks the service, in one small read, whether the enrollment still stands (and if the service can't be reached, keeps the enrollment and asks again next launch). When they changed — an app update, a new language — the new details go in one idempotent call under the same endpoint id. Persist the enrollment it carries: it remembers what was sent, which is how the next launch knows whether anything changed. An enrollment saved by an older version of this module sends once, then settles.
- `Outcome::Rotated { current, retired }` — the OS now issues a different token. The module checks that the previous handle is still this person's and retires it — a rotated token never leaks an orphan endpoint — so you persist `current` and drop `retired`. When the person is at the ten-endpoint bound, the quota refusal is answered by retiring the known-stale handle and re-registering once, so rotation at the quota still completes.
- `Outcome::Gone` — the service no longer has this enrollment for the signed-in person: it was [revoked on the server](#when-the-server-revokes-a-device), taken over by the device's next person, or was never theirs — a previous account's leftover handle. Nothing was registered. Clear the saved enrollment and the opt-in and show the switch as off; only the person's own tap enrolls again.
- `Outcome::Off` — you passed no previous enrollment, so nothing was registered and nothing asked; the cue to show the opt-in switch.

```rust
// At launch, without prompting:
match native::reconcile(Some(&enrollment)).await? {
	native::Outcome::Kept(kept) => enrollment = kept,
	native::Outcome::Rotated { current, .. } => enrollment = current,
	native::Outcome::Gone(_) => {/* clear what you saved; show the switch as off */}
	native::Outcome::Off => {/* show the switch; from its handler, enroll */}
}
```

Failures name their retry state rather than swallowing it: `EnrollError::Unconfirmed` means the OS did not answer within the host's bound — the previous enrollment stands untouched, and a stale token is never dressed up as success; `EnrollError::Enroll` carries the handle to keep retrying with (`None` when the attempt already retired it); `RetireFailed` and `OwnershipUnknown` leave the new enrollment live and name the old handle whose revoke to retry. A later `reconcile` retries an unfinished retirement by itself.

**Sign out by disabling.** While the person is still authenticated, `native::disable(&enrollment).await?` revokes this person's service enrollment with a stable idempotency key derived from the endpoint id — the same contract as the web's `browser::disable` — so a retried sign-out is safe. It never changes OS registration or permission; the token stays the device's, and the next account enrolls from clean. Clear the saved enrollment and opt-in once the revoke is acknowledged: reconciliation runs only on what you pass it. If the sign-out never reaches the service — no connection, or the session had already expired — the device passes to [its next person](#a-shared-device) when they enroll, and your server can [release it](#when-the-server-revokes-a-device) in the meantime.

Registration failures are surfaced, not guessed around: the OS's provisioning refusal comes back as an error carrying a stable code — `aps-environment` names the missing or mismatched entitlement — never the OS's own error text, a silent OS answers an explicit "unconfirmed" instead of hanging or replaying a prior token, and a cancelled wait gives up without losing the callback — the broker keeps it for the next read. There is no swizzling, no background daemon, and no token cached to storage.

**What a push does while the app is closed** is the OS's work: APNs wakes the system, which shows the alert from the payload — title and body, collapsed by the notification's id — and your durable inbox row is already in the service. A push that arrives while the app is **open** shows its banner too, and a tap on either reaches your code through the [receiver](receiving.md#know-what-was-tapped-and-what-arrived). What this does **not** include: any claim of live APNs acceptance here — provider acceptance is only ever proven by a real device with real credentials, which this setup doesn't exercise.

## Enroll an Android device

On an Android device the enrollment is the same `native` module as iOS — the same `enroll`, `reconcile`, and `disable` calls, the same error shapes — but the OS's answer is an **FCM registration token** instead of an APNs device token, so the opt-in is a build switch instead of an entitlement. The macOS flow is the same module with an APNs token again ([enroll a macOS device](#enroll-a-macos-device)).

Depend on the crate and select the plugin exactly as for iOS, then opt the Android build into push:

```toml
[dependencies]
leptoskit_notifications = { version = "0.3", features = ["native"] }

[native]
plugins = ["notifications"]

[native.android]
push = true
```

The `push` line is the opt-in. Generation adds the Firebase pieces to the generated Android project and nothing else changes:

- the pinned `com.google.firebase:firebase-messaging` dependency, and Google's services Gradle plugin, applied by id through `.leptoskit/android/gradle-plugins.txt`, which the `leptoskit new` scaffold reads;
- the messaging service that receives FCM's token refreshes, declared with the `com.google.firebase.MESSAGING_EVENT` intent filter Firebase finds it by;
- the default channel a push is posted through when the send names none: `leptoskit.notifications.default`, named in the manifest and created by the notifications plugin when the app starts ([Android channels](shaping-messages.md#shape-a-message) says how to rename it);
- the small icon a push shows in the status bar, when your app has one: set `notification` under `[app.icons]` ([app icons](../24-publishing.md)) and the manifest points Firebase at the generated `@drawable/ic_notification`. Without it Firebase falls back to your launcher icon, which Android draws as a plain white shape, and generation says so;
- keep rules for the Firebase names the plugin looks up at runtime, so a minified release build still finds them;
- the `POST_NOTIFICATIONS` permission the local-notifications plugin already declares.

An app without the line generates byte-for-byte as before, so selecting the plugin for local banners never turns an app into a push-enabled one.

Two things are yours, not leptoskit's: **`google-services.json`** — the file the Firebase console issues for your app's package id, placed at `android/app/google-services.json`. Generation stops and names the path when push is on and it's missing; leptoskit never generates or commits it. And the Gradle script: `android/app/build.gradle.kts` is your file, so generation checks it instead of rewriting it. An app scaffolded before this setting gets an error that names the exact lines to add, and an app scaffolded with a version whose lines read `apply(plugin)` gets one that says to change them to `apply(plugin = plugin)` — the first form asks Gradle for a script file named after the plugin, and the build fails.

At runtime the flow is the iOS flow verbatim — request permission, enroll from the gesture, reconcile at launch with the saved enrollment, disable on sign out:

```rust
let enrollment = native::enroll().await?;
```

What differs under the hood: Firebase answers with a registration token — printable ASCII of no assumed length, not hex — the generated messaging service forwards token **refreshes** to the same in-memory broker the registration task feeds, and a device without usable Firebase refuses instead of pretending. The refusal is a stable code, not SDK text: `firebase-not-configured` means the app generated without the push opt-in or has no `google-services.json` values (the Firebase classes are found reflectively, so an app that never opted in simply answers this), `os-refused` covers the rest, and `token-shape` means Firebase handed over something outside the contract's bounds. The token is held in memory only — Firebase hands it out again on the next launch — and its bytes never appear in a log, an error, or a `Debug` print.

A device at Google Play services' mercy is a real constraint worth knowing: delivery needs Play services on the device. While your app is in the **foreground**, Firebase hands an arriving push to the app instead of showing it. The generated messaging service passes it to the notifications plugin, which posts it in the tray itself — title, body, the channel the send named or your default one, with a repeat of the same notification replacing the first — and tells your code it arrived. The image, color, and sound a send asked for are not applied to a push shown this way. What a push does while the app is **closed** is Google's work: the message's notification block shows the title and body in the system tray, collapsed by the notification's id, and your durable inbox row is already in the service. A tap on either opens the app and reaches your code through the [receiver](receiving.md#know-what-was-tapped-and-what-arrived). An app whose `android/` folder was generated before this needs nothing changed: the messaging service is generated on every build, and the tap is read from the intent your activity already keeps. And as everywhere: acceptance at the provider is only ever proven by a real device with real credentials, which this setup doesn't exercise.

## Enroll a macOS device

On a Mac the enrollment is the same `native` module as iOS and Android — the same `enroll`, `reconcile`, and `disable` calls, the same error shapes — and the OS's answer is an **APNs device token** again, lower-case hex of whatever length the OS chooses. The opt-in is the iOS-style entitlement switch, under the macOS table:

```toml
[dependencies]
leptoskit_notifications = { version = "0.3", features = ["native"] }

[native]
plugins = ["notifications"]

[native.macos]
aps-environment = "development"
provisioning-profile = "signing/Notes_Development.provisionprofile"
```

The `aps-environment` line is the opt-in, and on a Mac it comes with two requirements that a local-only app doesn't have. Apple's push entitlement — `com.apple.developer.aps-environment`, a string like iOS's, which generation writes into `.leptoskit/macos/App.entitlements` — is a restricted one: macOS refuses to launch an app that claims it unless the app is signed by a real certificate and carries a provisioning profile that grants it. An ad-hoc signature, which is how leptoskit signs a Mac app that has no push, can't do that; the app would be killed as it starts.

So a push-enabled Mac app needs:

- **A provisioning profile**, named by `provisioning-profile` as a path under your project. Create it in your Apple developer account for this app's bundle id (`native.package_id`) with the Push Notifications capability, and download it. Generation stops, naming the setting, when `aps-environment` is set and the profile isn't configured or isn't there. The build copies it into the app as `Contents/embedded.provisionprofile` before signing.
- **A signing identity** for local builds, in the `LEPTOSKIT_MACOS_IDENTITY` environment variable: the certificate's name as `security find-identity -v -p codesigning` lists it, such as `Apple Development: Your Name (TEAMID)`. `leptoskit build --platform macos` and `leptoskit dev --platform macos` stop without it rather than ad-hoc sign an app that can't launch. `leptoskit build --release --platform macos --unsigned` still builds, ad-hoc signed and without the push entitlement, for measuring or testing everything but push.

The profile and the environment go together. For local work use a **macOS development** profile with `aps-environment = "development"`: it registers with Apple's sandbox, and it runs only on the Macs listed in the profile. For `leptoskit publish macos` switch both: a **Developer ID** profile with `aps-environment = "production"`. The distribution embeds that profile, signs with your `LEPTOSKIT_PUBLISH_MACOS_IDENTITY`, and registers with production. The app reads the environment from the embedded profile and reports it at enrollment, so the service pushes a development build through the sandbox and a distributed build through production. leptoskit copies the profile and checks that it is there; it doesn't read what the profile grants, so a profile for another bundle id, another environment, or one without the push capability is something macOS reports only when the app launches.

An app without the `aps-environment` setting generates and signs exactly as before, with no push entitlement and no profile: selecting the notifications plugin for local banners never turns an app into a push-enabled one.

The generated macOS app delegate already forwards the OS's answers — `didRegisterForRemoteNotificationsWithDeviceToken` and `didFailToRegisterForRemoteNotificationsWithError` land in the notifications plugin's in-memory broker, the same one iOS uses, which survives the early callbacks that can arrive before the plugin and its guest exist. `REMOTE_REGISTER` calls `NSApplication.registerForRemoteNotifications` and waits the bounded answer; a silent OS answers "unconfirmed" instead of replaying a prior token, and the token's bytes never reach a log.

At runtime the flow is the iOS flow verbatim — request permission from a gesture, enroll from the gesture, reconcile at launch with the saved enrollment, disable on sign out:

```rust
let enrollment = native::enroll().await?;
```

One honest limitation: the stock **menu-bar host** (a `[native] mode = "menu-bar"` app that keeps the generated status-item lifecycle) carries no forwarding delegate, so its registration answers stay "unconfirmed", and it installs no notification-centre delegate either, so it shows no banner while it runs and hears no taps — a push-enabled app wants the standard windowed host, or its own delegate wired to the plugin's `noteDeviceToken`, `noteRegistrationFailure`, `installDelegate`, and `noteRemoteNotification`. And what a push does while the app is **closed** is the OS's work, as on iOS: APNs shows the alert and your durable inbox row is already in the service. A push that arrives while the app is open shows its banner, and a tap reaches your code through the [receiver](receiving.md#know-what-was-tapped-and-what-arrived). As everywhere, provider acceptance is only ever proven by a real Mac with real credentials, which this setup doesn't exercise — and the same goes for the signing above: the profile and identity steps follow Apple's code-signing rules, but they haven't been run against a real Apple developer account as part of leptoskit's own testing, so treat your first push-enabled Mac build as the check.

## Development and store builds on one service

Apple runs two push servers. A device token is known to one of them: the sandbox for a build signed for development, production for every other build. A token sent to the wrong one is refused, and the delivery ends as `operator_configuration`.

The service picks the server for each device. At enrollment the iOS and macOS hosts read which environment the installed build is signed for, and the enrollment carries it with the other [device details](audiences.md#what-a-device-says-about-itself). Your app writes no code for it. So a build you run from Xcode and the build your testers have from TestFlight can enroll with the same service, and each is pushed through its own server.

What the app reports:

| The installed build                                                          | Reports                                                                     | Pushed through                                              |
| ---------------------------------------------------------------------------- | --------------------------------------------------------------------------- | ----------------------------------------------------------- |
| runs in the iOS simulator                                                    | `sandbox`                                                                   | Apple's sandbox server                                      |
| has an embedded provisioning profile: run from Xcode, ad hoc, or a Mac build | what the profile's `aps-environment` says, where `development` is `sandbox` | the server it names                                         |
| is an iOS build from TestFlight or the App Store                             | `production`                                                                | Apple's production server                                   |
| could not tell, or predates this report                                      | nothing                                                                     | the server `LEPTOSKIT_NOTIFICATIONS_APNS_ENVIRONMENT` names |

`LEPTOSKIT_NOTIFICATIONS_APNS_ENVIRONMENT` is the default, for a device that reported nothing. Set it to what most of your devices are: `production` for a service people use.

Three things stay the same for both environments, because the service has one of each: the `.p8` key, the team id, and the topic. So both builds need the same bundle id, and the key must be allowed to use both servers. A key that Apple restricted to one environment when you created it is refused by the other, which also shows as [`operator_configuration`](troubleshooting.md#when-the-reason-is-operator_configuration).

The environment is reported again when it changes, at the next launch, like the other details. [`person_endpoints`](audiences.md#know-your-audience-before-you-send) and the person's own `push_endpoints` show it as `device.apns_environment`. Conditions do not read it.

A call that registers or imports an APNs token itself can name the environment in the same place:

```rust
use leptoskit_notifications::push::ApnsEnvironment;
use leptoskit_notifications::push::DeviceDetails;

let device = DeviceDetails {
    apns_environment: Some(ApnsEnvironment::Sandbox),
    ..DeviceDetails::default()
};
```

The service keeps the environment only for an APNs endpoint. On an FCM token or a browser subscription it is dropped.

## Enroll a browser

On the web the whole enrollment is this crate's `browser` module, behind the feature of the same name. Depend on the crate in your app and turn the feature on:

```toml
[dependencies]
leptoskit_notifications = { version = "0.3", features = ["browser"] }
```

The module is three things: the **receiver** service-worker script, an **availability** check, and the **enroll** and **disable** flows. All of it rides your app's own RPC client, so the enrollment is the signed-in person's, forwarded by your app exactly as their inbox reads are — no service token, and no copy of the session anywhere in the module or in the worker.

**Serve the receiver.** Build the script with your inbox route baked in and ship it with your web assets at the origin's root — `web/sw.js`, served as `/sw.js` with a JavaScript content type — so its scope covers the app:

```rust
let script = leptoskit_notifications::browser::receiver_script("/inbox")?;
// Write `script` to `web/sw.js` at build time, or serve it from a route.
```

`leptoskit build` copies `web/` into the served output as-is, so a build step (a script in your `justfile`, or a small generator run before `build`) that writes the built script to `web/sw.js` is enough. Or serve it from your server, same-origin at the root, with a JavaScript content type:

```rust
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};

async fn sw_js() -> Response {
    match leptoskit_notifications::browser::receiver_script("/inbox") {
        Ok(script) => ([(header::CONTENT_TYPE, "text/javascript")], script).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

// .route("/sw.js", get(sw_js)) on the same router that serves your app.
```

The route is the only URL a notification click may open, checked against the worker's own origin again at click time — payload content can never add a host, a scheme, or a path, and a route the builder refuses (empty, not a `/`-rooted path, scheme-relative, backslash, control characters) never ships. Register the script once at startup, the way [the web platform page](../../platforms/web.md#service-workers) does it, and wait for the worker to be active:

```rust
use leptoskit_notifications::browser;

let registration = browser::register("/sw.js", browser::RegisterOptions::default()).await?;
let _ = browser::ready().await?;
```

`ready` resolves once the registration has an **active** worker, which is all enrollment needs. It doesn't wait for that worker to control the page. The receiver takes control of the pages already open as it activates (`clients.claim()`), so the tab a person enrolls from is one a later click can route, without a reload first.

**One worker per scope.** A browser keeps a single service worker for a scope, and registering a second script at the same scope replaces the first. If your app already ships its own worker — the caching `sw.js` from [the web platform page](../../platforms/web.md#service-workers), say — don't register the receiver beside it. Serve the built receiver under another path (`/lkn-receiver.js`) and load it from the top of your own worker with `importScripts("/lkn-receiver.js")`, so one script holds both; leave the `push` and `notificationclick` events to the receiver. This combination is guidance rather than something leptoskit tests, so check a push and a click in your own app after wiring it.

**iPhone and iPad.** Safari on iOS and iPadOS offers Web Push only to a web app the person has added to their Home Screen (iOS 16.4 and later), which needs a web app manifest that opens it standalone. In an ordinary Safari tab there is no Push API at all, so `availability()` answers `unsupported` — show "add this app to your Home Screen" there rather than a switch that can't work.

**Ask before you offer.** `browser::availability()` reads where push stands — `ready`, `insecure_context` (serve `https`; nothing in code works around it), `unsupported`, or `denied` (the person refused; link to the browser's site settings rather than prompting again). It never prompts, so a settings screen may call it on every render. **Prompting is the browser's answer to subscribing, and it happens only inside `enroll`** — the two calls are split by when they may run:

- **`browser::reconcile`**, on launch, once the person is signed in, with the `Enrollment` you saved beside their session when they turned notifications on. It renews the subscription the browser holds under that enrollment and answers it back. It answers `None` — the cue to clear what you saved and show the switch — when you pass no saved enrollment, when the browser no longer holds a subscription, or when the service [no longer has the enrollment](#when-the-server-revokes-a-device). A browser can hold a subscription the previous person left behind, which is why a held subscription alone never enrolls anyone. Reading a subscription needs no permission, so this may run from any startup path.
- **`browser::enroll`**, from an explicit gesture — a button, never a mount or a render. It subscribes only when the browser holds no subscription, and that subscribe is the one call the browser gates on the gesture; waiting for the worker at launch (`browser::ready`) keeps it settled long before, so nothing asynchronous eats the gesture's activation window.

```rust
// At launch, after register and ready, without prompting. `saved` is the
// enrollment you kept beside this person's session, if they opted in:
let options = browser::EnrollOptions::new(vapid_public_key);
match browser::reconcile(&options, saved.as_ref()).await? {
    // Still enrolled: keep the handle it answers.
    Some(enrollment) => saved = Some(enrollment),
    // Not enrolled, or not anymore: clear what you saved and show the
    // switch. From its click handler, with the operator's VAPID public
    // key — the private half never leaves the service:
    //     saved = Some(browser::enroll(&options).await?);
    None => saved = None,
}
```

Every enrollment also carries the browser's [details](audiences.md#what-a-device-says-about-itself). A browser can't report your app's version, so name it on the options if you want to target web versions too — `browser::EnrollOptions::new(vapid_public_key).app_version(env!("CARGO_PKG_VERSION"))` — or leave it out and `device.app_version` stays empty for browsers.

`enroll` checks availability first, waits for the **active** worker instead of racing its install, reuses the subscription the browser already holds (a second tap re-enrolls the same endpoint instead of prompting twice, and two racing calls converge on one subscription), and registers it under the person who is signed in — but only when that subscription was created for **your own VAPID key**: a subscription made for another sender's key can't receive your pushes, so claiming it would end in silent delivery failure, and it is refused (`KeyMismatch`) rather than replaced. When the receiver is registered under an explicit scope, name it in `EnrollOptions::worker_scope`: the match is exact — a worker registered for a broader scope never stands in for the one you asked for. It answers an opaque `Enrollment` — the endpoint's id — which you keep beside the person's session on the device.

**Sign out by disabling.** While the person is still authenticated, `browser::disable(&enrollment).await?` revokes this person's service enrollment. It never unsubscribes the browser or changes OS permission. The subscription remains reusable: re-enabling can reuse it, but this service requires enrollment before sending. If the previous account never got to revoke, the browser passes to [the next person who enrolls in it](#a-shared-device).

Disable requires a successful authenticated revoke acknowledgement. Failures, including not-found, stay explicit; no endpoint list can establish success. Retries use a stable key derived from the endpoint id, including after reload. Receipts are scoped to the authenticated caller and replay only while retained by the service; pruning an old receipt can leave a retry unconfirmed. Revocation stops future attempts at the service's documented revocation boundary; provider delivery already in flight and visible banners are not recalled. Origin-wide unsubscribe and permission revocation are separate actions in browser settings or the lower-level worker API.

**When the browser changes the subscription on its own** — a rotated or replaced subscription, the `pushsubscriptionchange` event — the worker cannot reconcile anything: it deliberately holds no credentials, so it has no identity to enroll with. The supported reconciliation is the next authenticated app launch calling `reconcile` again with the saved enrollment; a stale handle's `disable` leaves the replaced subscription alone, and the replaced subscription's endpoint dies at its push service, whose answer invalidates the stale record on its next delivery. Churn is bounded by the ten-endpoint limit, and the person can revoke the dead ones from their own list.

The receiver itself shows what the service pushed: the envelope's title and body under a tag built from the notification's id, so a repeat replaces rather than stacks; the send's `data` rides on the banner's own data, inert. A send's options reach it too — an image, a badge count (`navigator.setAppBadge` where the browser has it), a silent flag, and the web presentation set. The service never pushes a [data message](shaping-messages.md#silent-data-messages) to a browser. One flagged `lkdata` that arrived anyway is handed to the pages that are open; when one of them is in view it shows nothing, and when none is it shows the "New notification" banner, because a browser punishes a push that shows nothing. A push that is missing, oversized (past the service's 4 KiB record), malformed, or typed wrong still shows one truthful banner — "New notification", no invented text — under a stable tag, so a broken sender can never silently swallow a delivery. A click closes the banner, focuses an app window of this origin and opens the send's own link when it resolves to this origin — a path like `/invoice/42`, or an absolute URL of this origin — and otherwise brings it to your inbox route with the notification's id, or opens that route in a new window. A click on one of the `web.actions` buttons does the same and adds the button's id to the address as `lka`, so the page can tell "Approve" from a click on the banner. An address the worker opens for a click always carries the notification's id as `lkn`, on a send's own link too. A page that runs a [receiver](receiving.md#know-what-was-tapped-and-what-arrived) is handed the click instead and changes its own route, with nothing loading again; a page that doesn't answer within half a second is brought to the address the old way, which loads it again there. A link anywhere else is refused at click time: payload content can never steer a click off the app's own origin. The worker holds no credentials and calls no server: nothing fetches, and **no read state changes until the authenticated app itself opens** and marks what it read.

## Which push services a browser may use

A browser's subscription names the address the service must post to, and that address comes from the client. So the service posts only to the push services it knows: Google's (`fcm.googleapis.com`, used by Chrome), Mozilla's (`updates.push.services.mozilla.com`, used by Firefox), Apple's (any subdomain of `push.apple.com`, such as Safari's `web.push.apple.com`), and Microsoft's (any subdomain of `notify.windows.com`, used by Edge on Windows). A subscription for any other host is refused when the browser enrolls, with a `400`, and so is one with a port other than 443, credentials in the address, or a private address.

If your users' browsers use another push service, name its host in `LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_EXTRA_HOSTS`, comma-separated, and add that variable to `secrets`. The service never follows a redirect from a push service.

## Platforms without push

Windows and Linux apps cannot enroll, so they receive no push. Neither can the terminal, HarmonyOS, or the watch surfaces. On all of them the inbox still works as it does everywhere: the app reads it when it is open.

## What happens when a push arrives

[Receiving on the device](receiving.md#what-each-platform-does) has the table of what each platform does when a push arrives while the app is closed, while it is open, and when it is tapped. If a push you expected did not arrive, [Delivery and troubleshooting](troubleshooting.md) shows how to ask the service what it did.
