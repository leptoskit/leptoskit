# Notifications

You want to tell someone something after they have left the screen: an invoice was paid, a ride is arriving, someone replied. The usual route hands your users and your sending patterns to a third-party push platform, and binds delivery to OS credentials you have to enrol for. leptoskit has a notifications **service** you add with one table in `leptoskit.toml`: a small program with its own database that runs beside your app's server, stores one durable notification per recipient, and serves each person their own inbox. Your server sends to explicit recipients, to a topic's subscribers, to the people a condition matches over topics and the attributes you set, or to everyone the service knows, and when you configure a push transport, the same send also wakes the devices its recipients enrolled — APNs, FCM, and Web Push, first-party, with your app's own credentials and no management vendor in between. Each send can be shaped — priority, an image, a sound, a badge, a click link, per-platform overrides, or a silent data message — and one call tests a spec on a real device before a campaign.

This page shows how to add it, send from your server, read the inbox on every platform, manage categories and schedules, and stand up push delivery. It also says plainly what it does not do yet: **client enrollment beyond the web, iOS, Android, and macOS**, push tap routing, device attributes (a person's platform, locale, or app version as a segment), and any claim that a provider's acceptance means a person saw a banner.

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

The `idempotency_key` is how retries stay safe: derive one stable key per logical send (like the invoice id above) and reuse it on retry. The same key with the same intent answers the first run's result and writes nothing — across restarts, and past the send's expiry, because the send's bookkeeping lives in the service's database — and the same key with a different intent is refused with a conflict, so a reused key can't quietly send something else.

`SendSpec::visible_at` and `expires_at` take RFC 3339 instants, at most a year ahead; a `visible_at` in the past means now, so the notification shows from the moment it is sent and its push has its full delivery window from then. `data` is a JSON object of at most 4 KiB your screens read to route a tap; everything is length-checked on both sides, so a screen can reject bad input without a round trip. The whole spec is bounded as the service stores it, too: at most 8 KiB as JSON, with its options at most 4 KiB of that, so a spec that passes the checks is always one the service can keep, whatever its target.

### Cancel a scheduled send

A scheduled send can be cancelled until it becomes visible:

```rust
let cancelled: u32 = call_as_app(leptoskit_notifications::Cancel {
    idempotency_key: "launch-reminder".into(),
})
.await?;
```

After it's due the service refuses with a conflict — what someone has already seen can't be unsent. Cancelling again answers `0`, and so does cancelling a send every recipient opted out of: it delivered nothing, but the service knows the key, and cancelling it is still cancelling it.

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

The grammar is `&&`, `||`, `!`, and parentheses over the predicates below, with `!` binding tightest, then `&&`, then `||`. At most five distinct topics, at most ten attribute predicates, nesting at most eight levels deep, and at most 1,024 bytes — FCM's bounds, checked on both sides, with errors that name the byte they gave up at.

| Predicate                            | True when                                                   |
| ------------------------------------ | ----------------------------------------------------------- |
| `'name' in topics`                   | the person holds the topic                                  |
| `user.<key> == 'value'`              | the app set the attribute to exactly that value             |
| `user.<key> != 'value'`              | the app set the attribute to something else                 |
| `user.<key> in ['a', 'b']`           | the attribute is one of the listed values                   |
| `has(user.<key>)`                    | the app set the attribute, whatever its value               |
| `user.<key> < '10'` (also `<= > >=`) | both sides parse as finite numbers and the comparison holds |

An attribute the person doesn't have answers false to every predicate, `!=` and `in` included: nobody matches a segment merely for lacking the attribute. Numeric comparisons are numbers, not text — `user.version > '9'` is true for `10` — and a value that doesn't parse as a finite number answers false; `NaN` and `inf` are refused as literals at parse time. Values in quotes are at most 256 bytes with no whitespace inside, and keys are `a-z`, `0-9`, and `_`, up to 64 bytes — the same rules `set_person_attributes` enforces, so what you can send is what you can ask about.

One shape is refused outright: a condition that already holds for a person with no topics and no attributes, like `!('a' in topics)`, is an "everyone" audience in disguise — the plain target below says that with a name, so the condition grammar makes you use it. Everything else anchors on something: a topic, or an attribute predicate such as `user.plan == 'pro'`. Condition sends fan out like topic sends: same batches, same snapshot-as-reached, same idempotency. Which index a condition's fanout walks follows from its anchor — a condition that cannot match without one of its topics walks the topic's subscribers, and one anchored only on attributes walks the person registry below.

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

Keys are 1 to 64 bytes of `a-z`, `0-9`, and `_`; values are strings of at most 256 bytes; a person holds at most 50 attributes, and a key past that bound is skipped and counted in `skipped_full` rather than refusing the batch. The call is idempotent — sending the same value again changes nothing and counts nothing — and `remove` is the last word: a key in both `set` and `remove` is removed, never set.

Setting an attribute also registers the person with the service, because an attribute is one of the things that makes someone an audience: the registry the service keeps is who "everyone" means. Someone who subscribes to a topic, sets a category preference, or enrolls a push endpoint is registered too — including by the migration that adds the registry, which backfills it from those tables. Someone who only ever received a recipients send is not registered: naming people in a send never puts them in an audience.

When a person should belong to no audience anymore — a deleted account, a withdrawn consent — `ForgetPerson` removes them from the registry, with their attributes and their topic subscriptions:

```rust
call_as_app(leptoskit_notifications::ForgetPerson { person: alice_id }).await?;
```

Their inbox rows stay as they are, and so do their push endpoints and category preferences: forgetting ends their audiences, not their history. A later attribute, subscription, preference, or enrollment registers them again.

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
| `kind: "data"`  | background push, `content-available`, no banner                    | data-only message, no banner                         | receiver shows nothing (see below)                                                     |
| `priority`      | `apns-priority` 10 or 5                                            | `android.priority` high or normal                    | `Urgency` header                                                                       |
| `image`         | needs a Notification Service Extension in your app                 | shown in the notification                            | shown in the notification                                                              |
| `sound`         | `aps.sound`, the named file                                        | `android.notification.sound`                         | `silent: true` on the notification (browsers play no sound by default)                 |
| `badge`         | `aps.badge`                                                        | `notification_count`                                 | the receiver calls `navigator.setAppBadge`                                             |
| `link`          | carried in the payload for your app to route                       | carried in the data map                              | the receiver opens it on click (your origin only)                                      |
| `android { … }` | —                                                                  | channel, color, icon, tag, sticky                    | —                                                                                      |
| `apple { … }`   | subtitle, thread-id, category, interruption-level, relevance-score | mirrored into the APNs block FCM serves for iOS apps | —                                                                                      |
| `web { … }`     | —                                                                  | —                                                    | icon, badge shape, up to two actions buttons, require-interaction, direction, language |

The per-platform blocks only shape that platform: `android.channel_id` posts through an Android channel your app must have created (the local-notifications plugin does), `apple.interruption_level` is what Focus modes read, and `web.actions` become `showNotification` action buttons. Everything is checked on both sides with errors that name the field — a color must be `#RRGGBB`, an image or link at most 1,024 bytes of `https` URL (or, for a link, a path starting with `/`), a sound name 1–64 bytes of letters, digits, `-`, `_`, and `.`.

**When the payload is too big.** Every provider caps the message — 4 KiB for APNs and FCM, similar for Web Push — and options count toward it. When a send doesn't fit, the service drops the optional extras in a fixed order before giving up on anything you'd miss: the per-platform blocks first, then the image, then the sound, badge, and link, then the send's `data`, down to an id-only envelope that still shows the title and body and still collapses by the notification's id. A data message never loses its silent flag in the fallback. If even the id-only envelope doesn't fit, the delivery ends as `oversized` without a request.

### Silent data messages

```rust
options: MessageOptions {
    kind: MessageKind::Data,
    ..MessageOptions::default()
},
```

`kind: "data"` makes the send a **background message**: no banner on any platform. The send's `data` reaches your app instead — on Android through FCM's data message, on iOS and macOS as a background `content-available` push your app receives in the background, on the web as a push the receiver service worker deliberately does not show. Use it to sync, prefetch, or correct state without interrupting anyone.

A data send still creates one durable row per recipient — the send's bookkeeping, not an inbox item. It never appears in the inbox, never counts toward `unread_count`, and can always be cancelled, because nobody ever saw it. Its push deliveries, status, idempotency, and fanout behave exactly like a visible send's.

Two honest warnings. Apple requires background pushes at priority 5, so a data send ignores a higher one. And browsers penalize pushes that show nothing — Chrome may stop delivering silent pushes to a person who hasn't opened the site in a while — so on the web, prefer a visible notification whenever the person would want to know.

### Badges

```rust
options: MessageOptions {
    badge: Some(BadgeSetting::Unread),
    ..MessageOptions::default()
},
```

`badge` sets the app badge the platform shows next to your icon. `BadgeSetting::Set(n)` sets that exact number — `Set(0)` clears it. `BadgeSetting::Unread` is the interesting one: the service resolves it **per recipient at push time** to that person's live unread count — the same number `unread_count` would answer right then. One send to a thousand people badges each of their own counts.

On the web the receiver service worker calls `navigator.setAppBadge(n)` where the browser supports it (and `clearAppBadge` for zero); where it doesn't, the count rides in the payload and nothing else happens. Data sends can carry a badge too — a silent badge correction is a classic background message.

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
	// Show your own "notifications are off" state; the OS shows nothing.
}

notifications
	.configure_channel(&Channel::new(
		Identifier::new("inbox")?,
		"Inbox".into(),
		"New items in your inbox.".into(),
		Importance::Default,
		true,
	)?)
	.await?;

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

This shows a banner only while your app runs, and only where the plugin is implemented: iOS and macOS through `UNUserNotificationCenter`, Android 26+ through `NotificationManager`, the browser through the `Notification` API, and Linux through D-Bus. **Windows returns `Unavailable`** — there is no Windows toast backend yet. On Android the channel must exist before a post (posts to a channel that doesn't are dropped), and Android 13+ also needs `POST_NOTIFICATIONS` — declared in your app's `android/app/src/main/AndroidManifest.xml` ([the `android/` folder is yours](../platforms/android.md)) and granted at runtime through `request_permission`, which uses the host's permission broker. On every platform a successful post doesn't prove the person saw anything: Focus modes, notification permissions, channel settings, and foreground presentation policy decide that. Scheduled **local** notifications, badges from the plugin, and click routing are not available everywhere either — see the [plugin inventory](../platforms/plugin-inventory.md) for what each platform does. None of this posting reaches a device where your app isn't running; that is what server push, below, is for.

## Push delivery

When your app isn't running on a device, nothing above can show a banner. The service can: configure a transport, enroll the person's devices, and every send that creates an inbox row also reaches those devices. A push never stands in for the durable row — it is a knock on the door of an inbox the device can open anyway.

### Configure a transport

Each transport is optional and configured through the service's environment, which `lk` passes through from your deploy configuration — provider credentials are secrets and belong there, never in the repo or in client code. A transport with all of its variables set is on; a transport with none is off (its platform is refused at enrollment, and the inbox is untouched); a transport with some set refuses to start, naming every missing variable, because half-configured looks like push works until the first send fails.

| Transport | Variables                                                                                                                                                                                                                                                                                             |
| --------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| APNs      | `LEPTOSKIT_NOTIFICATIONS_APNS_KEY_P8` (the `.p8` file's contents), `_APNS_KEY_ID`, `_APNS_TEAM_ID`, `_APNS_TOPIC` (your app's bundle id), `_APNS_ENVIRONMENT` (`sandbox` or `production`)                                                                                                             |
| FCM       | `LEPTOSKIT_NOTIFICATIONS_FCM_SERVICE_ACCOUNT` (the service-account JSON file's contents)                                                                                                                                                                                                              |
| Web Push  | `LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_VAPID_PRIVATE_KEY` and `_VAPID_PUBLIC_KEY` (the pair the ecosystem's own tools generate: 32 and 65 bytes, base64url), `_VAPID_SUBJECT` (a `mailto:` or `https:` contact), `_WEB_PUSH_EXTRA_HOSTS` (optional, comma-separated push services beyond the shipped ones) |

The APNs topic and environment, the FCM project, and the VAPID identity are all yours on the server; an enrollment carries only the device credential, so nothing a client sends can steer where a push goes. The FCM service account must be the JSON Google issues — an **RSA** private key, which OAuth signs RS256; the exchange endpoint is fixed to Google's own, and the `token_uri` inside the JSON is ignored. The VAPID pair is cross-checked at startup, so a mistyped half refuses to start rather than failing at every push service at once. An `_WEB_PUSH_EXTRA_HOSTS` entry that isn't a plain host name also refuses to start, named.

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

The service answers an opaque endpoint id — the credential itself never comes back, and never appears in a list, a status row, or an error. Registering a destination that account already holds answers the same endpoint again, so a retried enrollment makes no duplicate; a Web Push subscription whose browser rotated its keys updates the stored credential in place, keeping the endpoint and its queued deliveries. A person holds at most ten live endpoints. A destination another account holds, including an invalidated destination, is refused: signing out of the app on a device revokes that device's endpoints, and then the next account on it enrolls cleanly. Only the same owner can reactivate an invalidated destination, within the live endpoint limit. Revoking it releases the destination; its next registration gets a fresh id. Credential rotation and reactivation advance a version: queued deliveries follow the new credentials, and late responses to older credentials cannot invalidate the endpoint or change the new queue. Invalidated ownership records stay until explicitly revoked; lists remain paginated.

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
aps-environment = "development" # or "production"
```

The `aps-environment` line is the opt-in. It is a string — Apple's entitlement takes exactly `development` or `production`, not a boolean — and generation writes it into `.leptoskit/ios/App.entitlements`. It must match three things that have to agree: this setting, the provisioning profile your build signs with, and the notifications service's `LEPTOSKIT_NOTIFICATIONS_APNS_ENVIRONMENT` — a development-signed build registers with Apple's sandbox, and a TestFlight or store build registers with production. An app without the setting generates exactly as before, with no push entitlement: selecting the notifications plugin for local banners never turns an app into a push-enabled one. No provisioning or certificate access happens in the build.

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
	// Show your own "notifications are off" state; the OS shows nothing.
}

// Still in the gesture: register, wait the OS's bounded answer, enroll.
// The answer is an opaque endpoint id — persist it, with the opt-in flag,
// beside the session. Never persist the token itself.
let enrollment = native::enroll().await?;
```

At **launch**, once the person is signed in, call `native::reconcile(Some(&enrollment))` — with the enrollment you saved, and only with one: it is the record that this person opted in on this device. Reconciliation registers with APNs again — registration prompts nothing, Apple expects it every launch, and a device that never opted in is never registered, so a person who turned notifications off is not silently re-enabled. The answer is an explicit transition you persist:

- `Outcome::Kept` — the OS still issues the same token; the service answered the same endpoint id, and the refresh was one idempotent call. Keep what you saved.
- `Outcome::Rotated { current, retired }` — the OS now issues a different token. The module verifies the previous handle against this person's endpoint list and retires it under its owner — a rotated token never leaks an orphan endpoint — so you persist `current` and drop `retired`. When the person is at the ten-endpoint bound, the quota refusal is answered by retiring the known-stale handle and re-registering once, so rotation at the quota still completes. A `retired` handle that turns out to be a **previous account's leftover** — the session switched without the old account signing out — is reported as `Retirement::Stale`: it is dropped, never operated under the new identity, and no credential moves.
- `Outcome::Off` — you passed no previous enrollment, so nothing was registered and nothing asked; the cue to show the opt-in switch.

```rust,ignore
// At launch, without prompting:
match native::reconcile(Some(&enrollment)).await? {
	native::Outcome::Kept(kept) => enrollment = kept,
	native::Outcome::Rotated { current, .. } => enrollment = current,
	native::Outcome::Off => {/* show the switch; from its handler, enroll */}
}
```

Failures name their retry state rather than swallowing it: `EnrollError::Unconfirmed` means the OS did not answer within the host's bound — the previous enrollment stands untouched, and a stale token is never dressed up as success; `EnrollError::Enroll` carries the handle to keep retrying with (`None` when the attempt already retired it); `RetireFailed` and `OwnershipUnknown` leave the new enrollment live and name the old handle whose revoke to retry. A later `reconcile` retries an unfinished retirement by itself.

**Sign out by disabling.** While the person is still authenticated, `native::disable(&enrollment).await?` revokes this person's service enrollment with a stable idempotency key derived from the endpoint id — the same contract as the web's `browser::disable` — so a retried sign-out is safe. It never changes OS registration or permission; the token stays the device's, and the next account enrolls from clean. Clear the saved enrollment and opt-in once the revoke is acknowledged: reconciliation runs only on what you pass it.

Registration failures are surfaced, not guessed around: the OS's provisioning refusal comes back as an error carrying a stable code — `aps-environment` names the missing or mismatched entitlement — never the OS's own error text, a silent OS answers an explicit "unconfirmed" instead of hanging or replaying a prior token, and a cancelled wait gives up without losing the callback — the broker keeps it for the next read. There is no swizzling, no background daemon, and no token cached to storage.

**What a push does while the app is closed** is the OS's work: APNs wakes the system, which shows the alert from the payload — title and body, collapsed by the notification's id — and your durable inbox row is already in the service. What this does **not** include yet: routing a tap into a screen (the envelope's `data` is carried, but the tap-routing story is later work), and any claim of live APNs acceptance here — provider acceptance is only ever proven by a real device with real credentials, which this setup doesn't exercise.

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

The `push` line is the opt-in. Generation adds four things to the generated Android project and nothing else changes: the pinned `com.google.firebase:firebase-messaging` dependency, Google's services Gradle plugin (applied through `.leptoskit/android/gradle-plugins.txt`, which the current `leptoskit new` scaffold reads), the messaging service that receives FCM's token refreshes, and the `POST_NOTIFICATIONS` permission the local-notifications plugin already declares. An app without the line generates byte-for-byte as before, so selecting the plugin for local banners never turns an app into a push-enabled one.

Two things are yours, not leptoskit's: **`google-services.json`** — the file the Firebase console issues for your app's package id, placed at `android/app/google-services.json`. Generation stops and names the path when push is on and it's missing; leptoskit never generates or commits it. And the Gradle script reader: apps scaffolded before this setting get a generation error that names the exact lines to add to their app-owned `android/app/build.gradle.kts`.

At runtime the flow is the iOS flow verbatim — request permission, enroll from the gesture, reconcile at launch with the saved enrollment, disable on sign out:

```rust,ignore
let enrollment = native::enroll().await?;
```

What differs under the hood: Firebase answers with a registration token — printable ASCII of no assumed length, not hex — the generated messaging service forwards token **refreshes** to the same in-memory broker the registration task feeds, and a device without usable Firebase refuses instead of pretending. The refusal is a stable code, not SDK text: `firebase-not-configured` means the app generated without the push opt-in or has no `google-services.json` values (the Firebase classes are found reflectively, so an app that never opted in simply answers this), `os-refused` covers the rest, and `token-shape` means Firebase handed over something outside the contract's bounds. The token is held in memory only — Firebase hands it out again on the next launch — and its bytes never appear in a log, an error, or a `Debug` print.

A device at Google Play services' mercy is a real constraint worth knowing: delivery needs Play services on the device, and while your app is **foregrounded** an arriving push is data your app must compose itself (the local-notifications plugin, above). What a push does while the app is **closed** is Google's work: the message's notification block shows the title and body in the system tray, collapsed by the notification's id, and your durable inbox row is already in the service. A tap opens the app — routing it into a screen is the same later work as iOS's. And as everywhere: acceptance at the provider is only ever proven by a real device with real credentials, which this setup doesn't exercise.

### Enroll a macOS device

On a Mac the enrollment is the same `native` module as iOS and Android — the same `enroll`, `reconcile`, and `disable` calls, the same error shapes — and the OS's answer is an **APNs device token** again, lower-case hex of whatever length the OS chooses. The opt-in is the iOS-style entitlement switch, under the macOS table:

```toml
[dependencies]
leptoskit_notifications = { version = "0.3", features = ["native"] }

[native]
plugins = ["notifications"]

[native.macos]
aps-environment = "development" # or "production"
```

The `aps-environment` line is the opt-in. Generation writes Apple's macOS entitlement key — `com.apple.developer.aps-environment`, a string like iOS's — into `.leptoskit/macos/App.entitlements`, and both the local build's ad-hoc signature and a `leptoskit publish macos` distribution sign the app with it. It must match the notifications service's `LEPTOSKIT_NOTIFICATIONS_APNS_ENVIRONMENT`: a development-signed build registers with Apple's sandbox, and a notarized production build registers with production. An app without the setting generates exactly as before, with no push entitlement: selecting the notifications plugin for local banners never turns an app into a push-enabled one.

The generated macOS app delegate already forwards the OS's answers — `didRegisterForRemoteNotificationsWithDeviceToken` and `didFailToRegisterForRemoteNotificationsWithError` land in the notifications plugin's in-memory broker, the same one iOS uses, which survives the early callbacks that can arrive before the plugin and its guest exist. `REMOTE_REGISTER` calls `NSApplication.registerForRemoteNotifications` and waits the bounded answer; a silent OS answers "unconfirmed" instead of replaying a prior token, and the token's bytes never reach a log.

At runtime the flow is the iOS flow verbatim — request permission from a gesture, enroll from the gesture, reconcile at launch with the saved enrollment, disable on sign out:

```rust,ignore
let enrollment = native::enroll().await?;
```

One honest limitation: the stock **menu-bar host** (a `[native] mode = "menu-bar"` app that keeps the generated status-item lifecycle) carries no forwarding delegate, so its registration answers stay "unconfirmed" — a push-enabled app wants the standard windowed host, or its own delegate wired to the plugin's `noteDeviceToken` and `noteRegistrationFailure`. And what a push does while the app is **closed** is the OS's work, as on iOS: APNs shows the alert, your durable inbox row is already in the service, and routing a tap into a screen is still later work. As everywhere, provider acceptance is only ever proven by a real Mac with real credentials, which this setup doesn't exercise.

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

The route is the only URL a notification click may open, checked against the worker's own origin again at click time — payload content can never add a host, a scheme, or a path, and a route the builder refuses (empty, not a `/`-rooted path, scheme-relative, backslash, control characters) never ships. Register the script once at startup, the way [the web platform page](../platforms/web.md#service-workers) does it, and wait for it to control the page:

```rust,ignore
use leptoskit_notifications::browser;

let registration = browser::register("/sw.js", browser::RegisterOptions::default()).await?;
let _ = browser::ready().await?;
```

**Ask before you offer.** `browser::availability()` reads where push stands — `ready`, `insecure_context` (serve `https`; nothing in code works around it), `unsupported`, or `denied` (the person refused; link to the browser's site settings rather than prompting again). It never prompts, so a settings screen may call it on every render. **Prompting is the browser's answer to subscribing, and it happens only inside `enroll`** — the two calls are split by when they may run:

- **`browser::reconcile`**, on launch, once the person is signed in. It registers the subscription the browser already holds, if it holds one for your key, and answers `None` when it holds none — the app's cue to show the switch. Reading a subscription needs no permission, so this may run from any startup path.
- **`browser::enroll`**, from an explicit gesture — a button, never a mount or a render. It subscribes only when the browser holds no subscription, and that subscribe is the one call the browser gates on the gesture; reconciling at launch keeps the worker settled long before, so nothing asynchronous eats the gesture's activation window.

```rust,ignore
// At launch, after register and ready, without prompting:
let options = browser::EnrollOptions::new(vapid_public_key);
if browser::reconcile(&options).await?.is_none() {
    // Show the switch; from its click handler, with the operator's
    // VAPID public key — the private half never leaves the service:
    let enrollment = browser::enroll(&options).await?;
}
```

`enroll` checks availability first, waits for the **active** worker instead of racing its install, reuses the subscription the browser already holds (a second tap re-enrolls the same endpoint instead of prompting twice, and two racing calls converge on one subscription), and registers it under the person who is signed in — but only when that subscription was created for **your own VAPID key**: a subscription made for another sender's key can't receive your pushes, so claiming it would end in silent delivery failure, and it is refused (`KeyMismatch`) rather than replaced. When the receiver is registered under an explicit scope, name it in `EnrollOptions::worker_scope`: the match is exact — a worker registered for a broader scope never stands in for the one you asked for. It answers an opaque `Enrollment` — the endpoint's id — which you keep beside the person's session on the device.

**Sign out by disabling.** While the person is still authenticated, `browser::disable(&enrollment).await?` revokes this person's service enrollment. It never unsubscribes the browser or changes OS permission. The subscription remains reusable: re-enabling can reuse it, but this service requires enrollment before sending. Until the previous account revokes, another account cannot enroll that destination.

Disable requires a successful authenticated revoke acknowledgement. Failures, including not-found, stay explicit; no endpoint list can establish success. Retries use a stable key derived from the endpoint id, including after reload. Receipts are scoped to the authenticated caller and replay only while retained by the service; pruning an old receipt can leave a retry unconfirmed. Revocation stops future attempts at the service's documented revocation boundary; provider delivery already in flight and visible banners are not recalled. Origin-wide unsubscribe and permission revocation are separate actions in browser settings or the lower-level worker API.

**When the browser changes the subscription on its own** — a rotated or replaced subscription, the `pushsubscriptionchange` event — the worker cannot reconcile anything: it deliberately holds no credentials, so it has no identity to enroll with. The supported reconciliation is the next authenticated app launch calling `reconcile` again; a stale handle's `disable` leaves the replaced subscription alone, and the replaced subscription's endpoint dies at its push service, whose answer invalidates the stale record on its next delivery. Churn is bounded by the ten-endpoint limit, and the person can revoke the dead ones from their own list.

The receiver itself shows what the service pushed: the envelope's title and body under a tag built from the notification's id, so a repeat replaces rather than stacks; the send's `data` rides on the banner's own data, inert. A send's options reach it too — an image, a badge count (`navigator.setAppBadge` where the browser has it), a silent flag, and the web presentation set — and a **data message** (`lkdata`) shows nothing at all, leaving only the badge it asked to set. A push that is missing, oversized (past the service's 4 KiB record), malformed, or typed wrong still shows one truthful banner — "New notification", no invented text — under a stable tag, so a broken sender can never silently swallow a delivery. A click closes the banner, focuses an app window of this origin and opens the send's own link when it resolves to this origin — a path like `/invoice/42`, or an absolute URL of this origin — and otherwise brings it to your inbox route with the notification's id, or opens that route in a new window. A link anywhere else is refused at click time: payload content can never steer a click off the app's own origin. The worker holds no credentials and calls no server: nothing fetches, and **no read state changes until the authenticated app itself opens** and marks what it read.

### What a send does

When a recipient of a send has enrolled endpoints and your server has configured that platform's transport, the send enqueues one durable delivery per notification and endpoint in the send's own transaction. The push job attempts each one when the notification becomes visible — a scheduled send pushes at its `visible_at`, not before — and re-checks, at the moment of the attempt, everything that could have changed since the send: cancellation, expiry, the category opt-out the person may have flipped meanwhile, and the endpoint's liveness. A cancelled send never pushes; an expired one is skipped; an opt-out made after the send reaches the queue still stops the push.

Transient failures — a 429, a 5xx, a network error — retry with backoff, and with the provider's `Retry-After` honored when it asks for longer, up to eight attempts (eight is eight: the ninth never goes out), and then end as rejected with `attempts_exhausted`. The waits start at one minute and double, so the eighth attempt comes about two hours after the first: a provider outage of an hour loses no delivery. Permanent failures end at once with a reason code, and only the provider's own "unregistered" answer invalidates an endpoint — a plain 404 from FCM is a project-naming problem on your side, not a dead device — so nothing further is pushed to a dead credential and its other queued deliveries are skipped. Failure of one endpoint never rolls back the inbox row or the other endpoints' deliveries: the inbox row committed with the send, and push is best-effort on top of it.

Delivery retries stop at eight actual push requests, including requests refused for authentication. OAuth token exchanges are separate and do not spend push attempts. FCM refreshes once after a 401; the next push goes through the durable queue and its attempt budget. An exchange failure retries every minute with sanitized `operator_configuration` status. Neither provider diagnostics nor credentials appear in that status.

Every delivery has a deadline: the earlier of notification expiry and 24 hours after `visible_at`, including notifications with no expiry. Provider `Retry-After` accepts delta seconds and HTTP dates; invalid values use backoff. A provider delay is never shortened into an earlier push. If it exceeds the deadline, the queue wakes at the deadline and ends delivery (`expired` or `horizon_exceeded`) without another request. Unrepresentable arithmetic ends with `retry_overflow`. Local schedule, cancellation, expiry, and horizon decisions do not spend attempts. Transient and refused retries on every platform use exponential backoff starting at one minute — 1, 2, 4, and so on to 64 minutes — plus up to a quarter more of jitter that differs from delivery to delivery, and respect longer provider delays. See [FCM error codes](https://firebase.google.com/docs/cloud-messaging/error-codes).

The push job works through a send in rounds of at most a thousand attempts, each started within a minute, and a round hands whatever is left to the next at once, so a send to a million devices is never one long job. A delivery whose stored options this server can't read — written by a newer version, before a rollback — waits with `operator_configuration` status rather than pushing something else, and goes out once a server that reads them is back, within its delivery window.

A send's [options](#shape-a-message) ride the same deliveries, and when a payload is over its provider's limit the extras go in a fixed order — per-platform blocks, image, sound, badge, and link, then the send's `data` — before the id-only envelope, and only then a clear `oversized` end without a request.

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

The spec is a full `SendSpec` — options included, silent data messages included — so the device receives exactly what a real send to it would carry. The blast radius is the person's own enrollment: at most their ten live endpoints, each attempted exactly once, one provider request each. A transient failure answers `rejected` with no reason and the retry is calling again; an endpoint id that isn't the person's own live one is `404`. There is no idempotency key to reuse — a test answers, it doesn't record — so a repeated call, a retried request included, pushes again. While the providers answer, the service holds nothing open, so a slow provider doesn't hold up its other work.

Read what happened from your server, per endpoint, with `call_as_app`:

```rust,ignore
let page = call_as_app(leptoskit_notifications::PushDeliveries {
    send_key: "invoice-paid-42".into(),
    page: Default::default(),
}).await?;
```

Each item names the notification, the endpoint's opaque id, the platform, a state — `enqueued`, `retrying`, `accepted`, `rejected`, or `skipped` — and, when it left the queue, a sanitized reason code. **`accepted` is the provider taking the message, not a person seeing one.** There are no delivery receipts here: focus modes, notification permissions, and quiet hours all decide what a delivered push becomes, and the service can't see past the provider's answer.

Delivery is **at least once**. An attempt whose answer was lost retries, so a device may see the same notification twice; every attempt carries the notification's id as its collapse id — `apns-collapse-id`, the FCM collapse key, the Web Push `Topic` — so a repeat _usually_ replaces its predecessor instead of stacking. Collapse is best-effort: a provider that doesn't coalesce, or a device that was offline for both attempts, can still show two banners, and the id-only fallback keeps its collapse id too. The payload is the visible title and body plus the notification's id for routing; the send's `data` rides along where it fits and is dropped, down to an id-only envelope, when the provider's payload limit says so (limits are checked on the encoded bytes, not the raw strings). A send's `expires_at` is the providers' TTL — APNs `apns-expiration`, the FCM `ttl`, the Web Push `TTL` — so a provider holds a message no longer than the notification is worth delivering. Nothing executable ever rides in a push, and your `data` keys that would collide with the envelope's own id key, or with the platform's reserved words, are dropped rather than sent.

### Web Push hosts and security

A Web Push subscription's endpoint is attacker-shaped material — anyone who can drive an enrollment can hand the service a URL — so the service pushes only to the shipped push services' hosts: Google's (`fcm.googleapis.com`, where Chrome and Android Chrome deliver), Mozilla's (`updates.push.services.mozilla.com`), and Apple's (`webpush.apple.com`). Lookalike hosts (`fcm.googleapis.com.attacker.test`), private and reserved addresses (the full IANA special-purpose registries, transition networks included), credentials-in-URL, and non-443 ports are refused at enrollment. To push through another operator-run push service, name it in `_WEB_PUSH_EXTRA_HOSTS`; those are resolved within a bounded time and validated at connection time (every address must be globally routable), and the connection is pinned to the exact addresses that passed — re-resolved on each attempt, so an address rotation is picked up without a rebind hole. Requests are never redirected: a push service that answers with a redirect has, as far as this service is concerned, not taken the message.

### Platforms

Push reaches a device through its platform's service: iOS and macOS through APNs, Android through FCM, and the web through the browser's own push service (Web Push with VAPID) — the browser's client half is [this crate's `browser` module](#enroll-a-browser), and the native client half for iOS, Android, and macOS is [the `native` module](#enroll-an-ios-device). **Windows and Linux have no enrollment story in leptoskit yet**, and HarmonyOS and the watch surfaces are out of scope for push — their inbox is durable retrieval, as everywhere. Background delivery on Android also needs the app's own OS setup there (`google-services.json`, and `[native.android] push` checked).

## What it doesn't do yet

- **Native client enrollment is iOS, Android, and macOS.** The service side of enrollment is done, the web client is this crate's `browser` module, and the native client — iOS's and macOS's APNs token and Android's FCM token alike — is the `native` module (all above). Push notification **tap routing** into a screen is later work, everywhere: a send's `link` is carried in every payload today, but opening it is each app's own code.
- **No delivery receipts.** The service knows what it stored and what the provider accepted; it doesn't track who saw a banner. `accepted` is as far as status goes.
- **No per-device state.** Read, dismissed, and cancelled state is per person, shared across their devices by design.
- **No localization or recipient-local scheduling.** A send's title and body are the same for everyone it reaches, and a scheduled send fires on the service's clock, not each recipient's timezone.
- **Person attributes are the app's own statements.** Segments read what your server set — plan, country, spend — and nothing about the device: platform, locale, app version, and timezone are not attributes yet, and there is no per-device targeting at all. Attributes are set in batches by the app's server only; a person cannot describe themselves into an audience.

The engineering notes in the repository describe the transport work that follows.

## Deploying

The service ships the way [services](32-services.md#deploying) do: `leptoskit build` builds its binary, and the deploy targets run it beside your app with its own database. Its `healthz` and `readyz` come from the service framework, and its migrations apply when it starts.
