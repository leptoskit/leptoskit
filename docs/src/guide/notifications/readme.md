# Notifications

You want to tell someone something after they have left the screen: an invoice was paid, a ride is arriving, someone replied. The usual route hands your users and your sending patterns to a third-party push platform. leptoskit has a notifications **service** you add with one table in `leptoskit.toml`. It is a small program with its own database that runs beside your app's server. It stores one durable notification per recipient, serves each person their own inbox, and, once you configure a push transport, sends the same notification to the devices that person enrolled over APNs, FCM, and Web Push, with your app's own credentials.

This chapter covers the whole service. If you are moving an app off Firebase Cloud Messaging, start with [Coming from Firebase](coming-from-firebase.md).

## How it fits together

Five words carry the rest of the chapter:

- A **person** is one of your app's users, named by the id your sign-in gives them. The service never sees their session, email, or roles.
- A **send** is one call your server makes: a message, an audience, and an idempotency key that makes a retry safe.
- An **inbox row** is one person's copy of a send. Read, opened, and dismissed state lives on the row, so it is the same on every device the person signs in on.
- An **endpoint** is one device or browser a person enrolled for push. The service keeps its credential and answers only an opaque id.
- A **delivery** is one push of one inbox row to one endpoint. It is attempted in the background, retried when the provider asks, and ends with a state and a reason code your server can read.

Only your app's server sends. Each person reads and changes only their own inbox, preferences, topics, and devices.

## What works where

| Platform                 | Read the inbox, live | Push while the app is closed   | Know what was tapped | Local banner while the app runs |
| ------------------------ | -------------------- | ------------------------------ | -------------------- | ------------------------------- |
| Web                      | Yes                  | Yes, Web Push                  | Yes                  | Yes                             |
| iOS                      | Yes                  | Yes, APNs                      | Yes                  | Yes                             |
| Android                  | Yes                  | Yes, FCM (needs Play services) | Yes                  | Yes, Android 8 and later        |
| macOS                    | Yes                  | Yes, APNs (windowed apps)      | Yes                  | Yes                             |
| Windows                  | Yes                  | No                             | No                   | No                              |
| Linux                    | Yes                  | No                             | No                   | Yes                             |
| Terminal                 | Yes                  | No                             | No                   | No                              |
| HarmonyOS (experimental) | Yes                  | No                             | No                   | No                              |

The experimental Wear OS and watchOS surfaces have no push enrollment, and this chapter makes no claim about them.

"Read the inbox" means the screen calls the service like any other [service](../32-services.md) and follows it live. [Push setup](push-setup.md) and [Receiving on the device](receiving.md) say what each platform does when a push arrives.

## Add the service

Name it in your app's `leptoskit.toml`:

```toml
[services.notifications]
crate = "leptoskit_notifications"
version = "0.3"
trust = "first-party"
```

That's all `leptoskit dev` and `leptoskit serve` need: they build the service, give it a database of its own on the same engine as your app's (SQLite, or Postgres when your app uses Postgres — add `features = ["service", "postgres"]` for that), start it beside your app, and answer for it at `/_leptoskit/services/notifications/` on your app's own address. [Services](../32-services.md) explains how services run and deploy.

The `trust` line matters because of what it defaults to: a `package` in your workspace is your own code and defaults to `first-party`, but a `crate` from the registry — like the snippet above — is someone else's, so it defaults to `isolated`, and an isolated service is not told who is signed in. The notifications service is first-party by intent: it reads your users' inboxes, so the first copy-paste setup says so explicitly. `forward_identity` then defaults to on for first-party services; if you prefer to name it, the whole table is:

```toml
[services.notifications]
crate = "leptoskit_notifications"
version = "0.3"
trust = "first-party"
forward_identity = true
```

Reading the inbox needs a signed-in person, which needs `forward_identity = true` on the service's table, your app's server serving `rpc::server::Server` with your sign-in as its `Identify` (which the scaffold already does), and, in a native app, `client::configure` with your credentials. Nothing is forwarded but the person's id: not their session, email, or roles. [Knowing who is signed in](../32-services.md#knowing-who-is-signed-in) has the details.

## Send one notification and read it

Three steps take you from nothing to a notification on a screen. Each has its own page for the details.

**1. Send from your server.** Add the crate to your server and call `send` with `call_as_app`, which signs the call with the service's token:

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
        ..SendSpec::default()
    },
    target: SendTarget::Recipients(vec![alice_id]),
    validate_only: false,
})
.await?;
```

[Sending](sending.md) explains the key, schedules, cancelling, and reports.

**2. Read the inbox in a screen.** The signed-in person's inbox is a live, paginated query. This component lists it, and works on every platform that draws leptoskit components:

```rust
use leptoskit::prelude::*;
use leptoskit_notifications::InboxShow;
use leptoskit_rpc::use_live_paginated;

#[component]
pub fn InboxScreen() -> impl IntoView {
    let inbox = use_live_paginated(
        move |page| leptoskit_notifications::Inbox { page, show: InboxShow::Active },
        20,
    );
    let list = move || {
        let rows = inbox
            .items()
            .into_iter()
            .map(|item| view! { <Text>{item.title}</Text> })
            .collect::<Fragment>();

        view! { <View>{rows}</View> }
    };

    view! { <ScrollView>{list}</ScrollView> }
}
```

The list updates by itself when a send reaches this person, on this device or after they read something on another.

**3. Mark it read.** One mutation, and the row is read on every device:

```rust
leptoskit_rpc::client::call(leptoskit_notifications::MarkRead { notification: id }).await?;
```

[The inbox](inbox.md) has the unread badge, dismissing, filters, and category opt-outs.

Nothing so far shows a banner on a device whose app is closed. That is push, and it is optional: [Push setup](push-setup.md) configures a transport and enrolls devices, and every send then reaches them too.

## The pages in this chapter

| Page                                                   | What it answers                                                                                 |
| ------------------------------------------------------ | ----------------------------------------------------------------------------------------------- |
| [Sending](sending.md)                                  | Sends, idempotency, schedules, cancel, repeating sends, progress, dry runs, reports, send list  |
| [The inbox](inbox.md)                                  | Reading the inbox, the unread badge, read and dismissed state, categories, local banners        |
| [Audiences](audiences.md)                              | Topics, conditions, attributes, segments, everyone, device targeting, local time, the catalogue |
| [Shaping messages](shaping-messages.md)                | Options per platform, silent data messages, badges, translated variants, payload size, previews |
| [Push setup](push-setup.md)                            | Transport credentials, and enrolling browsers, iOS, Android, and macOS devices                  |
| [Receiving on the device](receiving.md)                | Taps, arrivals while the app is open, data messages, and recording an open                      |
| [Delivery and troubleshooting](troubleshooting.md)     | What a send does, testing on a device, every reason code and error, and what to do about each   |
| [Campaigns](campaigns.md)                              | Drafts a person writes, tests, schedules, and launches                                          |
| [Privacy and data retention](privacy-and-retention.md) | What is stored, for how long, and how to erase a person                                         |
| [Coming from Firebase](coming-from-firebase.md)        | Concept mapping, a migration in steps, and what is not there yet                                |
| [Reference](reference.md)                              | Every operation and who may call it, limits, variables, features, deploying and upgrading       |

## What it doesn't do yet

- **Native client enrollment is iOS, Android, and macOS.** The service side of enrollment is done, the web client is this crate's `browser` module, and the native client — iOS's and macOS's APNs token and Android's FCM token alike — is the `native` module ([Push setup](push-setup.md)). Windows and Linux apps receive no push, so they have nothing to tap.
- **Opens are counted only when your app reports them, and nothing after the open is tracked.** A tap reaches your app's code; [`mark_opened`](receiving.md#record-the-open) reports it, and the [send's report](sending.md#see-what-a-send-came-to) counts it. There are no conversions or funnels, and no per-device or per-platform breakdown of opens.
- **Action buttons are web and Apple only, and Apple's aren't registered yet.** `web.actions` buttons work today. An Apple notification shows buttons only for a category the app registered with the system, which leptoskit doesn't do yet; the receiver already reports a pressed button's id when an app registers its own. Android sends carry no buttons.
- **No delivery receipts.** The service knows what it stored and what the provider accepted; it doesn't track who saw a banner. `accepted` is as far as status goes.
- **No per-device state.** Read, dismissed, and cancelled state is per person, shared across their devices by design.
- **Translations are yours to write.** [Variants](shaping-messages.md#say-it-in-each-persons-language) pick the right text for each person, but the service does not translate anything, and it has no localization keys that a device resolves from its own string tables.
- **Person attributes are the app's own statements.** Segments read what your server set — plan, country, spend. Attributes are set in batches by the app's server only; a person cannot describe themselves into an audience.
- **Device details are what the device reports.** The platform, locale, time zone, and app version are taken as each client sends them, checked for shape and never verified, and a device's time zone is not checked against a time zone database when it enrolls (a [local-time send](audiences.md#send-at-each-persons-local-time) skips one it does not know). There is no OS version or device model, `'apns'` doesn't tell an iPhone from a Mac, and a web app has a version only if you pass one. Windows and Linux devices can't enroll, so device conditions never reach them.

The [Coming from Firebase](coming-from-firebase.md#what-is-not-there-yet) page has a longer list, written for apps that use Firebase Cloud Messaging today.
