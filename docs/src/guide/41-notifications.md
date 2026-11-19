# Notifications

You want to tell someone something after they have left the screen: an invoice was paid, a ride is arriving, someone replied. The usual route hands your users and your sending patterns to a third-party push platform, and binds delivery to OS credentials you have to enrol for. leptoskit has a notifications **service** you add with one table in `leptoskit.toml`: a small program with its own database that runs beside your app's server, stores one durable notification per recipient, and serves each person their own inbox. Your server sends to explicit recipients, to a topic's subscribers, to the people a condition matches over topics, the attributes you set, and the devices people enrolled — their platform, language, time zone, or app version — or to everyone the service knows, and when you configure a push transport, the same send also wakes the devices its recipients enrolled — APNs, FCM, and Web Push, first-party, with your app's own credentials and no management vendor in between. Each send can be shaped — priority, an image, a sound, a badge, a click link, per-platform overrides, or a silent data message — and one call tests a spec on a real device before a campaign.

This page shows how to add it, send from your server, read the inbox on every platform, manage categories and schedules, and stand up push delivery. It also says plainly what it does not do yet: **client enrollment beyond the web, iOS, Android, and macOS**, a record of who opened what, and any claim that a provider's acceptance means a person saw a banner.

## Add the service

Name it in your app's `leptoskit.toml`:

```toml
[services.notifications]
crate = "leptoskit_notifications"
version = "0.3"
trust = "first-party"
```

That's all `leptoskit dev` and `leptoskit serve` need: they build the service, give it a database of its own on the same engine as your app's (SQLite, or Postgres when your app uses Postgres — add `features = ["service", "postgres"]` for that), start it beside your app, and answer for it at `/_leptoskit/services/notifications/` on your app's own address. [Services](32-services.md) explains how services run and deploy.

The `trust` line matters because of what it defaults to: a `package` in your workspace is your own code and defaults to `first-party`, but a `crate` from the registry — like the snippet above — is someone else's, so it defaults to `isolated`, and an isolated service is not told who is signed in. The notifications service is first-party by intent: it reads your users' inboxes, so the first copy-paste setup says so explicitly. `forward_identity` then defaults to on for first-party services; if you prefer to name it, the whole table is:

```toml
[services.notifications]
crate = "leptoskit_notifications"
version = "0.3"
trust = "first-party"
forward_identity = true
```

Reading the inbox needs a signed-in person, which needs `forward_identity = true` on the service's table, your app's server serving `rpc::server::Server` with your sign-in as its `Identify` (which the scaffold already does), and, in a native app, `client::configure` with your credentials. Nothing is forwarded but the person's id: not their session, email, or roles. [Knowing who is signed in](32-services.md#knowing-who-is-signed-in) has the details.

## Send from your server

Only your app's server sends. Add the crate and call `send` with `call_as_app`, which presents the service's token (that's [`call_as_app`](32-services.md#calling-as-the-app): it needs the `services` RPC feature on your server, which `leptoskit`'s `server` feature turns on):

```toml
[dependencies]
leptoskit_notifications = "0.3"
```

```rust
use leptoskit_notifications::SendSpec;
use leptoskit_notifications::SendTarget;
use leptoskit_rpc::client::call_as_app;

let sent = call_as_app(leptoskit_notifications::Send {
    idempotency_key: format!("invoice-paid-{invoice_id}"),
    spec: SendSpec {
        category: "billing".into(),
        title: "Invoice paid".into(),
        body: format!("Invoice #{invoice_id} is settled. Thank you!"),
        data: Some(serde_json::json!({ "screen": "invoice", "id": invoice_id }).to_string()),
        visible_at: None,
        expires_at: None,
    },
    target: SendTarget::Recipients(vec![alice_id, bob_id]),
    validate_only: false,
})
.await?;
```

Every named recipient gets one durable row of their own, visible immediately — or at `visible_at`, when you schedule one — until `expires_at`. The counts come back: `delivered`, `skipped_opted_out` (recipients who opted out of the category), and `recipients`. A recipients send is one transaction, so it lands all at once or not at all, and answers with its final counts.

The `idempotency_key` is how retries stay safe: derive one stable key per logical send (like the invoice id above) and reuse it on retry. The same key with the same intent answers the first run's result and writes nothing — across restarts, and past the send's expiry, because the send's bookkeeping lives in the service's database — and the same key with a different intent is refused with a conflict, so a reused key can't quietly send something else. The key is remembered for as long as the send's record is kept, which is [at least 90 days by default](#how-long-things-are-kept).

`SendSpec::visible_at` and `expires_at` take RFC 3339 instants, at most a year ahead; a `visible_at` in the past means now, so the notification shows from the moment it is sent and its push has its full delivery window from then. `data` is a JSON object of at most 4 KiB your screens read to route a tap; everything is length-checked on both sides, so a screen can reject bad input without a round trip. The whole spec is bounded as the service stores it, too: at most 8 KiB as JSON, with its options at most 4 KiB of that, so a spec that passes the checks is always one the service can keep, whatever its target.

### Cancel a scheduled send

A scheduled send can be cancelled until it becomes visible:

```rust
let cancelled: u32 = call_as_app(leptoskit_notifications::Cancel {
    idempotency_key: "launch-reminder".into(),
})
.await?;
```

After it's due the service refuses with a conflict — what someone has already seen can't be unsent. (A send in [each person's local time](#send-at-each-persons-local-time) is due at a different moment for each person, and can be cancelled for the people still waiting.) Cancelling again answers `0`, and so does cancelling a send every recipient opted out of: it delivered nothing, but the service knows the key, and cancelling it is still cancelling it.

A send to a topic, a condition, or everyone can have far more rows than one write should touch. Cancelling one with more than 500 answers how many it stops, stops the send's pushes at once, and cancels the rows themselves in the background, a batch at a time; asked again before they are all done, it answers how many are left.

## Send to a topic

Naming every recipient stops scaling the moment your audience chooses itself: everyone who pressed "follow" wants the next post, and your server shouldn't have to keep that list in memory to reach it. A topic is the list: the app's server subscribes people in batches, and a send to the topic reaches everyone subscribed, however many that is.

```rust
use leptoskit_rpc::client::call_as_app;

// Whenever someone follows, in batches of at most 1,000:
let changes = call_as_app(leptoskit_notifications::SubscribeToTopic {
    topic: "posts.alice".into(),
    persons: vec![follower_id],
})
.await?;

// And the send itself:
let sent = call_as_app(leptoskit_notifications::Send {
    idempotency_key: format!("post-{post_id}"),
    spec: SendSpec {
        category: "social".into(),
        title: "Alice posted".into(),
        body: preview.clone(),
        data: Some(serde_json::json!({ "screen": "post", "id": post_id }).to_string()),
        visible_at: None,
        expires_at: None,
    },
    target: SendTarget::Topic("posts.alice".into()),
    validate_only: false,
})
.await?;
```

A topic names 1 to 128 bytes of the characters FCM accepts — letters, digits, `-`, `.`, `_`, `~`, `%` — and a person holds at most 500 of them. Batching is idempotent: people who already hold the topic change nothing and count nothing, and people already at their 500 are skipped and counted in the answer rather than refusing the whole batch. Topics belong to people, not devices: every device signed in as the person receives what the person receives, the same way the inbox does.

Because the audience can be any size, the send doesn't hold it in one transaction. `Send` records the send and enqueues a fanout job, then answers immediately with state `fanning_out`. The job walks the audience in batches of 500, ordered by person id, writing each batch's rows — and its push deliveries, and the live updates open screens follow — exactly as a recipients send does: opt-outs skipped and counted, schedules and expiry honored. The audience is snapshotted as the fanout reaches each person, so someone who subscribes after it passed them by waits for the next send. A crashed or replayed batch never duplicates: the send holds one row per person by a database constraint.

Unsubscribe the same way — `UnsubscribeFromTopic { topic, persons }` in batches — and `send_status` (below) follows a large send's progress.

## Let people subscribe themselves

Your server can subscribe anyone; a person pressing a switch in your app should only be able to join the audiences you published as joinable. That boundary is configuration: topics are open to self-subscription only when the operator declares them open, in the service's environment — patterns like `news.*`:

```bash
LEPTOSKIT_NOTIFICATIONS_OPEN_TOPICS=news.*, inbox
```

Nothing is open by default, and a pattern that can't match a topic name refuses to start, named. `*` stands for any run of topic characters: `news.*` matches `news.breaking` but not `news`. The reason for the gate is whose choice a subscription is — your server follows your product's logic, while a person's own call should only reach the topics you said they could reach.

The person's own calls need no list and name no one but themselves — the person is read from the app's forwarded identity, never from an argument, so nobody can subscribe anyone else:

```rust
leptoskit_rpc::call(leptoskit_notifications::Subscribe { topic: "news.breaking".into() }).await?;
leptoskit_rpc::call(leptoskit_notifications::Unsubscribe { topic: "news.breaking".into() }).await?;
```

A closed topic is refused with a clear forbidden error; a bad topic name is invalid. Unsubscribing is always allowed — leaving an audience is the person's to do, whatever subscribed them — and `my_topics` is their own list, live and paginated, so a screen follows the app's batch subscriptions and the person's own joins and leaves alike. A follow switch is the whole client half:

```rust
use leptoskit::prelude::*;
use leptoskit_rpc::call;
use leptoskit_rpc::use_live_paginated;

#[component]
pub fn BreakingNews() -> impl IntoView {
    let topics = use_live_paginated(
        move |page| leptoskit_notifications::MyTopics { page },
        20,
    );
    let following = move || {
        topics
            .items()
            .iter()
            .any(|topic| topic == "news.breaking")
    };
    let toggle = move || {
        leptos::task::spawn_local(async move {
            let _ = if following() {
                call(leptoskit_notifications::Unsubscribe {
                    topic: "news.breaking".into(),
                })
                .await
            } else {
                call(leptoskit_notifications::Subscribe {
                    topic: "news.breaking".into(),
                })
                .await
            };
        });
    };

    view! {
        <Button on_press=toggle>
            {move || if following() { "Stop following" } else { "Follow breaking news" }}
        </Button>
    }
}
```

## Target with conditions

A topic is one audience; a product usually wants the overlap — everyone in `news.sports` who is also in `alerts.local`, or in one and not the other. A condition writes that, in the same grammar Firebase Cloud Messaging uses, so a migration is a paste:

```text
'news.sports' in topics && 'alerts.local' in topics
'TopicA' in topics && ('TopicB' in topics || !('TopicC' in topics))
```

```rust
let sent = call_as_app(leptoskit_notifications::Send {
    idempotency_key: "game-42-final".into(),
    spec: SendSpec {
        category: "alerts".into(),
        title: "Final score".into(),
        body: format!("{home} {home_score} – {away_score} {away}"),
        data: None,
        visible_at: None,
        expires_at: None,
    },
    target: SendTarget::Condition(
        "'news.sports' in topics && 'alerts.local' in topics".into(),
    ),
    validate_only: false,
})
.await?;
```

The grammar is `&&`, `||`, `!`, and parentheses over the predicates below — and the `device.` predicates in [Target devices](#target-devices) — with `!` binding tightest, then `&&`, then `||`. At most five distinct topics, at most ten attribute and device predicates together, nesting at most eight levels deep, and at most 1,024 bytes — FCM's bounds, checked on both sides, with errors that name the byte they gave up at.

| Predicate                            | True when                                                   |
| ------------------------------------ | ----------------------------------------------------------- |
| `'name' in topics`                   | the person holds the topic                                  |
| `user.<key> == 'value'`              | the app set the attribute to exactly that value             |
| `user.<key> != 'value'`              | the app set the attribute to something else                 |
| `user.<key> in ['a', 'b']`           | the attribute is one of the listed values                   |
| `has(user.<key>)`                    | the app set the attribute, whatever its value               |
| `user.<key> < '10'` (also `<= > >=`) | both sides parse as finite numbers and the comparison holds |

An attribute the person doesn't have answers false to every predicate, `!=` and `in` included: nobody matches a segment merely for lacking the attribute. Numeric comparisons are numbers, not text — `user.version > '9'` is true for `10` — and a value that doesn't parse as a finite number answers false; `NaN` and `inf` are refused as literals at parse time. Keys are `a-z`, `0-9`, and `_`, up to 64 bytes, and values are at most 256 bytes with no control characters — the same rules `set_person_attributes` enforces, so what you can send is what you can ask about.

**Values with spaces and quotes.** Real attribute values aren't always one word: a city is `New York`, a name is `O'Brien`. Everything between a value's quotes is the value, spaces included, and a quote inside a value is written twice:

```text
user.city == 'New York'
user.surname == 'O''Brien'
user.city in ['New York', 'Coeur d''Alene']
```

That doubled quote is the only escape there is; a backslash is just a backslash. When a value comes from somewhere you don't control, don't paste it between quotes yourself — `condition::quoted` writes it correctly, so a stray quote in someone's name can't end the value early and change who the send reaches:

```rust
use leptoskit_notifications::condition::quoted;

let target = SendTarget::Condition(format!(
    "user.city == {} && 'news.local' in topics",
    quoted(&city), // 'Coeur d''Alene'
));
```

Topic names need none of this — they can hold neither a space nor a quote — so a condition pasted from Firebase means exactly what it meant there. Parentheses and `!` can be opened at most 32 deep in the text, however little they group.

One shape is refused outright: a condition that already holds for a person with no topics, no attributes, and no device, like `!('a' in topics)`, is an "everyone" audience in disguise — the plain target below says that with a name, so the condition grammar makes you use it. Everything else anchors on something: a topic, or an attribute predicate such as `user.plan == 'pro'`. Condition sends fan out like topic sends: same batches, same snapshot-as-reached, same idempotency. Which index a condition's fanout walks follows from its anchor — a condition that cannot match without one of its topics walks the topic's subscribers, and one anchored only on attributes walks the person registry below.

## Describe people with attributes

Topics say what a person asked for; attributes say who they are. Your server describes each person with a handful of key-value pairs — the plan they're on, the country they're in, the campaign that brought them — and conditions read them as `user.<key>`. Only your server can set them, and only your server can read them back: an attribute is your statement about a person, and a segment built from client-set attributes would be an audience anyone could join by editing a request.

```rust
use leptoskit_rpc::client::call_as_app;

// When a plan changes, in batches of at most 1,000 people:
let changes = call_as_app(leptoskit_notifications::SetPersonAttributes {
    persons: vec![alice_id, bob_id],
    set: [
        ("plan".to_owned(), "pro".to_owned()),
        ("country".to_owned(), "de".to_owned()),
    ]
    .into(),
    remove: vec!["trial_lapsed".to_owned()],
})
.await?;

// What a segment will see, keyed and sorted:
let attributes = call_as_app(leptoskit_notifications::PersonAttributes {
    person: alice_id,
})
.await?;
```

Keys are 1 to 64 bytes of `a-z`, `0-9`, and `_`; values are strings of at most 256 bytes; a person holds at most 50 attributes, and a key past that bound is skipped and counted in `skipped_full` rather than refusing the batch. The call is idempotent — sending the same value again changes nothing and counts nothing — and `remove` is the last word: a key in both `set` and `remove` is removed, never set. One call names at most 100 keys across `set` and `remove`; a larger change is refused as a bad request, so split it into several calls.

Two keys mean something to the service itself: `locale` is the language a person reads [translated sends](#say-it-in-each-persons-language) in, and `time_zone` is where they are for [local-time sends](#send-at-each-persons-local-time). Both are ordinary attributes that conditions can read too. Set them when your app knows better than the person's device does.

Setting an attribute also registers the person with the service, because an attribute is one of the things that makes someone an audience: the registry the service keeps is who "everyone" means. Someone who subscribes to a topic, sets a category preference, or enrolls a push endpoint is registered too — including by the migration that adds the registry, which backfills it from those tables. Someone who only ever received a recipients send is not registered: naming people in a send never puts them in an audience.

### Erase a person

When someone deletes their account or withdraws consent, you need the service to hold nothing more about them — not only to stop sending. `ForgetPerson` erases everything it has for one person:

```rust
call_as_app(leptoskit_notifications::ForgetPerson { person: alice_id }).await?;
```

That removes their inbox, their enrolled devices and the push credentials stored for them, the record of what was pushed to those devices, their category preferences, their topic subscriptions, their attributes, and their place in the registry. Nothing more is pushed to their devices, no audience reaches them, and their inbox is empty. It is safe to call twice, and for someone the service never knew.

A long inbox is erased in the background, a batch at a time, so the call returns quickly whatever the person's history; the rest follows within moments. A send's own totals are not rewritten — "delivered to 1,204 people" stays 1,204.

It stays in effect. Their phone or browser learns at its next launch that its enrollment is gone and turns notifications off there, instead of enrolling again ([what happens at launch](#when-the-server-revokes-a-device)). The service hears of the person again only if something new happens: they turn notifications on, set a preference, or subscribe to a topic themselves, or your server names them in a send, a subscription, or an attribute call. If someone must stay out — a withdrawn consent, rather than a deleted account nobody can sign in to — stop offering them those switches and stop naming them; the service keeps no list of people it was told to forget, because that list would be data about them too.

## Target a segment

Attributes and topics mix freely in one condition, which is the whole point: a segment is "people like that, who also asked for this".

```rust
let sent = call_as_app(leptoskit_notifications::Send {
    idempotency_key: format!("pro-de-launch-{launch_id}"),
    spec: SendSpec {
        category: "product".into(),
        title: "New in your plan".into(),
        body: "The launch post, in your language.".into(),
        data: None,
        visible_at: None,
        expires_at: None,
    },
    target: SendTarget::Condition(
        "user.plan == 'pro' && user.country in ['de', 'at'] && 'news.product' in topics"
            .into(),
    ),
    validate_only: false,
})
.await?;
```

```text
user.plan == 'pro' && !('beta' in topics)
user.spend >= '1000' || has(user.enterprise)
user.country != 'us' && 'news.product' in topics
```

The second example is a condition anchored only on attributes: it can match a person who holds no topics at all, so its fanout walks the person registry instead of a topic's subscribers — every registered person is a candidate, evaluated against the attributes your server set. The first and third name a topic their match depends on, so they walk that topic's subscribers and cost no more than a topic send. You don't choose the walk; the condition's shape does, and `validate_only` counts either way:

```rust
let dry = call_as_app(leptoskit_notifications::Send {
    idempotency_key: "pro-de-launch-dry".into(),
    spec,
    target: SendTarget::Condition("user.plan == 'pro'".into()),
    validate_only: true,
})
.await?;
```

The audience is snapshotted as the fanout reaches each person, so change an attribute mid-fanout and the people the walk already passed keep the value it saw.

## Send to everyone

Some sends really are for everyone: a terms change, a service window, a new version shipped. `SendTarget::Everyone` reaches every person in the service's registry — the people whose attributes your server set, who subscribed to a topic, set a category preference, or enrolled a device.

```rust
let count = call_as_app(leptoskit_notifications::Send {
    idempotency_key: format!("terms-v{n}"),
    spec: SendSpec {
        category: "account".into(),
        title: "Terms of service changed".into(),
        body: "The new terms take effect next month.".into(),
        data: None,
        visible_at: None,
        expires_at: None,
    },
    target: SendTarget::Everyone,
    validate_only: true,
})
.await?;
```

Treat this target with the respect a big lever deserves. It fans out in the same batches as any large send, so the service stays healthy however many people it knows — but every registered person gets a row, and every enrolled device gets a push, in one send. Two courtesies are built in: people who opted out of the send's category are skipped and counted, and `validate_only` (as above) answers how many people the send would reach right now, before anything is written. `forget_person` is the other half of this: a person your server forgot is not part of "everyone" anymore, whatever their history.

Everyone is not a condition, and it can't be narrowed: `SendTarget::Everyone` is the whole registry, every time. When you want most people, say which ones — a condition over attributes — because a send to everyone is a promise to every person in it.

## Track a large send

A recipients send answers with its final counts because it finished inside the call. A topic, condition, or everyone send answers `fanning_out` and keeps working — so ask it how it's going, and try the whole thing without spending it:

```rust
// How the fanout is going, from your server:
let progress = call_as_app(leptoskit_notifications::SendStatus {
    idempotency_key: "post-1234".into(),
})
.await?;

// What the send would reach right now, writing nothing:
let dry = call_as_app(leptoskit_notifications::Send {
    idempotency_key: "post-1235".into(),
    spec,
    target: SendTarget::Topic("posts.alice".into()),
    validate_only: true,
})
.await?;
```

`send_status` answers the state — `fanning_out`, `complete`, or `cancelled` — and the counts so far: `targeted` (everyone the target matched), `delivered`, and `skipped_opted_out`. Once the state is `complete` the counts are final; `validate_only` answers the same shape for the audience as it stands now, without writing a row, the way FCM's `validate_only` does, for every target — recipients, topic, condition, and everyone. A dry run reads the audience a page at a time and holds nothing open while it does, so counting everyone the service knows doesn't hold up its other work. Cancelling works mid-fanout too: the fanout stops before its next batch, the rows written so far are cancelled, and the send answers `cancelled` from then on.

## Shape a message

A plain send shows a banner with its title and body everywhere. Most of the time that's right, and then there are the others: the message that shouldn't wake anyone, the one that should carry a picture, the one whose whole job is to correct the badge count. `SendSpec` takes an `options` object for those, and every field is optional — a send without options behaves exactly as sends always did, byte for byte on the wire.

```rust
let sent = call_as_app(leptoskit_notifications::Send {
    idempotency_key: format!("invoice-paid-{invoice_id}"),
    spec: SendSpec {
        category: "billing".into(),
        title: "Invoice paid".into(),
        body: format!("Invoice #{invoice_id} is settled."),
        data: None,
        visible_at: None,
        expires_at: None,
        options: MessageOptions {
            priority: Some(SendPriority::High),
            image: Some(format!("https://cdn.example.test/invoices/{invoice_id}.png")),
            sound: Some(NotificationSound::Default),
            badge: Some(BadgeSetting::Unread),
            link: Some(format!("/invoice/{invoice_id}")),
            ..MessageOptions::default()
        },
    },
    target: SendTarget::Recipients(vec![alice_id]),
    validate_only: false,
})
.await?;
```

Options are part of the send's intent: the same idempotency key with different options is a conflict, exactly as a changed body would be. Each platform takes the options it understands:

| Option          | APNs (iOS, macOS)                                                  | FCM (Android)                                        | Web Push                                                                               |
| --------------- | ------------------------------------------------------------------ | ---------------------------------------------------- | -------------------------------------------------------------------------------------- |
| `kind: "data"`  | background push, `content-available` alone, no banner              | data-only message, no notification block             | not sent: skipped as `platform_unsupported` (see below)                                |
| `priority`      | `apns-priority` 10 or 5                                            | `android.priority` high or normal                    | `Urgency` header                                                                       |
| `image`         | needs a Notification Service Extension in your app                 | shown in the notification                            | shown in the notification                                                              |
| `sound`         | `aps.sound`, the named file                                        | `android.notification.sound`                         | `silent: true` on the notification (browsers play no sound by default)                 |
| `badge`         | `aps.badge`; on a data send, `lkbadge` in the payload              | `notification_count`; on a data send, `lkbadge`      | the receiver calls `navigator.setAppBadge`                                             |
| `link`          | carried in the payload for your app to route                       | carried in the data map                              | the receiver opens it on click (your origin only)                                      |
| `android { … }` | —                                                                  | channel, color, icon, tag, sticky                    | —                                                                                      |
| `apple { … }`   | subtitle, thread-id, category, interruption-level, relevance-score | mirrored into the APNs block FCM serves for iOS apps | —                                                                                      |
| `web { … }`     | —                                                                  | —                                                    | icon, badge shape, up to two actions buttons, require-interaction, direction, language |

The per-platform blocks only shape that platform: `android.channel_id` names an Android channel, `apple.interruption_level` is what Focus modes read, and `web.actions` become `showNotification` action buttons — a click on one opens the same page a click on the banner does, with the button's id added to the address as `lka`. Everything is checked on both sides with errors that name the field — a color must be `#RRGGBB`, an image or link at most 1,024 bytes of `https` URL (or, for a link, a path starting with `/`), a sound name 1–64 bytes of letters, digits, `-`, `_`, and `.`. A data send shows nothing, so the options that only shape what is shown — `image`, `sound`, `link`, and the `android`, `apple`, and `web` blocks — are refused on one; its `priority` and `badge` are accepted.

**Android channels.** `android.channel_id` is the id you gave the [local-notifications plugin](#composing-with-local-notifications) when your app configured the channel: an app that calls `configure_channel` with `inbox` receives a send whose `android.channel_id` is `"inbox"` in that channel. The plugin keeps every channel it creates under its own `leptoskit.notifications.` prefix on the device, and the service adds that prefix for you; a channel your own Kotlin creates is reachable only if its id starts with the same prefix. A push that names no channel, or names one the device doesn't have, is posted through your app's **default** channel instead. A push-enabled app has that channel from its first launch, listed as "Notifications" in the system's settings until your app configures the channel `default` with its own name and importance.

**Apple permissions and entitlements.** On iOS and macOS a push's badge and sound take effect only where the person allowed them. `request_permission` asks for alerts, badges, and sounds in one question. The system asks once: someone who answered in an earlier build of your app, which asked for alerts only, may have badges and sounds switched off until they change that in Settings. `apple.interruption_level: "time-sensitive"` needs Apple's Time Sensitive Notifications capability, which leptoskit doesn't add to the app yet; Apple documents that without it the system treats the push as an ordinary (`active`) one, so it won't break through a Focus.

**When the payload is too big.** Every provider caps the message, and options count toward it: APNs takes 4 KiB of encoded JSON, FCM 4 KiB of the keys and values a device receives (the notification, the data, and the Android options, not the request around them), and a Web Push service must take a 4 KiB encrypted record. The longest title and body a send allows fit on every provider unless their text needs heavy JSON escaping, such as a body of quotation marks. When a send doesn't fit, the service drops the optional extras in the same fixed order on every provider before giving up on anything you'd miss: the per-platform blocks and the image first, then the sound, badge, and link, then the send's `data`, down to an id-only envelope that still shows the title and body and still collapses by the notification's id. A data message keeps its silent shape in the fallback. If even the id-only envelope doesn't fit, the delivery ends as `rejected` with the reason `oversized`, without a request.

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

Your app reads an arriving data message as `Received::Data` from the [receiver](#know-what-was-tapped-and-what-arrived): the send's `data`, the id of the send's row for this person, and the badge the send asked for. It is handed over while your app is running. On iOS and macOS the system starts or wakes the app for a background push only when it chooses to, and a message that arrives while the app isn't running waits, among the sixteen most recent, until the app next asks. On Android the message is handed to the app's messaging service, which keeps it in memory: your code reads it if the app is open or opens before the system ends the process. A data message is a hint to refresh, not a guaranteed delivery: keep the thing that matters in your own service and let the message say "go and look".

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

On the web the receiver service worker calls `navigator.setAppBadge(n)` where the browser supports it (and `clearAppBadge` for zero); where it doesn't, the count rides in the payload and nothing else happens. A data send can carry a badge too, but there is no notification for the platform to badge from: the number rides as `lkbadge` in the data, for your app to apply itself: [`Received::Data`](#know-what-was-tapped-and-what-arrived) carries it as `badge`, and your app sets it through its own host code (`UNUserNotificationCenter.setBadgeCount` on Apple platforms; on Android, launchers badge from the app's notifications).

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

1. The `locale` [attribute](#describe-people-with-attributes) your server set for them, if there is one. Set it when your app has its own language setting: `set: [("locale".to_owned(), "pt-BR".to_owned())]`.
2. Otherwise the language of the device the service heard from most recently. Every enrolled device [reports its language](#what-a-device-says-about-itself) with no code from you.
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

The language is chosen once, when the send writes a person's row. Someone who changes language afterwards keeps the row they have. Variants are part of what the send is: repeating the send with the same key and the same variants answers the first result, and changing a translation under the same key is refused as a different send. [`send_test`](#test-on-a-device) pushes the variant the person you name would read, and `validate_only` checks the variants without sending.

## Send at each person's local time

"Your weekly summary is ready" belongs at nine in the morning. But nine in the morning in Sydney is the middle of the night in Lisbon, and a send scheduled with `visible_at` goes to everyone at the same instant. Give the send a time on the clock instead, and each person gets it when their own clock shows that time.

```rust
use leptoskit_notifications::LocalTime;
use leptoskit_notifications::SendSpec;
use leptoskit_notifications::SendTarget;

let sent = call_as_app(leptoskit_notifications::Send {
    idempotency_key: "weekly-summary-2026-49".into(),
    spec: SendSpec {
        category: "summaries".into(),
        title: "Your weekly summary is ready".into(),
        local_time: Some(LocalTime {
            at: "2026-12-01T09:00".into(),
            default_time_zone: Some("Europe/London".into()),
        }),
        ..SendSpec::default()
    },
    target: SendTarget::Everyone,
    validate_only: false,
})
.await?;
```

`at` is a date and a time with no offset, written `2026-12-01T09:00` or `2026-12-01T09:00:00`. The service writes everyone's inbox row straight away, as it does for any scheduled send, and sets each row to appear when that time arrives where the person is. The push goes out at the same moment. A send to the whole world therefore arrives over about a day: first in the Pacific, last in the zones twelve hours behind UTC.

**Which time zone a person is in** is decided the same way as their language:

1. The `time_zone` attribute your server set for them, such as `America/Sao_Paulo`.
2. Otherwise the time zone of the device the service heard from most recently.
3. Otherwise `default_time_zone` from the send. If the send names none, that is UTC.

Time zones are IANA names like `Europe/London`. The service looks them up in the time zone data it was built with. A name it does not know, on an attribute or on a device, is skipped and the next step is used, so one odd value never fails a send. The send's own `default_time_zone` is different: an unknown name there is refused, because it is your typing mistake to fix.

A few cases worth knowing:

- **The time has already passed for someone.** If it is already past 09:00 on 1 December where a person is when the send reaches them, they get it at once. A time that has passed everywhere is the same as sending now.
- **Clocks that jump forward.** If the time falls in the hour that a daylight-saving change skips (02:30 on a night when clocks go from 02:00 to 03:00), it arrives just after the jump, at 03:30.
- **Clocks that go back.** If the time happens twice, it arrives the first time.
- **Expiry.** `expires_at` is still one instant for everyone. It must be later than the moment the local time reaches the last time zone, which is the time in `at` plus twelve hours, read as UTC. For `2026-12-01T09:00` that is `2026-12-01T21:00:00Z`. An earlier expiry is refused, with that instant in the message, because someone's notification would expire before it ever showed.
- **How far ahead.** The year's limit on scheduling is counted to the same last-zone moment.
- **`visible_at`.** A send has one schedule. Setting both `visible_at` and `local_time` is refused.

The time zone is chosen once, when the send writes a person's row. Someone who travels afterwards still gets it at the time worked out for where they were.

`send_status` counts a row as `delivered` when it is written, not when it appears, so a local-time send to a topic reads `complete` once every row exists, while some of them are still waiting for their morning. `validate_only` checks the time and the default zone and counts the audience. [`send_test`](#test-on-a-device) ignores `local_time` and pushes at once.

**Cancelling** works while anyone is still waiting. Once the first zone has seen the notification, a cancel stops it for everyone whose time has not come yet and leaves the rest alone; the answer is how many it stopped. Cancelling again answers `0`. When nobody is waiting any more, the cancel is refused with a conflict, as for any send people have already seen. For a topic, condition, or everyone send with more than 500 rows still waiting, the rows are stopped in the background, a batch at a time. A row whose time comes in the few seconds before its batch is reached can show and be pushed before it is withdrawn.

Variants and local time work together: one send can say good morning in each person's language at each person's nine o'clock.

## Read the inbox on any platform

The signed-in person reads their own inbox, newest first, one bounded page at a time, from the web, iOS, Android, macOS, Windows, Linux, the terminal, or (experimentally) HarmonyOS — wherever your screens run. Each page is a [live query](24-real-time.md), and what reaches an open screen is everything that changes its answer: a send that reaches this person, a read or dismissal from their other device, a category they opt out of, a scheduled send becoming due, and an expiring one leaving. The service runs the wake jobs that make time visible: when a send's schedule turns, the screens it reaches re-run with a fresh clock, so a scheduled item appears when its time comes and an expired one goes when its time is up — on every platform the live connection reaches.

```rust
use leptoskit_notifications::InboxShow;
use leptoskit_rpc::use_live_paginated;

let inbox = use_live_paginated(
	move |page| leptoskit_notifications::Inbox { page, show: InboxShow::Active },
	20,
);

let items = move || inbox.items();
let older = move || {
	inbox.has_more().then(|| {
		view! {
			<Button on_press=move || {
				inbox.load_more();
			}>
				"Load more"
			</Button>
		}
	})
};
```

Only visible rows are in the inbox: scheduled ones wait for their time, cancelled ones never arrive, expired ones have gone, and categories the person opted out of are hidden whatever was sent before. Each item carries its category, title, body, `data`, and its read and dismissed state. `unread_count` returns what a badge shows, live:

```rust
let unread = use_live(|| leptoskit_notifications::UnreadCount);

let badge = move || unread.data().unwrap_or(0);
```

Marking read or dismissed is a mutation on one item, and it lands on every device at once, because the state is the row's, not the device's:

```rust
leptoskit_rpc::call(leptoskit_notifications::MarkRead { notification: id }).await?;
```

Reading someone else's notification by id — guessing, replaying, whatever — answers "not found", exactly as if it didn't exist. The service resolves the owner from the forwarded identity, never from a request argument, so there is nothing to forge.

## Categories and opt-outs

A category is a short label a send carries, such as `billing` or `social`. The person opts in or out of each one:

```rust
leptoskit_rpc::call(leptoskit_notifications::SetCategoryPreference {
    category: "marketing".into(),
    opted_out: true,
})
.await?;
```

An opt-out works twice. At **send** time the service skips recipients who opted out of the category and counts them in `skipped_opted_out` — no row is created. At **visibility** time the inbox hides rows whose category its reader has since opted out of, so an opt-out reaches what was already delivered; opting back in shows it again. A person holds at most 100 preferences.

## Composing with local notifications

A received inbox item is a durable record, not an OS notification. Where you want a banner while the app runs, compose it with the [local notifications plugin](../platforms/plugin-inventory.md): ask for permission once, configure the channel Android posts through, and post each new item the live page reports.

```rust
use leptoskit_plugin_notifications::guest::Notifications;
use leptoskit_plugin_notifications::Channel;
use leptoskit_plugin_notifications::Identifier;
use leptoskit_plugin_notifications::Importance;
use leptoskit_plugin_notifications::Notification;

let notifications = Notifications::new()?;

// Once, before the first post: the OS question, and the Android channel.
// `request_permission` answers `Denied` without a dialog once the person
// has refused, so a settings screen is the honest place to retry from.
if notifications.request_permission().await? != leptoskit_plugin_notifications::Permission::Granted {
	// Show your own "notifications are off" state and stop here: the OS
	// shows nothing, and a post would be refused.
	return Ok(());
}

// Channels are Android's. iOS, macOS, the browser, and Linux have none and
// answer `Unavailable`, which is not a failure.
let inbox = Channel::new(
	Identifier::new("inbox")?,
	"Inbox".into(),
	"New items in your inbox.".into(),
	Importance::Default,
	true,
)?;
match notifications.configure_channel(&inbox).await {
	Ok(()) | Err(leptoskit_bridge::Error::Unavailable) => {}
	Err(error) => return Err(error.into()),
}

// Per item, when your screen sees one it hasn't shown yet. `item.id` is
// the inbox item's own id, so re-showing the same item replaces its
// banner instead of stacking a second one.
notifications
	.show(&Notification::new(
		Identifier::new(item.id.to_string())?,
		item.title.clone(),
		item.body.clone(),
		Identifier::new("inbox")?,
	)?)
	.await?;
```

This shows a banner only while your app runs, and only where the plugin is implemented: iOS and macOS through `UNUserNotificationCenter`, Android 26+ through `NotificationManager`, the browser through the `Notification` API, and Linux through D-Bus. **Windows returns `Unavailable`** — there is no Windows toast backend yet. On Android the channel must exist before a post (posts to a channel that doesn't are dropped), and Android 13+ also needs `POST_NOTIFICATIONS` — declared in your app's `android/app/src/main/AndroidManifest.xml` ([the `android/` folder is yours](../platforms/android.md)) and granted at runtime through `request_permission`, which uses the host's permission broker. On every platform a successful post doesn't prove the person saw anything: Focus modes, notification permissions, channel settings, and foreground presentation policy decide that. **iOS and macOS are the sharp case**: the system shows no banner for an app that is in the foreground unless the app's notification-centre delegate asks for one. The generated hosts install that delegate, so a post made while your app is open shows its banner; an app whose `hosts/ios/App.swift` or `hosts/macos/App.swift` was generated before this needs [three lines added](#apps-generated-before-receiving), and a menu-bar app has no delegate at all. A tap on a local notification reaches your app on iOS, macOS, and Android as `Received::Opened` with the notification's own id in `local` ([below](#know-what-was-tapped-and-what-arrived)). Scheduled **local** notifications and badges from the plugin are not available everywhere — see the [plugin inventory](../platforms/plugin-inventory.md) for what each platform does. None of this posting reaches a device where your app isn't running; that is what server push, below, is for.

## Push delivery

When your app isn't running on a device, nothing above can show a banner. The service can: configure a transport, enroll the person's devices, and every send that creates an inbox row also reaches those devices. A push never stands in for the durable row — it is a knock on the door of an inbox the device can open anyway.

### Configure a transport

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

`leptoskit dev` warns about a listed variable that isn't set, and a generated [`compose` deploy](24-deploying.md#the-targets) refuses to start without each one. What each holds:

| Variable                                             | What it holds                                                                                                                            |
| ---------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| `LEPTOSKIT_NOTIFICATIONS_APNS_KEY_P8`                | the contents of the `.p8` signing key from your Apple developer account, line breaks included                                            |
| `LEPTOSKIT_NOTIFICATIONS_APNS_KEY_ID`                | that key's id                                                                                                                            |
| `LEPTOSKIT_NOTIFICATIONS_APNS_TEAM_ID`               | your team id                                                                                                                             |
| `LEPTOSKIT_NOTIFICATIONS_APNS_TOPIC`                 | your app's bundle id                                                                                                                     |
| `LEPTOSKIT_NOTIFICATIONS_APNS_ENVIRONMENT`           | `sandbox` for development-signed builds, `production` for TestFlight, store, and notarized builds                                        |
| `LEPTOSKIT_NOTIFICATIONS_FCM_SERVICE_ACCOUNT`        | the contents of the service-account JSON file the Firebase console issues                                                                |
| `LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_VAPID_PRIVATE_KEY` | the VAPID private key: 32 bytes, base64url                                                                                               |
| `LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_VAPID_PUBLIC_KEY`  | the VAPID public key: 65 bytes, base64url — the same key your web app passes to `browser::EnrollOptions::new`                            |
| `LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_VAPID_SUBJECT`     | a `mailto:` or `https:` contact the push services can reach you at                                                                       |
| `LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_EXTRA_HOSTS`       | optional: comma-separated push services beyond the [shipped ones](#web-push-hosts-and-security); add it to `secrets` too when you use it |

The `.p8` key and the service account are files. Set the variables from the files rather than retyping them, so the key's line breaks survive:

```sh
export LEPTOSKIT_NOTIFICATIONS_APNS_KEY_P8="$(cat AuthKey_ABC123DEFG.p8)"
export LEPTOSKIT_NOTIFICATIONS_FCM_SERVICE_ACCOUNT="$(cat my-project-firebase-adminsdk.json)"
```

[Monosecret](secrets.md) holds the same values: declare the names in your manifest's profile and store each file's contents as its value.

leptoskit has no command for a VAPID pair yet. The Web Push ecosystem's own generator prints one in exactly the form these variables take, both halves base64url:

```sh
npx web-push generate-vapid-keys
```

Keep the pair once your app has subscribers: a subscription is made for one public key, and a new pair can't push to it.

A transport with all of its variables set is on; a transport with none is off (its platform is refused at enrollment, and the inbox is untouched); a transport with some set refuses to start, naming every missing variable in full, because half-configured looks like push works until the first send fails. `LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_EXTRA_HOSTS` without the VAPID variables counts as some.

The APNs topic and environment, the FCM project, and the VAPID identity are all yours on the server; an enrollment carries only the device credential, so nothing a client sends can steer where a push goes. The FCM service account must be the JSON Google issues — an **RSA** private key, which OAuth signs RS256; the exchange endpoint is fixed to Google's own, and the `token_uri` inside the JSON is ignored. The VAPID pair is cross-checked at startup, so a mistyped half refuses to start rather than failing at every push service at once. An extra-hosts entry that isn't a plain host name also refuses to start, named.

### Enroll a device

The signed-in person enrolls through the service's RPC — `register_push_endpoint` with one typed credential. What the credential is and how your app obtains it is platform work: request the OS's push permission, get the token or subscription, and hand it over. On iOS it is [this crate's `native` module](#enroll-an-ios-device), on Android the same module answers with an FCM token ([enroll an Android device](#enroll-an-android-device)), on macOS the same module answers with an APNs token ([enroll a macOS device](#enroll-a-macos-device)), and on the web it is [the `browser` module](#enroll-a-browser), further down.

```rust,ignore
leptoskit_rpc::call(leptoskit_notifications::push::RegisterPushEndpoint {
    credential: leptoskit_notifications::PushCredential::Apns {
        device_token: token_from_the_os,
    },
})
.await?;
```

The service answers an opaque endpoint id — the credential itself never comes back, and never appears in a list, a status row, or an error. Registering a destination that account already holds answers the same endpoint again, so a retried enrollment makes no duplicate; a Web Push subscription whose browser rotated its keys updates the stored credential in place, keeping the endpoint and its queued deliveries. Credential rotation and reactivation advance a version: queued deliveries follow the new credentials, and late responses to older credentials cannot invalidate the endpoint or change the new queue.

A person holds at most ten live endpoints, and the service keeps at most ten more that a provider has invalidated — an uninstalled app, an expired subscription — so their device list can explain them. Past that, enrolling drops the oldest invalidated ones. Only the owner can bring an invalidated endpoint back, by enrolling it again within the live limit.

### A shared device

Two people use the same phone or the same browser. The first never signed out properly — their session expired, or they signed out with no connection — so the service still thinks the device is theirs. Without a rule for this, the second person couldn't turn notifications on, and the first person's notifications would keep appearing on a device they no longer hold.

The rule: **whoever presents the device's own credential holds the device.** When the second person turns notifications on, their app hands over the same token (or the same browser subscription, keys included) the first person enrolled. The service gives the device to them under a new endpoint id and revokes the first person's: it leaves their device list, and anything still queued for it is skipped as `endpoint_revoked`. Nothing for you to write — `native::enroll` and `browser::enroll` already do this.

It only happens when a person turns notifications on. A launch never takes a device over (see below), and a Web Push address presented with different keys is refused with a conflict: knowing the address isn't holding the browser.

### When the server revokes a device

An enrollment can end without the device's help: the person removes the device from a list on another device, the next person takes it over, your server revokes it, or `ForgetPerson` erases the person. The device finds out at its next launch — `reconcile` asks the service about the enrollment the app saved — and answers that it is gone (`native::Outcome::Gone`, or `None` in a browser). Clear what you saved and show the switch as off. **A launch never enrolls again in its place**: a revoke made on the server stays made until the person turns notifications back on themselves.

When your server ends someone's session without the device — "sign out everywhere", a suspended account, an expired session it cleans up — it can release their devices too, so their notifications stop arriving on devices they're signed out of:

```rust
let revoked = call_as_app(leptoskit_notifications::push::RevokePersonEndpoints {
    person: alice_id,
})
.await?;
```

That removes every endpoint the person holds and answers how many. Their inbox, preferences, topics, and attributes stay; [`ForgetPerson`](#erase-a-person) is the call that erases those.

List and revoke are the person's own. The list is one bounded page at a time, oldest first, and it includes invalidated endpoints with their state — a device list that hides them can't explain them:

```rust,ignore
let page = leptoskit_rpc::call(leptoskit_notifications::push::PushEndpoints {
    page: Default::default(),
}).await?;
leptoskit_rpc::call(leptoskit_notifications::push::RevokePushEndpoint { endpoint: id }).await?;
```

Revoking deletes the endpoint and skips everything still queued for it as `endpoint_revoked`. Each attempt checks endpoint liveness immediately before sending: an attempt that passed that check before revocation committed may still reach the provider. Revocation does not recall provider delivery already in flight or visible banners. Another person's endpoint — or an id nobody holds — is `404`, exactly as if it didn't exist.

### Enroll an iOS device

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

What decides where a device registers is the signature of the build that is installed, not this line: a build run from Xcode registers with Apple's sandbox, and a TestFlight or App Store build registers with production. The notifications service's `LEPTOSKIT_NOTIFICATIONS_APNS_ENVIRONMENT` has to name the same one, so a service that serves TestFlight and store builds runs with `production`. An app without the setting generates exactly as before, with no push entitlement: selecting the notifications plugin for local banners never turns an app into a push-enabled one. No provisioning or certificate access happens in the build.

The generated app delegate already forwards the OS's answers: `didRegisterForRemoteNotificationsWithDeviceToken` and `didFailToRegisterForRemoteNotificationsWithError` land in the notifications plugin's in-memory broker, which survives the early callbacks that can arrive before the plugin and its guest exist. The token is held in memory only — the OS hands it out again on the next launch — and Apple's bytes are forwarded as exact lower-case hex, of whatever length the OS chooses; nothing assumes the historical 32 bytes, and nothing is invented on failure.

Then the app's own flow, in order:

```rust,ignore
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

At **launch**, once the person is signed in, call `native::reconcile(Some(&enrollment))` — with the enrollment you saved, and only with one: it is the record that this person opted in on this device. Reconciliation registers with APNs again — registration prompts nothing, Apple expects it every launch, and a device that never opted in is never registered, so a person who turned notifications off is not silently re-enabled. It also reads the [device's details](#what-a-device-says-about-itself) again. The answer is an explicit transition you persist:

- `Outcome::Kept` — the OS still issues the same token. When the device's details are unchanged too, nothing is registered: the module only asks the service, in one small read, whether the enrollment still stands (and if the service can't be reached, keeps the enrollment and asks again next launch). When they changed — an app update, a new language — the new details go in one idempotent call under the same endpoint id. Persist the enrollment it carries: it remembers what was sent, which is how the next launch knows whether anything changed. An enrollment saved by an older version of this module sends once, then settles.
- `Outcome::Rotated { current, retired }` — the OS now issues a different token. The module checks that the previous handle is still this person's and retires it — a rotated token never leaks an orphan endpoint — so you persist `current` and drop `retired`. When the person is at the ten-endpoint bound, the quota refusal is answered by retiring the known-stale handle and re-registering once, so rotation at the quota still completes.
- `Outcome::Gone` — the service no longer has this enrollment for the signed-in person: it was [revoked on the server](#when-the-server-revokes-a-device), taken over by the device's next person, or was never theirs — a previous account's leftover handle. Nothing was registered. Clear the saved enrollment and the opt-in and show the switch as off; only the person's own tap enrolls again.
- `Outcome::Off` — you passed no previous enrollment, so nothing was registered and nothing asked; the cue to show the opt-in switch.

```rust,ignore
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

**What a push does while the app is closed** is the OS's work: APNs wakes the system, which shows the alert from the payload — title and body, collapsed by the notification's id — and your durable inbox row is already in the service. A push that arrives while the app is **open** shows its banner too, and a tap on either reaches your code through the [receiver](#know-what-was-tapped-and-what-arrived). What this does **not** include: any claim of live APNs acceptance here — provider acceptance is only ever proven by a real device with real credentials, which this setup doesn't exercise.

### Enroll an Android device

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
- the default channel a push is posted through when the send names none: `leptoskit.notifications.default`, named in the manifest and created by the notifications plugin when the app starts ([Android channels](#shape-a-message) says how to rename it);
- the small icon a push shows in the status bar, when your app has one: set `notification` under `[app.icons]` ([app icons](24-publishing.md)) and the manifest points Firebase at the generated `@drawable/ic_notification`. Without it Firebase falls back to your launcher icon, which Android draws as a plain white shape, and generation says so;
- keep rules for the Firebase names the plugin looks up at runtime, so a minified release build still finds them;
- the `POST_NOTIFICATIONS` permission the local-notifications plugin already declares.

An app without the line generates byte-for-byte as before, so selecting the plugin for local banners never turns an app into a push-enabled one.

Two things are yours, not leptoskit's: **`google-services.json`** — the file the Firebase console issues for your app's package id, placed at `android/app/google-services.json`. Generation stops and names the path when push is on and it's missing; leptoskit never generates or commits it. And the Gradle script: `android/app/build.gradle.kts` is your file, so generation checks it instead of rewriting it. An app scaffolded before this setting gets an error that names the exact lines to add, and an app scaffolded with a version whose lines read `apply(plugin)` gets one that says to change them to `apply(plugin = plugin)` — the first form asks Gradle for a script file named after the plugin, and the build fails.

At runtime the flow is the iOS flow verbatim — request permission, enroll from the gesture, reconcile at launch with the saved enrollment, disable on sign out:

```rust,ignore
let enrollment = native::enroll().await?;
```

What differs under the hood: Firebase answers with a registration token — printable ASCII of no assumed length, not hex — the generated messaging service forwards token **refreshes** to the same in-memory broker the registration task feeds, and a device without usable Firebase refuses instead of pretending. The refusal is a stable code, not SDK text: `firebase-not-configured` means the app generated without the push opt-in or has no `google-services.json` values (the Firebase classes are found reflectively, so an app that never opted in simply answers this), `os-refused` covers the rest, and `token-shape` means Firebase handed over something outside the contract's bounds. The token is held in memory only — Firebase hands it out again on the next launch — and its bytes never appear in a log, an error, or a `Debug` print.

A device at Google Play services' mercy is a real constraint worth knowing: delivery needs Play services on the device. While your app is in the **foreground**, Firebase hands an arriving push to the app instead of showing it. The generated messaging service passes it to the notifications plugin, which posts it in the tray itself — title, body, the channel the send named or your default one, with a repeat of the same notification replacing the first — and tells your code it arrived. The image, color, and sound a send asked for are not applied to a push shown this way. What a push does while the app is **closed** is Google's work: the message's notification block shows the title and body in the system tray, collapsed by the notification's id, and your durable inbox row is already in the service. A tap on either opens the app and reaches your code through the [receiver](#know-what-was-tapped-and-what-arrived). An app whose `android/` folder was generated before this needs nothing changed: the messaging service is generated on every build, and the tap is read from the intent your activity already keeps. And as everywhere: acceptance at the provider is only ever proven by a real device with real credentials, which this setup doesn't exercise.

### Enroll a macOS device

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

The profile and the environment go together. For local work use a **macOS development** profile with `aps-environment = "development"`: it registers with Apple's sandbox, and it runs only on the Macs listed in the profile. For `leptoskit publish macos` switch both: a **Developer ID** profile with `aps-environment = "production"`. The distribution embeds that profile, signs with your `LEPTOSKIT_PUBLISH_MACOS_IDENTITY`, and registers with production. Either way the value must match the notifications service's `LEPTOSKIT_NOTIFICATIONS_APNS_ENVIRONMENT`. leptoskit copies the profile and checks that it is there; it doesn't read what the profile grants, so a profile for another bundle id, another environment, or one without the push capability is something macOS reports only when the app launches.

An app without the `aps-environment` setting generates and signs exactly as before, with no push entitlement and no profile: selecting the notifications plugin for local banners never turns an app into a push-enabled one.

The generated macOS app delegate already forwards the OS's answers — `didRegisterForRemoteNotificationsWithDeviceToken` and `didFailToRegisterForRemoteNotificationsWithError` land in the notifications plugin's in-memory broker, the same one iOS uses, which survives the early callbacks that can arrive before the plugin and its guest exist. `REMOTE_REGISTER` calls `NSApplication.registerForRemoteNotifications` and waits the bounded answer; a silent OS answers "unconfirmed" instead of replaying a prior token, and the token's bytes never reach a log.

At runtime the flow is the iOS flow verbatim — request permission from a gesture, enroll from the gesture, reconcile at launch with the saved enrollment, disable on sign out:

```rust,ignore
let enrollment = native::enroll().await?;
```

One honest limitation: the stock **menu-bar host** (a `[native] mode = "menu-bar"` app that keeps the generated status-item lifecycle) carries no forwarding delegate, so its registration answers stay "unconfirmed", and it installs no notification-centre delegate either, so it shows no banner while it runs and hears no taps — a push-enabled app wants the standard windowed host, or its own delegate wired to the plugin's `noteDeviceToken`, `noteRegistrationFailure`, `installDelegate`, and `noteRemoteNotification`. And what a push does while the app is **closed** is the OS's work, as on iOS: APNs shows the alert and your durable inbox row is already in the service. A push that arrives while the app is open shows its banner, and a tap reaches your code through the [receiver](#know-what-was-tapped-and-what-arrived). As everywhere, provider acceptance is only ever proven by a real Mac with real credentials, which this setup doesn't exercise — and the same goes for the signing above: the profile and identity steps follow Apple's code-signing rules, but they haven't been run against a real Apple developer account as part of leptoskit's own testing, so treat your first push-enabled Mac build as the check.

### Enroll a browser

On the web the whole enrollment is this crate's `browser` module, behind the feature of the same name. Depend on the crate in your app and turn the feature on:

```toml
[dependencies]
leptoskit_notifications = { version = "0.3", features = ["browser"] }
```

The module is three things: the **receiver** service-worker script, an **availability** check, and the **enroll** and **disable** flows. All of it rides your app's own RPC client, so the enrollment is the signed-in person's, forwarded by your app exactly as their inbox reads are — no service token, and no copy of the session anywhere in the module or in the worker.

**Serve the receiver.** Build the script with your inbox route baked in and ship it with your web assets at the origin's root — `web/sw.js`, served as `/sw.js` with a JavaScript content type — so its scope covers the app:

```rust,ignore
let script = leptoskit_notifications::browser::receiver_script("/inbox")?;
// Write `script` to `web/sw.js` at build time, or serve it from a route.
```

`leptoskit build` copies `web/` into the served output as-is, so a build step (a script in your `justfile`, or a small generator run before `build`) that writes the built script to `web/sw.js` is enough. Or serve it from your server, same-origin at the root, with a JavaScript content type:

```rust,ignore
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};

async fn sw_js() -> Response {
    let script = leptoskit_notifications::browser::receiver_script("/inbox")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    ([(header::CONTENT_TYPE, "text/javascript")], script).into_response()
}

// .route("/sw.js", get(sw_js)) on the same router that serves your app.
```

The route is the only URL a notification click may open, checked against the worker's own origin again at click time — payload content can never add a host, a scheme, or a path, and a route the builder refuses (empty, not a `/`-rooted path, scheme-relative, backslash, control characters) never ships. Register the script once at startup, the way [the web platform page](../platforms/web.md#service-workers) does it, and wait for the worker to be active:

```rust,ignore
use leptoskit_notifications::browser;

let registration = browser::register("/sw.js", browser::RegisterOptions::default()).await?;
let _ = browser::ready().await?;
```

`ready` resolves once the registration has an **active** worker, which is all enrollment needs. It doesn't wait for that worker to control the page. The receiver takes control of the pages already open as it activates (`clients.claim()`), so the tab a person enrolls from is one a later click can route, without a reload first.

**One worker per scope.** A browser keeps a single service worker for a scope, and registering a second script at the same scope replaces the first. If your app already ships its own worker — the caching `sw.js` from [the web platform page](../platforms/web.md#service-workers), say — don't register the receiver beside it. Serve the built receiver under another path (`/lkn-receiver.js`) and load it from the top of your own worker with `importScripts("/lkn-receiver.js")`, so one script holds both; leave the `push` and `notificationclick` events to the receiver. This combination is guidance rather than something leptoskit tests, so check a push and a click in your own app after wiring it.

**iPhone and iPad.** Safari on iOS and iPadOS offers Web Push only to a web app the person has added to their Home Screen (iOS 16.4 and later), which needs a web app manifest that opens it standalone. In an ordinary Safari tab there is no Push API at all, so `availability()` answers `unsupported` — show "add this app to your Home Screen" there rather than a switch that can't work.

**Ask before you offer.** `browser::availability()` reads where push stands — `ready`, `insecure_context` (serve `https`; nothing in code works around it), `unsupported`, or `denied` (the person refused; link to the browser's site settings rather than prompting again). It never prompts, so a settings screen may call it on every render. **Prompting is the browser's answer to subscribing, and it happens only inside `enroll`** — the two calls are split by when they may run:

- **`browser::reconcile`**, on launch, once the person is signed in, with the `Enrollment` you saved beside their session when they turned notifications on. It renews the subscription the browser holds under that enrollment and answers it back. It answers `None` — the cue to clear what you saved and show the switch — when you pass no saved enrollment, when the browser no longer holds a subscription, or when the service [no longer has the enrollment](#when-the-server-revokes-a-device). A browser can hold a subscription the previous person left behind, which is why a held subscription alone never enrolls anyone. Reading a subscription needs no permission, so this may run from any startup path.
- **`browser::enroll`**, from an explicit gesture — a button, never a mount or a render. It subscribes only when the browser holds no subscription, and that subscribe is the one call the browser gates on the gesture; waiting for the worker at launch (`browser::ready`) keeps it settled long before, so nothing asynchronous eats the gesture's activation window.

```rust,ignore
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

Every enrollment also carries the browser's [details](#what-a-device-says-about-itself). A browser can't report your app's version, so name it on the options if you want to target web versions too — `browser::EnrollOptions::new(vapid_public_key).app_version(env!("CARGO_PKG_VERSION"))` — or leave it out and `device.app_version` stays empty for browsers.

`enroll` checks availability first, waits for the **active** worker instead of racing its install, reuses the subscription the browser already holds (a second tap re-enrolls the same endpoint instead of prompting twice, and two racing calls converge on one subscription), and registers it under the person who is signed in — but only when that subscription was created for **your own VAPID key**: a subscription made for another sender's key can't receive your pushes, so claiming it would end in silent delivery failure, and it is refused (`KeyMismatch`) rather than replaced. When the receiver is registered under an explicit scope, name it in `EnrollOptions::worker_scope`: the match is exact — a worker registered for a broader scope never stands in for the one you asked for. It answers an opaque `Enrollment` — the endpoint's id — which you keep beside the person's session on the device.

**Sign out by disabling.** While the person is still authenticated, `browser::disable(&enrollment).await?` revokes this person's service enrollment. It never unsubscribes the browser or changes OS permission. The subscription remains reusable: re-enabling can reuse it, but this service requires enrollment before sending. If the previous account never got to revoke, the browser passes to [the next person who enrolls in it](#a-shared-device).

Disable requires a successful authenticated revoke acknowledgement. Failures, including not-found, stay explicit; no endpoint list can establish success. Retries use a stable key derived from the endpoint id, including after reload. Receipts are scoped to the authenticated caller and replay only while retained by the service; pruning an old receipt can leave a retry unconfirmed. Revocation stops future attempts at the service's documented revocation boundary; provider delivery already in flight and visible banners are not recalled. Origin-wide unsubscribe and permission revocation are separate actions in browser settings or the lower-level worker API.

**When the browser changes the subscription on its own** — a rotated or replaced subscription, the `pushsubscriptionchange` event — the worker cannot reconcile anything: it deliberately holds no credentials, so it has no identity to enroll with. The supported reconciliation is the next authenticated app launch calling `reconcile` again with the saved enrollment; a stale handle's `disable` leaves the replaced subscription alone, and the replaced subscription's endpoint dies at its push service, whose answer invalidates the stale record on its next delivery. Churn is bounded by the ten-endpoint limit, and the person can revoke the dead ones from their own list.

The receiver itself shows what the service pushed: the envelope's title and body under a tag built from the notification's id, so a repeat replaces rather than stacks; the send's `data` rides on the banner's own data, inert. A send's options reach it too — an image, a badge count (`navigator.setAppBadge` where the browser has it), a silent flag, and the web presentation set. The service never pushes a [data message](#silent-data-messages) to a browser. One flagged `lkdata` that arrived anyway is handed to the pages that are open; when one of them is in view it shows nothing, and when none is it shows the "New notification" banner, because a browser punishes a push that shows nothing. A push that is missing, oversized (past the service's 4 KiB record), malformed, or typed wrong still shows one truthful banner — "New notification", no invented text — under a stable tag, so a broken sender can never silently swallow a delivery. A click closes the banner, focuses an app window of this origin and opens the send's own link when it resolves to this origin — a path like `/invoice/42`, or an absolute URL of this origin — and otherwise brings it to your inbox route with the notification's id, or opens that route in a new window. A click on one of the `web.actions` buttons does the same and adds the button's id to the address as `lka`, so the page can tell "Approve" from a click on the banner. An address the worker opens for a click always carries the notification's id as `lkn`, on a send's own link too. A page that runs a [receiver](#know-what-was-tapped-and-what-arrived) is handed the click instead and changes its own route, with nothing loading again; a page that doesn't answer within half a second is brought to the address the old way, which loads it again there. A link anywhere else is refused at click time: payload content can never steer a click off the app's own origin. The worker holds no credentials and calls no server: nothing fetches, and **no read state changes until the authenticated app itself opens** and marks what it read.

### Know what was tapped and what arrived

A push that only shows a banner leaves three questions to your app. Which notification did the person tap, so the right screen opens? What arrived while the app was already open, so the screen in front of them is current? And what did a silent data message say? One receiver answers all three, on iOS, macOS, Android, and the web.

Make a receiver when your app starts and read it in a loop. On a device it is `native::Receiver`:

```rust
use leptoskit::prelude::*;
use leptoskit_notifications::native::Receiver;
use leptoskit_notifications::receive::Received;

use crate::routes::Navigation;
use crate::routes::Route;

#[component]
pub fn App() -> impl IntoView {
	let navigation = Navigation::new(Route::Home);
	let taps = navigation.clone();

	leptos::task::spawn_local(async move {
		let mut receiver = Receiver::new();

		while let Ok(received) = receiver.next().await {
			match received {
				Received::Opened(opened) => {
					if let Ok(route) = Route::parse(&opened.route("/inbox")) {
						taps.navigate(route);
					}
				}
				Received::Shown(_shown) => {
					// A banner just appeared over your app.
				}
				Received::Data(_message) => {
					// A silent message: read `data`, refresh what it names.
				}
			}
		}
	});

	view! { {navigation.render()} }
}
```

In a browser the loop is the same with `browser::Receiver`, which re-exports the same types (`leptoskit_notifications::browser::Received`). An app that builds for both picks the module with `#[cfg(target_arch = "wasm32")]` the way it already does for `enroll`.

**A tap** is `Received::Opened`. It carries:

- `notification`: the id the inbox lists the item under, so you can open it or [mark it read](#read-the-inbox-on-any-platform).
- `link`: the send's `options.link`, when it had one.
- `action`: the id of the action button that was pressed, or `None` for a tap on the notification itself.
- `data`: the send's `data`.
- `local`: the id of a local notification your app posted with the [plugin](#composing-with-local-notifications), when that is what was tapped.
- `launched`: `true` for the tap that started the app.

`opened.route("/inbox")` answers where to go: the send's link when it is a path on your app, such as `/invoice/42`, and otherwise the route you name with the notification's id added as `lkn` — the same address the web receiver opens. A link that is a full `https` URL is left for you to read from `link`, because only your app knows whether that URL is its own.

**The tap that started the app** is not lost. The host keeps what arrived before your code ran, and the receiver's first answer holds it, marked `launched`. If you want it before the first screen is drawn, ask without waiting:

```rust
let mut receiver = Receiver::new();

for received in receiver.pending().await.unwrap_or_default() {
	if let Received::Opened(opened) = received
		&& opened.launched
	{
		// Start on this screen instead of the home screen.
	}
}
```

Use one receiver for the life of the app: what the first one has taken, a second one never sees.

**A notification that arrived while the app was open** is `Received::Shown`, with its id and `data`. The system has shown it already, so there is nothing to draw. Your inbox query is live and updates itself; use this for anything else that should react.

**A silent data message** is `Received::Data`, described [above](#silent-data-messages).

What each platform does:

|                | Shown while the app is open                   | Tap while the app is running | Tap that starts the app                            | Action button                                               | Data message |
| -------------- | --------------------------------------------- | ---------------------------- | -------------------------------------------------- | ----------------------------------------------------------- | ------------ |
| iOS, macOS     | Banner, list entry, sound, and badge; `Shown` | `Opened`                     | `Opened`, `launched`                               | `Opened` with `action`, once the app registers the category | `Data`       |
| Android        | Posted by the plugin; `Shown`                 | `Opened`                     | `Opened`, `launched`                               | Not sent                                                    | `Data`       |
| Web            | Banner from the worker; `Shown` on open pages | `Opened`, no reload          | `Opened`, `launched`, read from the page's address | `Opened` with `action`                                      | Not sent     |
| Windows, Linux | No push                                       | —                            | —                                                  | —                                                           | —            |

Four things to know before you rely on it:

- **It is not a durable record.** The host keeps the sixteen most recent events between asks and drops older ones, and a force-quit app starts empty apart from the launching tap. The inbox is the record; this is what happened on this device just now.
- **On Android every value in `data` is a string.** FCM carries a flat map of strings, so a number arrives as its text and a nested object as its JSON text. Keys the envelope or Firebase reserve are not in `data`.
- **On the web, a click reaches a page only while a receiver exists.** Create it once at startup. Until then, and in a page that never creates one, the worker behaves as before and loads the address.
- **Dismissing a notification is not reported.** Only taps are.

Where nothing can be received — Windows, Linux, the terminal, a browser without service workers, an app whose host predates this — `next` answers `ReceiveError::Unavailable` and the loop above ends. `ReceiveError::NoPlugin` means the notifications plugin is missing from `[native] plugins`.

#### Apps generated before receiving

`hosts/ios/App.swift` and `hosts/macos/App.swift` are written once, when the host is first generated, and are yours from then on, so an existing app doesn't get the new lines by itself. Without them the app builds and enrolls as before, shows no banner while it is open, and its receiver hears nothing. Add them to your app delegate.

On iOS, install the delegate before launch finishes and forward silent pushes:

```swift
func application(
    _ application: UIApplication,
    willFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil
) -> Bool {
    #if canImport(LeptosKitNotifications)
    NotificationsPlugin.installDelegate()
    #endif

    return true
}

func application(
    _ application: UIApplication,
    didReceiveRemoteNotification userInfo: [AnyHashable: Any],
    fetchCompletionHandler completionHandler: @escaping (UIBackgroundFetchResult) -> Void
) {
    #if canImport(LeptosKitNotifications)
    NotificationsPlugin.noteRemoteNotification(userInfo)
    completionHandler(.newData)
    #else
    completionHandler(.noData)
    #endif
}
```

On macOS the two are `applicationWillFinishLaunching(_:)` and `application(_:didReceiveRemoteNotification:)`, with the same two calls. It has to be the _will_ finish launching callback: the system delivers the tap that started the app as launch ends, and only to a delegate that is already installed.

An iOS app with `aps-environment` set also gets the `remote-notification` background mode in its `Info.plist`, which is what lets the system wake it for a silent push. That file is generated on every build, so there is nothing to add. Android needs no change at all.

### What a device says about itself

You want to reach "iPhones still on version 2.0" or "devices set to French", so the service needs to know those things about each device — not about each person, because one person's phone and laptop can differ. Every enrollment carries three details the device reports about itself, and your app writes no code for them:

| Detail        | iOS and macOS                                 | Android                           | Web                                                        |
| ------------- | --------------------------------------------- | --------------------------------- | ---------------------------------------------------------- |
| `locale`      | the first of the person's preferred languages | the default locale's language tag | `navigator.language`                                       |
| `time_zone`   | the current time zone's identifier            | the default time zone's id        | `Intl.DateTimeFormat().resolvedOptions().timeZone`         |
| `app_version` | the app's `CFBundleShortVersionString`        | the package's version name        | only what you pass to `EnrollOptions::app_version`, if any |

They are sent when the device enrolls and again whenever they change: on iOS, Android, and macOS the `native` module's launch `reconcile` reads them every launch and sends them only when they differ from what it last sent, so an app update or a language change reaches the service at the next launch and an unchanged launch costs one small read; on the web, `reconcile` and `enroll` send them each time. Windows and Linux have no enrollment yet, so they have no devices to describe.

The service checks each detail's shape: the locale is a BCP 47 tag of at most 35 bytes, stored in canonical case (`en_gb` becomes `en-GB`); the time zone is an IANA-style name of at most 64 bytes — checked for shape only, not against a time zone database, because a device's database can be newer than the server's; the app version is free text of at most 64 bytes. A client drops a detail that fails its check rather than lose its enrollment over it, and a request that sends a bad one directly is refused with the detail named. Each re-enrollment replaces what the endpoint said before; a change to the details alone never rotates the endpoint's credential, so pushes already queued for it carry on untouched.

The service also uses the language and the time zone itself: a [translated send](#say-it-in-each-persons-language) or a [local-time send](#send-at-each-persons-local-time) reads them from the device it heard from most recently, when your server has not set the person's `locale` or `time_zone` attribute. "Heard from" means the device enrolled, changed one of these details, or (on the web) renewed its enrollment at launch.

Details belong to the endpoint and are read back only by its owner: `push_endpoints` answers each endpoint's `device` beside its platform, and nothing your server or anyone else reads — delivery status, test-send outcomes — carries them. The condition grammar reads them, below.

### What a send does

When a recipient of a send has enrolled endpoints and your server has configured that platform's transport, the send enqueues one durable delivery per notification and endpoint in the send's own transaction. The push job attempts each one when the notification becomes visible — a scheduled send pushes at its `visible_at`, not before — and re-checks, at the moment of the attempt, everything that could have changed since the send: cancellation, expiry, the category opt-out the person may have flipped meanwhile, and the endpoint's liveness. A cancelled send never pushes; an expired one is skipped; an opt-out made after the send reaches the queue still stops the push.

Transient failures — a 429, a 5xx, a network error — retry with backoff, and with the provider's `Retry-After` honored when it asks for longer, up to eight attempts (eight is eight: the ninth never goes out), and then end as rejected with `attempts_exhausted`. The waits start at one minute and double, so the eighth attempt comes about two hours after the first: a provider outage of an hour loses no delivery. Only a provider's answer that the device is gone invalidates an endpoint — APNs's `410`, FCM's `UNREGISTERED`, a push service's `404` or `410` — so nothing further is pushed to a dead credential and its other queued deliveries are skipped. An answer that means your setup is wrong ends the delivery as rejected with `operator_configuration` and leaves the device enrolled, because the fix is yours: an APNs topic or environment the token doesn't belong to (`BadDeviceToken` is what a sandbox token gets from production, `DeviceTokenNotForTopic` what any token gets under the wrong bundle id), an FCM token from another Firebase project or a Firebase project missing its APNs or Web Push credential, and a push service refusing your VAPID signature, as one does after the key pair changes. A provider token or access token the provider refuses is replaced and the push retried, and a plain 404 from FCM — a project-naming problem on your side, not a dead device — retries the same way while you fix it. Other permanent failures end at once with `provider_rejected`. Failure of one endpoint never rolls back the inbox row or the other endpoints' deliveries: the inbox row committed with the send, and push is best-effort on top of it.

Delivery retries stop at eight actual push requests, including requests refused for authentication. OAuth token exchanges are separate and do not spend push attempts. FCM refreshes once after a 401; the next push goes through the durable queue and its attempt budget. An exchange failure retries every minute with sanitized `operator_configuration` status. Neither provider diagnostics nor credentials appear in that status.

Every delivery has a deadline: the earlier of notification expiry and 24 hours after `visible_at`, including notifications with no expiry. Provider `Retry-After` accepts delta seconds and HTTP dates; invalid values use backoff. A provider delay is never shortened into an earlier push. If it exceeds the deadline, the queue wakes at the deadline and ends delivery (`expired` or `horizon_exceeded`) without another request. Unrepresentable arithmetic ends with `retry_overflow`. Local schedule, cancellation, expiry, and horizon decisions do not spend attempts. Transient and refused retries on every platform use exponential backoff starting at one minute — 1, 2, 4, and so on to 64 minutes — plus up to a quarter more of jitter that differs from delivery to delivery, and respect longer provider delays. See [FCM error codes](https://firebase.google.com/docs/cloud-messaging/error-codes).

The push job works through a send in rounds of at most a thousand attempts, each started within a minute, and a round hands whatever is left to the next at once, so a send to a million devices is never one long job. A delivery whose stored options this server can't read — written by a newer version, before a rollback — waits with `operator_configuration` status rather than pushing something else, and goes out once a server that reads them is back, within its delivery window.

A send's [options](#shape-a-message) ride the same deliveries, and when a payload is over its provider's limit the extras go in a fixed order — the per-platform blocks and the image, then the sound, badge, and link, then the send's `data` — before the id-only envelope, and only then a clear `oversized` end without a request.

### Test on a device

Before a campaign, ask a real device what it thinks of your message. `send_test` pushes a spec to one person's live endpoints — the way FCM's "test on device" does — and answers per endpoint, having written nothing: no inbox row, no delivery row, no count anywhere.

```rust,ignore
let outcomes = call_as_app(leptoskit_notifications::SendTest {
    spec: test_spec,
    person: alice_id,
    endpoint: None, // or one endpoint id, to probe a single device
})
.await?;
```

The spec is a full `SendSpec` — options included, silent data messages included — so the device receives exactly what a real send to it would carry. If the spec has [variants](#say-it-in-each-persons-language), the test pushes the one this person would read. The blast radius is the person's own enrollment: at most their ten live endpoints, each attempted exactly once, one provider request each. A transient failure answers `rejected` with no reason and the retry is calling again; an endpoint id that isn't the person's own live one is `404`. There is no idempotency key to reuse — a test answers, it doesn't record — so a repeated call, a retried request included, pushes again. While the providers answer, the service holds nothing open, so a slow provider doesn't hold up its other work.

Read what happened from your server, per endpoint, with `call_as_app`:

```rust,ignore
let page = call_as_app(leptoskit_notifications::PushDeliveries {
    send_key: "invoice-paid-42".into(),
    page: Default::default(),
}).await?;
```

Each item names the notification, the endpoint's opaque id, the platform, a state — `enqueued`, `retrying`, `accepted`, `rejected`, or `skipped` — and, when it left the queue, a sanitized reason code. **`accepted` is the provider taking the message, not a person seeing one.** There are no delivery receipts here: focus modes, notification permissions, and quiet hours all decide what a delivered push becomes, and the service can't see past the provider's answer.

Delivery is **at least once**. An attempt whose answer was lost retries, so a device may see the same notification twice; every attempt carries the notification's id as its collapse id — `apns-collapse-id`, the FCM collapse key, the Web Push `Topic` — so a repeat _usually_ replaces its predecessor instead of stacking. Collapse is best-effort: a provider that doesn't coalesce, or a device that was offline for both attempts, can still show two banners, and the id-only fallback keeps its collapse id too. The payload is the visible title and body plus the notification's id for routing; the send's `data` rides along where it fits and is dropped, down to an id-only envelope, when the provider's payload limit says so (limits are checked on the encoded bytes, not the raw strings). A send's `expires_at` is the providers' TTL — APNs `apns-expiration`, the FCM `ttl`, the Web Push `TTL` — so a provider holds a message no longer than the notification is worth delivering. Nothing executable ever rides in a push. APNs and Web Push carry your `data` whole under its own key; FCM's data is one flat map, so keys that would collide with the envelope's own (`lkn`, `lklink`, `lkimg`, `lkbadge`) or that FCM reserves (`from`, `message_type`, and anything starting `google.` or `gcm.`) are dropped from the FCM copy rather than sent, because FCM refuses a whole message for one of them.

### Web Push hosts and security

A Web Push subscription's endpoint is attacker-shaped material — anyone who can drive an enrollment can hand the service a URL — so the service pushes only to the shipped push services' hosts: Google's (`fcm.googleapis.com`, where Chrome and Android Chrome deliver), Mozilla's (`updates.push.services.mozilla.com`, Firefox's), Apple's (any subdomain of `push.apple.com`, such as Safari's `web.push.apple.com`, as Apple asks senders to allow), and Microsoft's Windows Push Notification Services (any subdomain of `notify.windows.com`, where Edge on Windows delivers). The two domains match whole labels: `web.push.apple.com` passes, while `webpush.apple.com`, the bare `push.apple.com`, and `web.push.apple.com.attacker.test` don't. Lookalike hosts (`fcm.googleapis.com.attacker.test`), private and reserved addresses (the full IANA special-purpose registries, transition networks included), credentials-in-URL, and non-443 ports are refused at enrollment. To push through another operator-run push service, name it in `LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_EXTRA_HOSTS`; those are resolved within a bounded time and validated at connection time (every address must be globally routable), and the connection is pinned to the exact addresses that passed — re-resolved on each attempt, so an address rotation is picked up without a rebind hole. Requests are never redirected: a push service that answers with a redirect has, as far as this service is concerned, not taken the message.

### Platforms

Push reaches a device through its platform's service: iOS and macOS through APNs, Android through FCM, and the web through the browser's own push service (Web Push with VAPID) — the browser's client half is [this crate's `browser` module](#enroll-a-browser), and the native client half for iOS, Android, and macOS is [the `native` module](#enroll-an-ios-device). **Windows and Linux have no enrollment story in leptoskit yet**, and HarmonyOS and the watch surfaces are out of scope for push — their inbox is durable retrieval, as everywhere. Background delivery on Android also needs the app's own OS setup there (`google-services.json`, and `[native.android] push` checked).

## Target devices

Some messages are about a device, not a person: "please update" belongs on the phones still running the old version, and a French announcement on the devices set to French. Conditions read what each enrolled device [says about itself](#what-a-device-says-about-itself) with `device.` predicates, beside `user.` and topics:

```rust
let sent = call_as_app(leptoskit_notifications::Send {
    idempotency_key: "update-to-2.1".into(),
    spec: SendSpec {
        category: "app".into(),
        title: "Update available".into(),
        body: "Version 2.1 fixes the sign-in bug. Update from the App Store.".into(),
        data: None,
        visible_at: None,
        expires_at: None,
        options: MessageOptions::default(),
    },
    target: SendTarget::Condition(
        "device.platform == 'apns' && device.app_version < '2.1'".into(),
    ),
    validate_only: false,
})
.await?;
```

| Predicate            | Reads                                                                                                |
| -------------------- | ---------------------------------------------------------------------------------------------------- |
| `device.platform`    | the push service the device enrolled with: `'apns'` (iOS and macOS), `'fcm'` (Android), `'web_push'` |
| `device.locale`      | the full language tag, such as `'en-GB'`, compared without regard to case                            |
| `device.language`    | the tag's first part, lower case: `device.language == 'fr'` matches `fr-CA` and `fr-FR`              |
| `device.app_version` | the app's version, compared as a version (below)                                                     |
| `device.time_zone`   | the IANA name, such as `'Europe/London'`                                                             |

Each takes the operators `user.` predicates take — `==`, `!=`, `in ['a', 'b']`, `has(device.<key>)`, and `<`, `<=`, `>`, `>=`. Any other key is refused when you send, naming the five there are, and so is a platform that isn't one of the three — `device.platform == 'ios'` would match nobody, so it never gets that far. Note that APNs serves both iPhones and Macs, so `'apns'` means both.

**Versions compare as versions.** For `device.app_version`, every comparison — `<`, `<=`, `>`, `>=`, `==`, `!=`, and `in` — follows one rule:

1. Everything from the first `-` or `+` is ignored, so `2.1.0-beta` and `2.1.0+812` read as `2.1.0`.
2. The rest is split on `.`, and every part must be digits; otherwise it isn't a version. The empty string, `beta`, `v2.1`, and `2..1` aren't versions.
3. The parts compare as whole numbers from left to right, and a missing part counts as 0: `2.10 > 2.9`, and `2.1 == 2.1.0`.

When the device's version isn't a version by this rule, `<`, `<=`, `>`, and `>=` answer false, and `==`, `!=`, and `in` compare the text exactly — so a build named `nightly` still matches `device.app_version == 'nightly'`. The bound of `<`, `<=`, `>`, or `>=` must itself be a version, or the send is refused.

**A device predicate is judged per device.** A send still addresses people — one inbox row each — so the service judges every person it walks once with no device at all, and once for each of their enrolled devices with that device's details (devices the provider has invalidated don't count). The person is in the audience when the condition holds with no device or on at least one device, and the push goes only to the devices it holds for. With no device, a `device.` predicate answers false, exactly as a `user.` predicate does for an attribute the person doesn't have — `!=` and `in` included.

Take the update message above, and Alice, who has an iPhone on 2.0.4 and an Android phone on 2.2.0:

- with no device, `device.platform == 'apns'` is false, so the condition is false;
- on her iPhone, both halves hold;
- on her Android phone, the platform doesn't match.

Alice is in the audience, so she gets one inbox row — which her Android phone shows too, because the inbox is hers, not the device's — and exactly one push, to her iPhone. Bob, whose only device is an Android phone, isn't in the audience at all: no row, no push. A condition that never mentions `device.` works exactly as before: a matching person's every device gets the push.

The rule cuts the other way for a negated predicate. `user.plan == 'pro' && !(device.platform == 'apns')` holds with no device for anyone on the pro plan, so every pro person gets the inbox row — including Dave, who has no devices, and Erin, whose only device is an iPhone. The push goes to the devices the condition holds for: Alice's Android phone, and neither Dave nor Erin gets one.

A condition made only of device predicates walks the same person registry `SendTarget::Everyone` does, and one that names a topic it needs walks that topic's subscribers, as always. The devices are decided when the fanout writes a batch's rows, with each device's details as they stand then — for a scheduled send too, whose rows and pushes then wait for its time; a device that changes its details after that keeps the decision. `validate_only` counts the same per-device audience, and a refused key is refused there too.

## How long things are kept

A service that never deletes anything gets slower and more expensive every month, and keeps personal data long after anyone needs it. So the service deletes what is over, by itself, once an hour, in small pieces that never hold up a send or an inbox. What counts as over:

| What                                                                                         | Kept for                                                          | Setting                                             | Default  |
| -------------------------------------------------------------------------------------------- | ----------------------------------------------------------------- | --------------------------------------------------- | -------- |
| A notification in someone's inbox, read or not                                               | this long after it became visible                                 | `LEPTOSKIT_NOTIFICATIONS_RETENTION_INBOX_DAYS`      | 365 days |
| A notification nobody can see any more: expired, cancelled, or a silent data message         | this long after it ended                                          | `LEPTOSKIT_NOTIFICATIONS_RETENTION_ENDED_DAYS`      | 30 days  |
| The record of one push to one device, once it finished (accepted, rejected, or skipped)      | this long after it finished                                       | `LEPTOSKIT_NOTIFICATIONS_RETENTION_DELIVERIES_DAYS` | 30 days  |
| The record of a send: its idempotency key, counts, and (for a topic or condition) its target | at least this long after the send, and as long as any row it made | `LEPTOSKIT_NOTIFICATIONS_RETENTION_SENDS_DAYS`      | 90 days  |

Each setting is a whole number of days, or `off` to keep that kind of row forever. A value that is neither refuses to start, named. Like every variable the service reads, a setting only reaches it when your `[services.notifications]` table lists its name under `secrets`:

```toml
[services.notifications]
secrets = [
	"LEPTOSKIT_NOTIFICATIONS_RETENTION_INBOX_DAYS",
	"LEPTOSKIT_NOTIFICATIONS_RETENTION_SENDS_DAYS",
]
```

```bash
LEPTOSKIT_NOTIFICATIONS_RETENTION_INBOX_DAYS=90
LEPTOSKIT_NOTIFICATIONS_RETENTION_SENDS_DAYS=off
```

The service prints what it will keep when it starts, so you can check the settings arrived.

A few rules hold whatever you set:

- **Nothing in use is deleted.** A notification with a push still waiting to go out stays, and so does that push. A send that is still working through its audience keeps every row it has written. A scheduled notification is never too old before its time has come, and one you cancelled ahead of its time is kept until that time is `ENDED_DAYS` in the past.
- **A push record never outlives its notification.** When a notification goes, its push records go with it, whatever their age.
- **A person's inbox simply gets shorter.** Deleted rows leave the inbox and the unread count the same way a dismissed one does. Nothing on an open screen breaks.

**How long a retry is safe.** The idempotency key is remembered for as long as the send's record is kept: at least `SENDS_DAYS` after the send, and for as long as any notification or push record from that send is still stored. With the defaults that is a year for a send that reached an inbox. During that time a repeat of the same call answers the first result and sends nothing. After the record is deleted, the service has forgotten the key, and the same call is a new send. So keep `SENDS_DAYS` longer than the longest time any of your systems might retry a send. It cannot be set below 2 days.

Retention is also the answer for personal data that isn't tied to one person's deletion request. A condition can carry personal values (`user.email == '…'`), and it is stored with its send's record until that record is deleted. To delete one person's data now, use [`forget_person`](#erase-a-person).

## What it doesn't do yet

- **Native client enrollment is iOS, Android, and macOS.** The service side of enrollment is done, the web client is this crate's `browser` module, and the native client — iOS's and macOS's APNs token and Android's FCM token alike — is the `native` module (all above). Windows and Linux apps receive no push, so they have nothing to tap.
- **No record of who opened what.** A tap reaches your app's code, and nothing reports it to the service: there is no opened count per send. Mark the item read from your own screen when the tap lands, as below.
- **Action buttons are web and Apple only, and Apple's aren't registered yet.** `web.actions` buttons work today. An Apple notification shows buttons only for a category the app registered with the system, which leptoskit doesn't do yet; the receiver already reports a pressed button's id when an app registers its own. Android sends carry no buttons.
- **No delivery receipts.** The service knows what it stored and what the provider accepted; it doesn't track who saw a banner. `accepted` is as far as status goes.
- **No per-device state.** Read, dismissed, and cancelled state is per person, shared across their devices by design.
- **Translations are yours to write.** [Variants](#say-it-in-each-persons-language) pick the right text for each person, but the service does not translate anything, and it has no localization keys that a device resolves from its own string tables. [Local time](#send-at-each-persons-local-time) is a single date and time; a send that repeats every morning is a send your server makes each day.
- **Person attributes are the app's own statements.** Segments read what your server set — plan, country, spend. Attributes are set in batches by the app's server only; a person cannot describe themselves into an audience.
- **Device details are what the device reports.** The platform, locale, time zone, and app version are taken as each client sends them, checked for shape and never verified, and a device's time zone is not checked against a time zone database when it enrolls (a [local-time send](#send-at-each-persons-local-time) skips one it does not know). There is no OS version or device model, `'apns'` doesn't tell an iPhone from a Mac, and a web app has a version only if you pass one. Windows and Linux devices can't enroll, so device conditions never reach them.

The engineering notes in the repository describe the transport work that follows.

## Deploying

The service ships the way [services](32-services.md#deploying) do: `leptoskit build` builds its binary, and the deploy targets run it beside your app with its own database. Its `healthz` and `readyz` come from the service framework, and its migrations apply when it starts.

### Upgrading a large database

A new version of the service can change its tables, and it makes those changes when it starts, before it serves anything. On a small database that takes no time you would notice. On a large one a few of the changes take as long as copying or indexing the table, and the service is not serving while they run. Plan a quiet moment for these:

| Change                                                            | SQLite                                                                                            | Postgres                                                                                                                   |
| ----------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `topics_and_fanout` (adds fanout state to send records)           | copies every send record and builds an index over every notification                              | builds a unique index over every notification and rewrites every send record; writes to both tables wait until it finishes |
| `message_options` (adds a notification's kind and options)        | copies every notification and rebuilds its indexes                                                | reads every notification once to check the new column; nothing can read or write notifications until it finishes           |
| `scale_indexes` (indexes for the unread count and the push queue) | builds one index over every notification and two over every push record; writes wait, reads go on | the same three builds; writes to those two tables wait, reads go on. You can build them ahead with no waiting, as below    |

The other changes add empty tables or empty columns and take no measurable time at any size. That includes `endpoint_seen`, which adds one empty column recording when each device was last heard from.

On Postgres you can take the wait out of `scale_indexes` by building its three indexes yourself before you upgrade, while the old version is still running. Build each with `CREATE INDEX CONCURRENTLY`, under exactly the name and columns the migration file uses, and check that each one finished valid. The migration then finds them and only has a moment's work left. The migration file's own comment has the details.

Rolling a change back is for emergencies, and some rollbacks lose data: the comment at the top of each `down.sql` says what. Rolling `message_options` back cancels every silent data message, because the older version could not tell them from ordinary notifications and would have shown them in people's inboxes.
