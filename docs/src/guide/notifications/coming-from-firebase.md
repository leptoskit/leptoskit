# Coming from Firebase

Your app sends its notifications with Firebase Cloud Messaging today: the Admin SDK on your server, the Firebase SDK in your clients, and the console's composer for campaigns. You want to move to a service you run yourself. This page maps what you know to what leptoskit has, walks through the move in an order that never leaves people without notifications, and lists plainly what leptoskit does not have yet, so you can decide before you start.

It applies to web, iOS, Android, and macOS apps. On Android you keep one piece of Firebase, described next.

## What stays and what goes

**You keep a Firebase project for Android.** FCM is how a push reaches an Android device, for leptoskit as for everyone. The service sends to FCM with a service account from your project, and your Android app keeps its `google-services.json`. You pay nothing more for this and use nothing else of Firebase.

**You stop using** the Firebase console's composer, Analytics audiences as targets, FCM's own topics, the Admin SDK's messaging calls, and the Firebase client SDKs on the web and on iOS. iOS and macOS pushes go from the service straight to APNs, and web pushes straight to each browser's push service.

**You gain** a durable inbox with read state shared across a person's devices, category opt-outs enforced by the service, idempotent sends, cancelling a send that is part-way out, a badge that is each person's own unread count, and all of the data in a database you host.

## The biggest difference: people, not devices

FCM addresses a **device**: you store registration tokens, send to tokens, and subscribe tokens to topics. leptoskit addresses a **person**: you send to the user id your sign-in already has, and the service keeps track of which devices that person enrolled.

So the token table in your own database goes away, after you [import it once](#moving-step-by-step). You no longer store tokens, refresh them, or prune dead ones. A topic belongs to the person, so subscribing on a phone also subscribes that person's laptop. And every device has to be signed in before it can enroll: there is no push to an anonymous installation.

## How the concepts map

| Firebase                                             | leptoskit                                                                                     | Notes                                                                                                                                                 |
| ---------------------------------------------------- | --------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| Registration token, installation id                  | [endpoint](push-setup.md#enroll-a-device)                                                     | an opaque id; the token itself never comes back out of the service                                                                                    |
| `send({ token })`                                    | [`SendTarget::Recipients`](sending.md#send-from-your-server)                                  | names people, and reaches all of each person's devices                                                                                                |
| `sendEachForMulticast` (500 tokens)                  | `SendTarget::Recipients` (1,000 people)                                                       | one transaction                                                                                                                                       |
| Topic                                                | [topic](audiences.md#send-to-a-topic)                                                         | per person, not per device; same name rules                                                                                                           |
| `subscribeToTopic(tokens, topic)`                    | `subscribe_to_topic`                                                                          | 1,000 people per call                                                                                                                                 |
| Client `subscribeToTopic`                            | [`subscribe`](audiences.md#let-people-subscribe-themselves)                                   | only for topics the operator opened; none are open by default                                                                                         |
| Condition                                            | [condition](audiences.md#target-with-conditions)                                              | the same grammar, so a topic condition pastes across; also reads `user.` attributes and `device.` details                                             |
| User properties, Analytics audiences                 | [attributes](audiences.md#describe-people-with-attributes)                                    | set by your server; you compute the audience and store it as an attribute or a topic                                                                  |
| Target by app, version, language                     | [`device.` predicates](audiences.md#target-devices)                                           | `device.platform`, `device.app_version`, `device.language`, `device.locale`, `device.time_zone`                                                       |
| Notification message                                 | a plain send                                                                                  | also writes the inbox row                                                                                                                             |
| Data message                                         | [`kind: data`](shaping-messages.md#silent-data-messages)                                      | not sent to browsers                                                                                                                                  |
| `android`, `apns`, `webpush` blocks                  | [`options.android`, `.apple`, `.web`](shaping-messages.md#shape-a-message)                    | a chosen set of fields, not every one FCM has                                                                                                         |
| `notification.image`                                 | `options.image`                                                                               | on iOS it needs a Notification Service Extension you add yourself                                                                                     |
| `android.priority`, `apns-priority`, `Urgency`       | `options.priority`                                                                            | `high` or `normal`                                                                                                                                    |
| `collapse_key`, `apns-collapse-id`, Web Push `Topic` | [`options.collapse_key`](shaping-messages.md#replace-an-earlier-push-and-let-a-push-go-stale) | one key for all three providers, 1 to 32 letters, digits, `-`, `_`; without one, the notification's own id                                            |
| `ttl`, `apns-expiration`, Web Push `TTL`             | [`options.push_ttl`](shaping-messages.md#replace-an-earlier-push-and-let-a-push-go-stale)     | seconds from when the notification becomes visible; `expires_at` is separate and also removes the inbox item. Pushes are retried for at most 24 hours |
| `fcm_options.analytics_label`                        | [`label`](sending.md#browse-the-sends-you-made)                                               | stored with the send and answered in its report                                                                                                       |
| `validate_only` / `dryRun`                           | [`validate_only`](sending.md#track-a-large-send)                                              | checks and counts inside the service; does not contact a provider                                                                                     |
| "Send test message" in the console                   | [`send_test`](troubleshooting.md#test-on-a-device)                                            | by person, not by token                                                                                                                               |
| Scheduled campaign                                   | `visible_at`                                                                                  | up to 366 days ahead                                                                                                                                  |
| "Recipient time zone" scheduling                     | [`local_time`](audiences.md#send-at-each-persons-local-time)                                  |                                                                                                                                                       |
| `title_loc_key`, `loc-key`                           | [variants](shaping-messages.md#say-it-in-each-persons-language)                               | the server picks the text per person; devices do not resolve keys from their own string tables                                                        |
| The console composer                                 | [campaigns](campaigns.md)                                                                     | the calls exist; a screen does not yet                                                                                                                |
| Campaign reports                                     | [`send_report`](sending.md#see-what-a-send-came-to)                                           | sends, provider acceptance, reads, and opens your app reports; no conversions                                                                         |
| `getToken`, `onTokenRefresh`, `onNewToken`           | [`native::enroll`, `native::reconcile`](push-setup.md#enroll-an-ios-device)                   | call `reconcile` at each launch                                                                                                                       |
| Web `getToken({ vapidKey })`                         | [`browser::enroll`](push-setup.md#enroll-a-browser)                                           |                                                                                                                                                       |
| `deleteToken`                                        | `native::disable`, `browser::disable`                                                         | removes the enrollment; the system's own registration is left alone                                                                                   |
| `onMessage`, `onBackgroundMessage`                   | [`Receiver`](receiving.md)                                                                    | `Received::Shown` and `Received::Data`                                                                                                                |
| `getInitialMessage`, `onNotificationOpenedApp`       | [`Receiver`](receiving.md)                                                                    | `Received::Opened`, with `launched` set for the tap that started the app                                                                              |
| Android notification channel                         | [`android.channel_id`, `android.raw_channel_id`](shaping-messages.md#shape-a-message)         | `channel_id` names a channel made with leptoskit's plugin; `raw_channel_id` names one your app already had                                            |
| Device group                                         | not needed                                                                                    | a person's devices already are the group                                                                                                              |
| Delivery data, BigQuery export                       | [`push_deliveries`](troubleshooting.md#read-what-happened-to-a-send)                          | provider acceptance per device; no device-side receipt and no export                                                                                  |
| Error codes                                          | [reason codes](troubleshooting.md#reason-codes)                                               | the service's own codes, not the provider's text                                                                                                      |

## The same calls, before and after

The Firebase snippets use the Node.js Admin SDK. The leptoskit ones run on your app's server.

**Send to one user's devices.** Before, you looked up the user's tokens and sent to them:

```javascript
await getMessaging().sendEachForMulticast({
	tokens: await tokensFor(userId),
	notification: { title: "Invoice paid", body: "Invoice #42 is settled." },
	data: { screen: "invoice", id: "42" },
});
```

After, you name the user:

```rust
use leptoskit_notifications::SendSpec;
use leptoskit_notifications::SendTarget;
use leptoskit_rpc::client::call_as_app;

call_as_app(leptoskit_notifications::Send {
    idempotency_key: "invoice-paid-42".into(),
    spec: SendSpec {
        category: "billing".into(),
        title: "Invoice paid".into(),
        body: "Invoice #42 is settled.".into(),
        data: Some(serde_json::json!({ "screen": "invoice", "id": "42" }).to_string()),
        ..SendSpec::default()
    },
    target: SendTarget::Recipients(vec![user_id]),
    validate_only: false,
})
.await?;
```

Two fields are new. The `idempotency_key` makes a retry safe: derive it from the event, not from the time. The `category` is what a person can opt out of.

**Send to a topic or a condition.** Before:

```javascript
await getMessaging().send({ topic: "news.sports", notification });
await getMessaging().send({
	condition: "'news.sports' in topics && 'alerts.local' in topics",
	notification,
});
```

After, with the same topic names and the same condition text:

```rust
call_as_app(leptoskit_notifications::Send {
    idempotency_key: "game-42-final".into(),
    spec: spec.clone(),
    target: SendTarget::Topic("news.sports".into()),
    validate_only: false,
})
.await?;

call_as_app(leptoskit_notifications::Send {
    idempotency_key: "game-42-final-local".into(),
    spec,
    target: SendTarget::Condition("'news.sports' in topics && 'alerts.local' in topics".into()),
    validate_only: false,
})
.await?;
```

These answer at once with the state `fanning_out` and deliver in the background; [`send_status`](sending.md#track-a-large-send) follows them.

**Subscribe to a topic.** Before, tokens:

```javascript
await getMessaging().subscribeToTopic(tokens, "news.sports");
```

After, people:

```rust
call_as_app(leptoskit_notifications::SubscribeToTopic {
    topic: "news.sports".into(),
    persons: vec![user_id],
})
.await?;
```

**Platform overrides.** Before:

```javascript
await getMessaging().send({
	token,
	notification: { title, body },
	android: {
		priority: "high",
		notification: { channelId: "orders", color: "#0A84FF" },
	},
	apns: {
		payload: { aps: { badge: 3, sound: "default", "thread-id": "order-42" } },
	},
	webpush: { fcmOptions: { link: "/orders/42" } },
});
```

After, the shared options come first and the per-platform blocks hold only what is particular to a platform:

```rust
use leptoskit_notifications::AndroidOptions;
use leptoskit_notifications::AppleOptions;
use leptoskit_notifications::BadgeSetting;
use leptoskit_notifications::MessageOptions;
use leptoskit_notifications::NotificationSound;
use leptoskit_notifications::SendPriority;

let options = MessageOptions {
    priority: Some(SendPriority::High),
    badge: Some(BadgeSetting::Set(3)),
    sound: Some(NotificationSound::Default),
    link: Some("/orders/42".into()),
    android: Some(AndroidOptions {
        channel_id: Some("orders".into()),
        color: Some("#0A84FF".into()),
        ..AndroidOptions::default()
    }),
    apple: Some(AppleOptions {
        thread_id: Some("order-42".into()),
        ..AppleOptions::default()
    }),
    ..MessageOptions::default()
};
```

`BadgeSetting::Unread` instead of a number badges each person with their own unread count, which FCM cannot do in one send.

**In the app.** `getToken` and the token-refresh callbacks become `enroll` when the person turns notifications on and `reconcile` at every launch. `onMessage`, `onBackgroundMessage`, `getInitialMessage`, and `onNotificationOpenedApp` become one loop over a `Receiver`. [Push setup](push-setup.md) and [Receiving on the device](receiving.md) have the code for each platform.

## Moving, step by step

The order matters. Each step leaves notifications working for everyone.

**1. Add the service and its credentials.** Add `[services.notifications]` ([Overview](readme.md#add-the-service)) and [configure the transports](push-setup.md#configure-a-transport):

- **APNs:** the `.p8` key you uploaded to Firebase works here too, with its key id and your team id. If you no longer have the file, create a new key in your Apple developer account; a key is not tied to one sender.
- **FCM:** create a service account in the **same Firebase project** your Android app already uses, so the app's `google-services.json` stays valid.
- **Web Push:** generate a VAPID pair. If you imported your own key pair into Firebase and still hold the private key, use that pair instead and existing browser subscriptions keep working. A pair Firebase generated for you cannot be exported.

Nothing changes for your users yet.

**2. Copy what you know about people.** From your own records, in batches of 1,000:

- subscribe people to the topics they follow, with `subscribe_to_topic`. FCM cannot list a topic's members, so this comes from your database, not from Firebase;
- set the attributes your segments need, with `set_person_attributes`;
- list the topics a person may join from a client in `LEPTOSKIT_NOTIFICATIONS_OPEN_TOPICS`.

**3. Import the tokens you hold.** Your server has a table of device tokens it collected for Firebase. Import them, and the people they belong to are pushed to from your next send, before anyone has updated the app:

```rust
use leptoskit_notifications::push::ImportEndpoint;
use leptoskit_notifications::push::ImportPushEndpoints;
use leptoskit_notifications::push::PushCredential;
use leptoskit_notifications::DeviceDetails;

let report = call_as_app(ImportPushEndpoints {
    endpoints: rows
        .iter()
        .map(|row| ImportEndpoint {
            person: row.user_id,
            credential: PushCredential::Fcm {
                registration_token: row.fcm_token.clone(),
            },
            device: DeviceDetails {
                locale: row.locale.clone(),
                ..DeviceDetails::default()
            },
        })
        .collect(),
})
.await?;
```

A call takes at most 500 entries, so send your table in batches. What to import:

- **Android:** the FCM registration tokens, as `PushCredential::Fcm`. They stay valid because the service's FCM service account is in the same Firebase project.
- **iOS, when your server holds APNs device tokens:** import them as `PushCredential::Apns`.
- **iOS, when your server holds only FCM tokens:** import them as `PushCredential::Fcm` too. The service sends them through FCM, which forwards to APNs for as long as your Firebase project keeps its APNs key. `device.platform` reads `fcm` for these.
- **Browsers:** nothing. A Web Push subscription cannot be imported; see the next step.

What the import does and does not do:

- The answer counts `imported` and `unchanged` entries and lists `skipped` ones by their index in the call. It never repeats a token.
- Importing the same token for the same person again changes nothing, so you can run the import again after a failure.
- A token another person already holds is skipped as `held_by_another`. An import never moves a device between people.
- A person holds at most ten live endpoints. Entries past that are skipped as `person_full`.
- One entry with a malformed token or a bad detail refuses the whole call and names the entry. Nothing is written.
- An imported endpoint has the `device` details you gave it and no others, so a [`device.` condition](audiences.md#target-devices) on a detail you did not give does not match it. [`person_endpoints`](audiences.md#know-your-audience-before-you-send) shows it with `imported: true`.
- The service keeps no record of a revoke or an erasure. If you import a token again after its person turned notifications off, or after `forget_person`, it is enrolled again. Import from your current records, once, and leave out people you erased.

An old app version treats a push from leptoskit like any other FCM notification message: the system shows it while the app is in the background, and your old `onMessage` code receives it while the app is open. It does not know leptoskit's tap routing: a tap opens the app, and the keys your old code read from the message are there only if you put them in the send's `data`.

**4. Release an app version that enrolls with leptoskit.** Each device enrolls itself when the new version runs. When it presents a token you imported for the same person, the imported endpoint becomes the device's own: it keeps its id and its queued pushes, takes the details the device reports, and is no longer marked imported.

- **Android:** set `[native.android] push = true` and keep `google-services.json`. Remove your own Firebase Messaging service and token code; the generated app has its own.
- **iOS and macOS:** set `aps-environment`. The app now enrolls its APNs device token directly. If you imported an FCM token for the same iPhone, the phone now has two endpoints until the FCM token dies. Both pushes carry the same collapse id, so iOS is asked to show one banner. To end the old endpoint sooner, call Firebase's `deleteToken` once in the updated app before you remove the SDK; FCM then reports the token as unregistered and the service invalidates the endpoint. This guide does not cover running the Firebase iOS SDK beside leptoskit's notification delegate in one app: remove Firebase Messaging from the iOS app in the same release.
- **Web:** replace `firebase-messaging-sw.js` with the [receiver](push-setup.md#enroll-a-browser). A browser that subscribed with Firebase's key holds a subscription leptoskit cannot use, and `browser::enroll` answers `KeyMismatch`. Unsubscribe the old subscription first, with the browser's own [`PushSubscription.unsubscribe()`](https://developer.mozilla.org/en-US/docs/Web/API/PushSubscription/unsubscribe), then enroll. Subscribing again needs a click from the person.

People who had notifications on must end up enrolled without being asked twice. On iOS, Android, and macOS the system permission they already gave still stands, and `native::enroll` never prompts by itself, so your app can enroll them once after the update if your own records say they had opted in. Whether that earlier opt-in covers the new enrollment is your call to make. In a browser the new subscription needs their click.

In the same release, add the [`Receiver`](receiving.md) loop so taps keep opening the right screen. A push from leptoskit carries its link and data in its own keys, not in Firebase's.

**5. Send each person through one system.** A person with an imported or enrolled endpoint is reachable through leptoskit, and sending through Firebase as well would show two banners. Your server can ask the service for a person's devices with [`person_endpoints`](audiences.md#know-your-audience-before-you-send): send through leptoskit to people who have at least one, and through Firebase to the rest. The rest are mostly browsers, which have to subscribe again, and tokens the import skipped. For topic sends during this period, send through leptoskit, and through Firebase only if browsers matter to that topic.

**6. Move the sends.** Replace each Admin SDK call with its equivalent from the section above. Give every send a category and a stable idempotency key. Try each kind on your own device with `send_test` first.

**7. Cut over.** When the share of people still on old versions is small enough for you, stop sending through Firebase, delete your token table and its refresh code, and remove the Firebase SDKs you no longer use. Keep the Firebase project and the service account: Android delivery depends on them.

## A checklist

Before you stop sending through Firebase, each of these should be true:

- The service runs, and `notifications_overview` shows each platform you need as configured.
- The APNs environment of the service matches the builds you ship (`production` for TestFlight and the App Store).
- The FCM service account and `google-services.json` are from the same Firebase project.
- Topics and attributes are copied from your own records.
- Your token table is imported, and you have looked at what the import skipped.
- The new app version enrolls, reconciles at launch, disables at sign-out, and runs a `Receiver`.
- The web app unsubscribes a Firebase subscription before enrolling.
- `send_test` is `accepted` on a real iPhone, a real Android phone, and a real browser.
- Your server sends each person through one system only.
- Every send has a category and an idempotency key derived from its event.
- Data messages you relied on in browsers have another route, such as a live query.

## What is not there yet

Check this list against what your app uses before you start. None of these has a date.

**Sending**

- **No recurring sends.** A send that repeats daily is a send your server makes each day.
- **No batch of different messages in one call**, like `sendEach`. Each send is one call.
- **No send to a raw token.** `send_test` reaches one device by its endpoint id, for testing only.
- **No localization keys.** [Variants](shaping-messages.md#say-it-in-each-persons-language) cover translated text; a device cannot resolve a key from its own string tables.
- **Fewer per-platform fields than FCM.** The Android, Apple, and web blocks hold the fields in [the options table](shaping-messages.md#shape-a-message) and no others. There is no notification visibility, vibration pattern, or light setting for Android, no critical alert sound for Apple, and no Live Activity push.

**Devices and apps**

- **No import of browser subscriptions.** [`import_push_endpoints`](#moving-step-by-step) takes FCM and APNs tokens. A browser subscribes again when the person next clicks, as in step 4.
- **An imported iOS FCM token and the same phone's own APNs enrollment are two endpoints.** The service cannot tell that they are one device, and your server cannot revoke a single endpoint: `revoke_person_endpoints` removes all of a person's.
- **One APNs environment per service.** Development builds and store builds cannot share one service.
- **No iOS Notification Service Extension is generated**, so a push's image is not shown on iOS or macOS until you add an extension of your own.
- **Android channels you created under Firebase are reached only by `android.raw_channel_id`.** `android.channel_id` names a channel made with the plugin, which keeps its channels under the `leptoskit.notifications.` prefix. For a channel your earlier app created as `orders`, send `raw_channel_id: "orders"`, and keep the code that creates the channel. The plugin cannot configure or list such a channel.
- **Android enrollment uses the FCM registration token**, not the Firebase installation id that Firebase is moving to.
- **Action buttons are partial.** Web buttons work. Apple buttons appear only for a category your own host code registers. Android pushes carry no buttons.
- **Data messages are not sent to browsers at all**, and on Android a data message that arrives while the app is not running is held in memory, not stored.
- **A push shown by the app while it is open on Android** ignores the send's image, colour, and sound. It is also not replaced by a later push with the same `collapse_key`.
- **A macOS menu-bar app** receives no taps and shows no banner while it runs.
- **No provisional (quiet) permission request on iOS.**
- **No push on Windows or Linux.**
- **Endpoints are not pruned by age.** A device is removed when its provider says it is gone, not after a period of silence.

**Composing and reporting**

- **No compose screen.** [Campaigns](campaigns.md) are calls your own interface drives. The checks a screen would do for you, such as who may send to everyone, are your server's to make.
- **No A/B tests.**
- **No conversions or funnels.** A report counts stored rows, reads, opens your app reported, and provider acceptance.
- **No delivery receipts from devices, and no export** such as FCM's BigQuery export. Opens are counted once per person, not per device.
- **Report counts shrink with retention.** They are counted over rows still stored, so keep the numbers yourself if you need them for longer than the [retention](privacy-and-retention.md#how-long-things-are-kept) you set.
- **No targeting by first open or last activity.** Keep such values fresh as attributes yourself.
- **Your server cannot read one person's topic list.** It can list topics and their sizes; a person's own list is theirs to read.

**Erasure**

- **`forget_person` does not edit campaign drafts**, and the service keeps no record that a person was erased. See [Privacy and data retention](privacy-and-retention.md#erase-one-person).

## Where Firebase documents its side

- [FCM HTTP v1 message reference](https://firebase.google.com/docs/reference/fcm/rest/v1/projects.messages), for the fields compared above.
- [FCM error codes](https://firebase.google.com/docs/cloud-messaging/error-codes), which the service reduces to its [reason codes](troubleshooting.md#reason-codes).
- Apple's [Handling notification responses from APNs](https://developer.apple.com/documentation/usernotifications/handling-notification-responses-from-apns).
