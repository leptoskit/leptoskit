# Audiences

Naming every recipient works until your audience chooses itself or is too large to list. This page covers the other ways to say who a send is for: a topic's subscribers, the people a condition matches, a segment built from attributes your server sets, everyone the service knows, and the devices people enrolled. It also covers sending at each person's own local time, looking at an audience before you send to it, and erasing a person. All of it is the same on every platform, because an audience is made of people, and a person's inbox follows them to whatever device they sign in on.

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
        ..SendSpec::default()
    },
    target: SendTarget::Topic("posts.alice".into()),
    validate_only: false,
})
.await?;
```

A topic names 1 to 128 bytes of the characters FCM accepts — letters, digits, `-`, `.`, `_`, `~`, `%` — and a person holds at most 500 of them. Batching is idempotent: people who already hold the topic change nothing and count nothing, and people already at their 500 are skipped rather than refusing the whole batch. The answer is a `TopicChanges`: `subscribed` is how many people newly hold the topic, and `skipped_full` how many were left out because they were at the limit. Topics belong to people, not devices: every device signed in as the person receives what the person receives, the same way the inbox does.

Because the audience can be any size, the send doesn't hold it in one transaction. `Send` records the send and enqueues a fanout job, then answers immediately with state `fanning_out`. The job walks the audience in batches of 500, ordered by person id, writing each batch's rows — and its push deliveries, and the live updates open screens follow — exactly as a recipients send does: opt-outs skipped and counted, schedules and expiry honored. The audience is snapshotted as the fanout reaches each person, so someone who subscribes after it passed them by waits for the next send. A crashed or replayed batch never duplicates: the send holds one row per person by a database constraint.

Unsubscribe the same way — `UnsubscribeFromTopic { topic, persons }` in batches — and [`send_status`](sending.md#track-a-large-send) follows a large send's progress.

## Let people subscribe themselves

Your server can subscribe anyone; a person pressing a switch in your app should only be able to join the audiences you published as joinable. That boundary is configuration: topics are open to self-subscription only when the operator declares them open, in the service's environment — patterns like `news.*`:

```bash
LEPTOSKIT_NOTIFICATIONS_OPEN_TOPICS=news.*, inbox
```

Nothing is open by default, and a pattern that can't match a topic name refuses to start, named. `*` stands for any run of topic characters: `news.*` matches `news.breaking` but not `news`. The reason for the gate is whose choice a subscription is — your server follows your product's logic, while a person's own call should only reach the topics you said they could reach.

The person's own calls need no list and name no one but themselves — the person is read from the app's forwarded identity, never from an argument, so nobody can subscribe anyone else:

```rust
leptoskit_rpc::client::call(leptoskit_notifications::Subscribe { topic: "news.breaking".into() }).await?;
leptoskit_rpc::client::call(leptoskit_notifications::Unsubscribe { topic: "news.breaking".into() }).await?;
```

A closed topic is refused with a clear forbidden error; a bad topic name is invalid. Unsubscribing is always allowed — leaving an audience is the person's to do, whatever subscribed them — and `my_topics` is their own list, live and paginated, so a screen follows the app's batch subscriptions and the person's own joins and leaves alike. A follow switch is the whole client half:

```rust
use leptoskit::prelude::*;
use leptoskit_rpc::client::call;
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
        ..SendSpec::default()
    },
    target: SendTarget::Condition(
        "'news.sports' in topics && 'alerts.local' in topics".into(),
    ),
    validate_only: false,
})
.await?;
```

The grammar is `&&`, `||`, `!`, and parentheses over the predicates below — and the `device.` predicates in [Target devices](#target-devices) — with `!` binding tightest, then `&&`, then `||`. At most five distinct topics, at most ten attribute and device predicates together, nesting at most eight levels deep, and at most 1,024 bytes. The five-topic limit is FCM's own; the others are this service's. All are checked on both sides, with errors that name the byte they gave up at.

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

**From a parsed condition back to text.** A screen that lets someone build an audience holds the condition as a tree, not as text, and has to hand the service text. `Condition` prints itself: `to_string()` writes the canonical spelling, with spaces around operators, parentheses only where the grouping needs them, and every value quoted the way `quoted` writes it. Parsing that text gives back the same tree, for every tree the parser can produce:

```rust
use leptoskit_notifications::condition::Condition;

let parsed = Condition::parse("user.city=='New York'&&('a' in topics||'b' in topics)")?;
assert_eq!(
    parsed.to_string(),
    "user.city == 'New York' && ('a' in topics || 'b' in topics)",
);
assert_eq!(Condition::parse(&parsed.to_string())?, parsed);
```

Every attribute value the service stores can be written as a quoted value, the empty value included (`''`), so there is no stored value a condition cannot name.

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

Keys are 1 to 64 bytes of `a-z`, `0-9`, and `_`; values are strings of at most 256 bytes; a person holds at most 50 attributes, and a key past that bound is skipped rather than refusing the batch. The answer is a `PersonAttributeChanges`: `registered` counts the people the service had not known before, `set` the values written or changed, `removed` the values removed, and `skipped_full` the values left out because their person was at the limit. The call is idempotent — sending the same value again changes nothing and counts nothing — and `remove` is the last word: a key in both `set` and `remove` is removed, never set. One call names at most 100 keys across `set` and `remove`; a larger change is refused as a bad request, so split it into several calls.

Two keys mean something to the service itself: `locale` is the language a person reads [translated sends](shaping-messages.md#say-it-in-each-persons-language) in, and `time_zone` is where they are for [local-time sends](#send-at-each-persons-local-time). Both are ordinary attributes that conditions can read too. Set them when your app knows better than the person's device does.

Setting an attribute also registers the person with the service, because an attribute is one of the things that makes someone an audience: the registry the service keeps is who "everyone" means. Someone who subscribes to a topic, sets a category preference, or enrolls a push endpoint is registered too — including by the migration that adds the registry, which backfills it from those tables. Someone who only ever received a recipients send is not registered: naming people in a send never puts them in an audience.

### Erase a person

When someone deletes their account or withdraws consent, you need the service to hold nothing more about them — not only to stop sending. `ForgetPerson` erases everything it has for one person:

```rust
call_as_app(leptoskit_notifications::ForgetPerson { person: alice_id }).await?;
```

That removes their inbox, their enrolled devices and the push credentials stored for them, the record of what was pushed to those devices, their category preferences, their topic subscriptions, their attributes, and their place in the registry. Nothing more is pushed to their devices, no audience reaches them, and their inbox is empty. It is safe to call twice, and for someone the service never knew.

A long inbox is erased in the background, a batch at a time, so the call returns quickly whatever the person's history; the rest follows within moments. A send's own totals are not rewritten — "delivered to 1,204 people" stays 1,204.

It stays in effect. Their phone or browser learns at its next launch that its enrollment is gone and turns notifications off there, instead of enrolling again ([what happens at launch](push-setup.md#when-the-server-revokes-a-device)). The service hears of the person again only if something new happens: they turn notifications on, set a preference, or subscribe to a topic themselves, or your server names them in a send, a subscription, or an attribute call. If someone must stay out — a withdrawn consent, rather than a deleted account nobody can sign in to — stop offering them those switches and stop naming them; the service keeps no list of people it was told to forget, because that list would be data about them too.

## Target a segment

Attributes and topics mix freely in one condition, which is the whole point: a segment is "people like that, who also asked for this".

```rust
let sent = call_as_app(leptoskit_notifications::Send {
    idempotency_key: format!("pro-de-launch-{launch_id}"),
    spec: SendSpec {
        category: "product".into(),
        title: "New in your plan".into(),
        body: "The launch post, in your language.".into(),
        ..SendSpec::default()
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
        ..SendSpec::default()
    },
    target: SendTarget::Everyone,
    validate_only: true,
})
.await?;
```

Treat this target with the respect a big lever deserves. It fans out in the same batches as any large send, so the service stays healthy however many people it knows — but every registered person gets a row, and every enrolled device gets a push, in one send. Two courtesies are built in: people who opted out of the send's category are skipped and counted, and `validate_only` (as above) answers how many people the send would reach right now, before anything is written. `forget_person` is the other half of this: a person your server forgot is not part of "everyone" anymore, whatever their history.

Everyone is not a condition, and it can't be narrowed: `SendTarget::Everyone` is the whole registry, every time. When you want most people, say which ones — a condition over attributes — because a send to everyone is a promise to every person in it.

## What a device says about itself

You want to reach "iPhones still on version 2.0" or "devices set to French", so the service needs to know those things about each device — not about each person, because one person's phone and laptop can differ. Every enrollment carries three details the device reports about itself, and your app writes no code for them:

| Detail        | iOS and macOS                                 | Android                           | Web                                                        |
| ------------- | --------------------------------------------- | --------------------------------- | ---------------------------------------------------------- |
| `locale`      | the first of the person's preferred languages | the default locale's language tag | `navigator.language`                                       |
| `time_zone`   | the current time zone's identifier            | the default time zone's id        | `Intl.DateTimeFormat().resolvedOptions().timeZone`         |
| `app_version` | the app's `CFBundleShortVersionString`        | the package's version name        | only what you pass to `EnrollOptions::app_version`, if any |

They are sent when the device enrolls and again whenever they change: on iOS, Android, and macOS the `native` module's launch `reconcile` reads them every launch and sends them only when they differ from what it last sent, so an app update or a language change reaches the service at the next launch and an unchanged launch costs one small read; on the web, `reconcile` and `enroll` send them each time. Windows and Linux have no enrollment yet, so they have no devices to describe.

The service checks each detail's shape: the locale is a BCP 47 tag of at most 35 bytes, stored in canonical case (`en_gb` becomes `en-GB`); the time zone is an IANA-style name of at most 64 bytes — checked for shape only, not against a time zone database, because a device's database can be newer than the server's; the app version is free text of at most 64 bytes. A client drops a detail that fails its check rather than lose its enrollment over it, and a request that sends a bad one directly is refused with the detail named. Each re-enrollment replaces what the endpoint said before; a change to the details alone never rotates the endpoint's credential, so pushes already queued for it carry on untouched.

The service also uses the language and the time zone itself: a [translated send](shaping-messages.md#say-it-in-each-persons-language) or a [local-time send](#send-at-each-persons-local-time) reads them from the device it heard from most recently, when your server has not set the person's `locale` or `time_zone` attribute. "Heard from" means the device enrolled, changed one of these details, or (on the web) renewed its enrollment at launch.

Details belong to the endpoint and are read back only by its owner: `push_endpoints` answers each endpoint's `device` beside its platform, and nothing your server or anyone else reads — delivery status, test-send outcomes — carries them. The condition grammar reads them, below.

## Target devices

Some messages are about a device, not a person: "please update" belongs on the phones still running the old version, and a French announcement on the devices set to French. Conditions read what each enrolled device [says about itself](#what-a-device-says-about-itself) with `device.` predicates, beside `user.` and topics:

```rust
let sent = call_as_app(leptoskit_notifications::Send {
    idempotency_key: "update-to-2.1".into(),
    spec: SendSpec {
        category: "app".into(),
        title: "Update available".into(),
        body: "Version 2.1 fixes the sign-in bug. Update from the App Store.".into(),
        ..SendSpec::default()
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

`send_status` counts a row as `delivered` when it is written, not when it appears, so a local-time send to a topic reads `complete` once every row exists, while some of them are still waiting for their morning. `validate_only` checks the time and the default zone and counts the audience. [`send_test`](troubleshooting.md#test-on-a-device) ignores `local_time` and pushes at once.

**Cancelling** works while anyone is still waiting. Once the first zone has seen the notification, a cancel stops it for everyone whose time has not come yet and leaves the rest alone; the answer is how many it stopped. Cancelling again answers `0`. When nobody is waiting any more, the cancel is refused with a conflict, as for any send people have already seen. For a topic, condition, or everyone send with more than 500 rows still waiting, the rows are stopped in the background, a batch at a time. A row whose time comes in the few seconds before its batch is reached can show and be pushed before it is withdrawn. A local-time send that was cancelled part-way still answers the state `complete` from `send_status`, not `cancelled`: the people who had already seen it keep it.

Variants and local time work together: one send can say good morning in each person's language at each person's nine o'clock.

## Know your audience before you send

Picking an audience blind is how a message meant for a hundred people goes to a hundred thousand. Before a send, a person composing it needs to see what there is to choose from and about how many people a choice would reach. The audience catalogue answers both, to your app's server only:

```rust
use leptoskit_notifications::catalogue::{
    AttributeValues, EstimateAudience, NotificationsOverview, Topics,
};
use leptoskit_rpc::PageArgs;

let first = PageArgs { after: None, until: None, limit: 20 };

// What the service knows, in one call.
let overview = call_as_app(NotificationsOverview {}).await?;

// Topics that start with "news.", with their subscriber counts.
let topics = call_as_app(Topics { page: first.clone(), prefix: Some("news.".into()) }).await?;

// The values people hold for one attribute.
let plans = call_as_app(AttributeValues { key: "plan".into(), page: first, prefix: None }).await?;

// About how many people a target would reach.
let estimate = call_as_app(EstimateAudience {
    target: SendTarget::Condition("user.plan == 'pro' && 'news.sports' in topics".into()),
    category: Some("news".into()),
})
.await?;
```

| Call                     | Answers                                                                                                                                                                                                   |
| ------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `notifications_overview` | how many people the service knows, how many have a working device on each platform and whether that platform is configured, how many topics exist, and which topic patterns are open to self-subscription |
| `topics`                 | topics with at least one subscriber, in name order, each with its subscriber count and whether it is open; `prefix` narrows the list                                                                      |
| `attribute_keys`         | the attribute keys anyone holds, each with how many people hold it                                                                                                                                        |
| `attribute_values`       | the values people hold for one key, each with how many people hold it; `prefix` narrows the list                                                                                                          |
| `categories`             | the categories you have sent in or that anyone set a preference for, each with how many people opted out and when you last sent in it                                                                     |
| `estimate_audience`      | for a target: how many people it matches, how many of them opted out of the category, how many would get it, and how many of those have a device on each platform                                         |
| `person_endpoints`       | one person's devices, as the person's own device list shows them: platform, dates, and the details the device reported                                                                                    |

**Counts stop at a cap and say so.** Counting every subscriber of a topic with ten million of them would take as long as reading them. So every count is a `Count { count, exact }`: when `exact` is `false`, the real number is larger than `count`, and a screen should show "100,000+". A topic, attribute, or category count stops at 100,000. The overview's counts of people stop at 1,000,000 and its count of topics at 1,000.

**An estimate is exact for a small audience and a floor for a large one.** `estimate_audience` walks the audience the way a send does, for at most 5,000 people. If it reaches the end, `exact` is `true` and the numbers are what a send right now would do. If it doesn't, `exact` is `false` and the numbers count only the people it walked: read them as "at least". It is meant to be asked as someone edits a target. For the exact size of a large audience, use `validate_only` (in [Track a large send](sending.md#track-a-large-send)), which walks all of it. A person whose only devices have been [revoked or have stopped working](push-setup.md#when-the-server-revokes-a-device) is not counted in the per-platform reach.

**Lists are pages of at most 50.** Each list answers a `Page`; pass its `end` as the next `after`. Names come in the order your database sorts text. On Postgres a `prefix` search looks at up to 500 names per call, so a call can answer fewer rows than you asked for with `has_more` set: keep paging.

**No credential is ever answered.** `person_endpoints` shows the same summary the person's own device list shows. Device tokens and browser subscription keys never leave the service.

**Attribute values are your data about people.** A value can be personal (a city, an email domain). These calls are for your server and for operators you trust with that data.

A topic with no subscribers and an attribute nobody holds do not appear: the service only knows what is stored.
