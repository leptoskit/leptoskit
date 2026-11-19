# Shaping messages

One title and one body, shown the same way everywhere, is the right message most of the time. This page is for the other times: a message that should not make a sound, one that carries a picture, one that only corrects the badge, one that nobody should see at all, and one that reads in each person's language. It also shows how to see exactly what each provider would be sent before you send it. Options take effect where a push is delivered: iOS and macOS through APNs, Android through FCM, and browsers through Web Push. The inbox row is the same on every platform whatever the options say.

## Shape a message

A plain send shows a banner with its title and body everywhere. Most of the time that's right, and then there are the others: the message that shouldn't wake anyone, the one that should carry a picture, the one whose whole job is to correct the badge count. `SendSpec` takes an `options` object for those, and every field is optional — a send without options behaves exactly as sends always did, byte for byte on the wire.

```rust
use leptoskit_notifications::BadgeSetting;
use leptoskit_notifications::MessageOptions;
use leptoskit_notifications::NotificationSound;
use leptoskit_notifications::SendPriority;

let sent = call_as_app(leptoskit_notifications::Send {
    idempotency_key: format!("invoice-paid-{invoice_id}"),
    spec: SendSpec {
        category: "billing".into(),
        title: "Invoice paid".into(),
        body: format!("Invoice #{invoice_id} is settled."),
        options: MessageOptions {
            priority: Some(SendPriority::High),
            image: Some(format!("https://cdn.example.test/invoices/{invoice_id}.png")),
            sound: Some(NotificationSound::Default),
            badge: Some(BadgeSetting::Unread),
            link: Some(format!("/invoice/{invoice_id}")),
            ..MessageOptions::default()
        },
        ..SendSpec::default()
    },
    target: SendTarget::Recipients(vec![alice_id]),
    validate_only: false,
})
.await?;
```

Options are part of the send's intent: the same idempotency key with different options is a conflict, exactly as a changed body would be. Each platform takes the options it understands:

| Option          | APNs (iOS, macOS)                                                  | FCM (Android)                                                                   | Web Push                                                                               |
| --------------- | ------------------------------------------------------------------ | ------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------- |
| `kind: "data"`  | background push, `content-available` alone, no banner              | data-only message, no notification block                                        | not sent: skipped as `platform_unsupported` (see below)                                |
| `priority`      | `apns-priority` 10 or 5                                            | `android.priority` high or normal                                               | `Urgency` header                                                                       |
| `image`         | not shown unless you add your own Notification Service Extension   | shown in the notification                                                       | shown in the notification                                                              |
| `sound`         | `aps.sound`, the named file                                        | `android.notification.sound`                                                    | `silent: true` on the notification (browsers play no sound by default)                 |
| `badge`         | `aps.badge`; on a data send, `lkbadge` in the payload              | `notification_count`; on a data send, `lkbadge`                                 | the receiver calls `navigator.setAppBadge`                                             |
| `link`          | carried in the payload for your app to route                       | carried in the data map                                                         | the receiver opens it on click (your origin only)                                      |
| `collapse_key`  | `apns-collapse-id`                                                 | `android.collapse_key`, and the notification's tag unless you set `android.tag` | `Topic` header, and the banner's tag                                                   |
| `push_ttl`      | `apns-expiration`                                                  | `android.ttl`                                                                   | `TTL` header                                                                           |
| `android { … }` | —                                                                  | channel (or a raw channel id), color, icon, tag, sticky                         | —                                                                                      |
| `apple { … }`   | subtitle, thread-id, category, interruption-level, relevance-score | mirrored into the APNs block FCM serves for iOS apps                            | —                                                                                      |
| `web { … }`     | —                                                                  | —                                                                               | icon, badge shape, up to two actions buttons, require-interaction, direction, language |

The per-platform blocks only shape that platform: `android.channel_id` names an Android channel, `apple.interruption_level` is what Focus modes read, and `web.actions` become `showNotification` action buttons — a click on one opens the same page a click on the banner does, with the button's id added to the address as `lka`. Everything is checked on both sides with errors that name the field — a color must be `#RRGGBB`, an image or link at most 1,024 bytes of `https` URL (or, for a link, a path starting with `/`), a sound name 1–64 bytes of letters, digits, `-`, `_`, and `.`. A data send shows nothing, so the options that only shape what is shown — `image`, `sound`, `link`, and the `android`, `apple`, and `web` blocks — are refused on one; its `priority` and `badge` are accepted.

**Android channels.** `android.channel_id` is the id you gave the [local-notifications plugin](inbox.md#composing-with-local-notifications) when your app configured the channel: an app that calls `configure_channel` with `inbox` receives a send whose `android.channel_id` is `"inbox"` in that channel. The plugin keeps every channel it creates under its own `leptoskit.notifications.` prefix on the device, and the service adds that prefix for you; a channel your own Kotlin creates is reachable only if its id starts with the same prefix. A push that names no channel, or names one the device doesn't have, is posted through your app's **default** channel instead. A push-enabled app has that channel from its first launch, listed as "Notifications" in the system's settings until your app configures the channel `default` with its own name and importance.

**A channel from before leptoskit.** An app that moved here from Firebase already has channels on people's phones under ids of its own, such as `orders`, and people have set their sound and importance. `android.raw_channel_id` names such a channel exactly as the device holds it, with no prefix added. Set it instead of `channel_id`; a send that sets both is refused. The service cannot tell whether the channel exists: your app still has to create it at launch as it always did, and a push naming a channel the device doesn't have goes through the default channel.

```rust
use leptoskit_notifications::AndroidOptions;
use leptoskit_notifications::MessageOptions;

let options = MessageOptions {
    android: Some(AndroidOptions {
        raw_channel_id: Some("orders".into()),
        ..AndroidOptions::default()
    }),
    ..MessageOptions::default()
};
```

**Apple permissions and entitlements.** On iOS and macOS a push's badge and sound take effect only where the person allowed them. `request_permission` asks for alerts, badges, and sounds in one question. The system asks once: someone who answered in an earlier build of your app, which asked for alerts only, may have badges and sounds switched off until they change that in Settings. `apple.interruption_level: "time-sensitive"` needs Apple's Time Sensitive Notifications capability, which leptoskit doesn't add to the app yet; Apple documents that without it the system treats the push as an ordinary (`active`) one, so it won't break through a Focus.

**When the payload is too big.** Every provider caps the message, and options count toward it: APNs takes 4 KiB of encoded JSON, FCM 4 KiB of the keys and values a device receives (the notification, the data, and the Android options, not the request around them), and a Web Push service must take a 4 KiB encrypted record. The longest title and body a send allows fit on every provider unless their text needs heavy JSON escaping, such as a body of quotation marks. When a send doesn't fit, the service drops the optional extras in the same fixed order on every provider before giving up on anything you'd miss: the per-platform blocks and the image first, then the sound, badge, and link, then the send's `data`, down to an id-only envelope that still shows the title and body and still collapses by the notification's id, or by your `collapse_key`. A data message keeps its silent shape in the fallback. If even the id-only envelope doesn't fit, the delivery ends as `rejected` with the reason `oversized`, without a request.

**What a push carries.** Besides the title and body, every push carries a small envelope your app can read. [Receiving on the device](receiving.md) hands it to you already parsed, so you need these names only when you write host code of your own:

| Key       | Holds                                          | APNs              | FCM                           | Web Push        |
| --------- | ---------------------------------------------- | ----------------- | ----------------------------- | --------------- |
| `lkn`     | the id of this person's inbox row              | top level         | data map                      | top level       |
| `lk`      | the send's `data`, as the JSON object you sent | top level         | not used: keys are flattened  | top level       |
| `lklink`  | `options.link`                                 | top level         | data map                      | top level       |
| `lkimg`   | `options.image`                                | top level         | not used: FCM shows the image | sent as `image` |
| `lkbadge` | the badge number                               | on a data message | on a data message, as text    | whenever set    |
| `lkw`     | the `web` block: icon, actions, and the rest   | not used          | not used                      | top level       |

On FCM your `data` object's keys sit directly in the data map beside the envelope's, and every value is text.

**The image on iOS and macOS.** APNs cannot show a remote image by itself. The service sets `mutable-content` and puts the address in `lkimg`, which is what a Notification Service Extension needs to download and attach it. leptoskit does not generate that extension, so until you add one to your Xcode project the push arrives with its text and no picture.

### Replace an earlier push, and let a push go stale

A live score changes every few minutes. Sending each change as a new notification leaves a phone that was in a tunnel with six banners, five of them wrong. Two options fix that, and neither touches the inbox.

```rust
use leptoskit_notifications::MessageOptions;

let options = MessageOptions {
    // The same key on a later send replaces this push.
    collapse_key: Some(format!("match-{match_id}")),
    // The providers drop it if the device stays unreachable for ten minutes.
    push_ttl: Some(600),
    ..MessageOptions::default()
};
```

**`collapse_key`** is a name you choose, 1 to 32 letters, digits, `-`, and `_`. A later push with the same key replaces an earlier one that a device has not received yet. It also replaces the banner already on screen on iOS and macOS, on Android (the service sends the key as the notification's tag unless you set `android.tag` yourself), and in browsers (the receiver shows the banner under the key). Without a key, a push collapses only with repeats of the same notification. Three things to know:

- The key is per app, not per person or per send. Use the same key only for pushes that should replace each other.
- FCM keeps at most four different collapse keys waiting for one offline device. With more, it keeps four and does not say which.
- The inbox is not collapsed. Each send still writes its own item. If only the latest should stay there too, [cancel](sending.md#cancel-a-scheduled-send) the earlier send.

**`push_ttl`** is how many seconds a provider may keep the push for a device it cannot reach, counted from the moment the notification becomes visible. The longest is 2,419,200 seconds (28 days). It is separate from `expires_at`, which removes the item from the inbox: a push worth ten minutes can belong to an inbox item that stays for a week. When a send sets both, the push stops at the earlier one. The service's own retries stop at the same moment, and a delivery that was still waiting then ends with the reason [`expired`](troubleshooting.md#reason-codes). Retries never go on for more than 24 hours, whatever the lifetime.

`push_ttl: Some(0)` means now or never: the service makes one attempt and asks the provider not to store the push at all. It is not retried, even when the provider asks for a retry.

Both options work on a [data message](#silent-data-messages) too. On Android, a push that arrives while your app is open is shown by the app, not the system, and that banner is not replaced by its key.

### Silent data messages

```rust
options: MessageOptions {
    kind: MessageKind::Data,
    ..MessageOptions::default()
},
```

`kind: "data"` makes the send a **background message**: no banner on any platform, with the send's `data` addressed to your app instead. Each platform gets the shape its provider requires:

- **iOS and macOS**: an APNs background push — `content-available` alone in `aps`, the background push type, priority 5 — with your `data` under `lk`, the notification's id as `lkn`, and the badge, when the send sets one, as `lkbadge`. The system wakes the app when it judges it can, and may hold or drop background pushes; Apple asks for no more than two or three an hour.
- **Android**: an FCM data message with no notification block of any kind — a message with one is shown in the tray instead of reaching the app — carrying your `data`'s keys, `lkn`, and `lkbadge` as strings in the data map.
- **Web**: not sent. Browsers require every push to show a notification — Chrome shows a generic one of its own, Safari revokes the site's push permission after pushes that show nothing, and Edge doesn't take them. The service skips a data send's Web Push deliveries as `skipped`, with the reason `platform_unsupported`; its deliveries to native devices go as usual.

Your app reads an arriving data message as `Received::Data` from the [receiver](receiving.md#know-what-was-tapped-and-what-arrived): the send's `data`, the id of the send's row for this person, and the badge the send asked for. It is handed over while your app is running. On iOS and macOS the system starts or wakes the app for a background push only when it chooses to, and a message that arrives while the app isn't running waits, among the sixteen most recent, until the app next asks. On Android the message is handed to the app's messaging service, which keeps it in memory: your code reads it if the app is open or opens before the system ends the process. A data message is a hint to refresh, not a guaranteed delivery: keep the thing that matters in your own service and let the message say "go and look".

A data send still creates one durable row per recipient — the send's bookkeeping, not an inbox item. It never appears in the inbox, never counts toward `unread_count`, and can always be cancelled, because nobody ever saw it. Its push deliveries, status, idempotency, and fanout behave exactly like a visible send's.

The options that only shape what is shown — `image`, `sound`, `link`, and the `android`, `apple`, and `web` blocks — are refused on a data send, with an error naming the field. Apple requires background pushes at priority 5, so a data send ignores a higher one there; Android honors it.

### Badges

```rust
options: MessageOptions {
    badge: Some(BadgeSetting::Unread),
    ..MessageOptions::default()
},
```

`badge` sets the app badge the platform shows next to your icon. `BadgeSetting::Set(n)` sets that exact number — `Set(0)` clears it. `BadgeSetting::Unread` is the interesting one: the service resolves it **per recipient at push time** to that person's live unread count — the same number `unread_count` would answer right then. One send to a thousand people badges each of their own counts.

On the web the receiver service worker calls `navigator.setAppBadge(n)` where the browser supports it (and `clearAppBadge` for zero); where it doesn't, the count rides in the payload and nothing else happens. A data send can carry a badge too, but there is no notification for the platform to badge from: the number rides as `lkbadge` in the data, for your app to apply itself: [`Received::Data`](receiving.md#know-what-was-tapped-and-what-arrived) carries it as `badge`, and your app sets it through its own host code (`UNUserNotificationCenter.setBadgeCount` on Apple platforms; on Android, launchers badge from the app's notifications).

## Say it in each person's language

Your app has people who read Portuguese, French, and English, and one announcement to make. You could keep three lists and make three sends, and then keep those lists right as people change their phone's language. Instead, make one send that carries each translation, and let the service give each person the one they read.

```rust
use leptoskit_notifications::SendSpec;
use leptoskit_notifications::SendTarget;
use leptoskit_notifications::Variant;

let sent = call_as_app(leptoskit_notifications::Send {
    idempotency_key: "spring-sale".into(),
    spec: SendSpec {
        category: "offers".into(),
        title: "The spring sale is on".into(),
        body: "Everything is 20% off until Sunday.".into(),
        variants: [
            (
                "pt-BR".to_owned(),
                Variant {
                    title: Some("A promoção de primavera começou".into()),
                    body: Some("Tudo com 20% de desconto até domingo.".into()),
                    ..Variant::default()
                },
            ),
            (
                "fr".to_owned(),
                Variant {
                    title: Some("Les soldes de printemps sont là".into()),
                    body: Some("Tout est à -20 % jusqu'à dimanche.".into()),
                    ..Variant::default()
                },
            ),
        ]
        .into(),
        ..SendSpec::default()
    },
    target: SendTarget::Topic("offers".into()),
    validate_only: false,
})
.await?;
```

The title and body of the spec are the send's default text. Each entry of `variants` is keyed by a language tag (`pt-BR`, `fr`, `zh-Hant`) and replaces the text for people who read that language. Everyone still gets exactly one inbox row. The row is written in their language, and the push for it carries the same words on every platform.

**Which language a person reads** is decided in this order:

1. The `locale` [attribute](audiences.md#describe-people-with-attributes) your server set for them, if there is one. Set it when your app has its own language setting: `set: [("locale".to_owned(), "pt-BR".to_owned())]`.
2. Otherwise the language of the device the service heard from most recently. Every enrolled device [reports its language](audiences.md#what-a-device-says-about-itself) with no code from you.
3. Otherwise nothing is known, and they read the default text.

**Which variant that language gets:** the variant with exactly that tag, in any letter case. If there is none, the tag is shortened one part at a time, so `pt-BR` falls back to a `pt` variant and `zh-Hant-TW` tries `zh-Hant`, then `zh`. If nothing matches, the person reads the default text. A match never goes the other way: someone whose language is plain `pt` does not get a `pt-BR` variant, so add a `pt` variant if you want one text for all Portuguese readers.

Your server's attribute wins even when it matches no variant. A person you marked `ja` reads the default text, not the French their browser reports.

A variant only needs the fields it changes. Leave `body` out and that language keeps the default body. Besides `title` and `body`, a variant can translate two more pieces of text, when the send has them:

- `subtitle` replaces `options.apple.subtitle`.
- `actions` replaces the titles of `options.web.actions`, by action id: `actions: [("open".to_owned(), "Ouvrir".to_owned())].into()`.

The limits, all checked before anything is sent and refused with the variant named:

- At most 32 variants in one send.
- Each key is a BCP 47 language tag, and no two keys may be the same tag in different spelling (`en-GB` and `en_gb`).
- A variant must change something. Its title and body have the same length limits as the send's own.
- A `subtitle` or an `actions` entry is accepted only when the send itself sets that subtitle or names that action.
- The whole spec, variants included, must fit in 8 KiB as stored JSON. Five 2,000-byte bodies do not fit, so long texts mean fewer variants.
- A [silent data message](#silent-data-messages) shows nothing, so it cannot carry variants.

The language is chosen once, when the send writes a person's row. Someone who changes language afterwards keeps the row they have. Variants are part of what the send is: repeating the send with the same key and the same variants answers the first result, and changing a translation under the same key is refused as a different send. [`send_test`](troubleshooting.md#test-on-a-device) pushes the variant the person you name would read, and `validate_only` checks the variants without sending.

## Preview a push

A notification that is too big is not refused. Each provider has a size limit near four kilobytes, and the service trims a push that is over it until it fits: first the per-platform options and the image, then the sound, badge, and link, then the send's `data`. The first you hear of it is a notification that arrives without its picture. `preview_push` shows you before you send:

```rust
use leptoskit_notifications::preview::{PreviewPush, PreviewSample};

let previews = call_as_app(PreviewPush {
    spec: spec.clone(),
    sample: PreviewSample::default(),
})
.await?;

for preview in &previews {
    println!(
        "{:?}: {} of {} bytes, trimmed {:?}, fits: {}",
        preview.platform, preview.bytes, preview.limit, preview.trimmed, preview.fits,
    );
}
```

It answers one `PushPreview` each for APNs, FCM, and Web Push, in that order:

- `headers` and `body`: the request the provider would be sent. The body is built by the same code that builds a real push, so it is the same bytes.
- `bytes` and `limit`: what the provider counts against its limit, and the limit. Apple counts the body. FCM counts the keys and values a device receives. For Web Push it is the size after encryption.
- `trimmed`: what was removed to make it fit, in order: `platform_and_image`, `sound_badge_link`, `data`. Empty when everything fits.
- `fits`: `false` when even the smallest form is too big. Nothing would be sent on that platform, and the delivery would end `oversized`.
- `skipped`: set when the platform is never sent this kind of send. A silent data message is not pushed to browsers.

A preview cannot know three things only a real delivery knows, so it uses stand-ins. The notification's id is the all-zero id unless you pass one in `PreviewSample`. The FCM body shows `<device token>` where the device's token goes. A badge set to the unread count shows 1 unless you pass `unread`. Pass `locale` to see the text a person with that language would get from the send's [variants](#say-it-in-each-persons-language).

The preview leaves out the authorization header and the APNs topic, reads no credential, and works on a server with no push configured. The Web Push body is shown before encryption, because the encrypted bytes are different for every push. A spec that `send` would refuse is refused here the same way. Only your app's server can ask for a preview.
