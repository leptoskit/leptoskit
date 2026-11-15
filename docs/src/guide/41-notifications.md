# Notifications

You want to tell someone something after they have left the screen: an invoice was paid, a ride is arriving, someone replied. The usual route hands your users and your sending patterns to a third-party push platform, and binds delivery to OS credentials you have to enrol for. leptoskit has a notifications **service** you add with one table in `leptoskit.toml`: a small program with its own database that runs beside your app's server, stores one durable notification per recipient, and serves each person their own inbox.

This page shows how to add it, send from your server, read the inbox on every platform, and manage categories and schedules. It also says plainly what it does not do yet: **OS background push**. Delivery today is inbox retrieval while your app runs — a signed-in screen asks the service what is new, and a live query updates it the moment it arrives. APNs, FCM, and Web Push transports and device enrollment are later work.

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
    recipients: vec![alice_id, bob_id],
})
.await?;
```

Every named recipient gets one durable row of their own, visible immediately — or at `visible_at`, when you schedule one — until `expires_at`. The counts come back: `delivered`, `skipped_opted_out` (recipients who opted out of the category), and `recipients`. The whole send is one transaction, so it lands all at once or not at all.

The `idempotency_key` is how retries stay safe: derive one stable key per logical send (like the invoice id above) and reuse it on retry. The same key with the same intent answers the first run's result and writes nothing — across restarts, and past the send's expiry, because the send's bookkeeping lives in the service's database — and the same key with a different intent is refused with a conflict, so a reused key can't quietly send something else.

`SendSpec::visible_at` and `expires_at` take RFC 3339 instants, at most a year ahead. `data` is a JSON object of at most 4 KiB your screens read to route a tap; everything is length-checked on both sides, so a screen can reject bad input without a round trip.

### Cancel a scheduled send

A scheduled send can be cancelled until it becomes visible:

```rust
let cancelled: u32 = call_as_app(leptoskit_notifications::Cancel {
    idempotency_key: "launch-reminder".into(),
})
.await?;
```

After it's due the service refuses with a conflict — what someone has already seen can't be unsent. Cancelling again answers `0`, and so does cancelling a send every recipient opted out of: it delivered nothing, but the service knows the key, and cancelling it is still cancelling it.

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

This shows a banner only while your app runs, and only where the plugin is implemented: iOS and macOS through `UNUserNotificationCenter`, Android 26+ through `NotificationManager`, the browser through the `Notification` API, and Linux through D-Bus. **Windows returns `Unavailable`** — there is no Windows toast backend yet. On Android the channel must exist before a post (posts to a channel that doesn't are dropped), and Android 13+ also needs `POST_NOTIFICATIONS` — declared in your app's `android/app/src/main/AndroidManifest.xml` ([the `android/` folder is yours](../platforms/android.md)) and granted at runtime through `request_permission`, which uses the host's permission broker. On every platform a successful post doesn't prove the person saw anything: Focus modes, notification permissions, channel settings, and foreground presentation policy decide that. Scheduled **local** notifications, badges from the plugin, and click routing are not available everywhere either — see the [plugin inventory](../platforms/plugin-inventory.md) for what each platform does. None of this posting reaches a device where your app isn't running; that is the background-push gap below.

## What it doesn't do yet

- **No OS background push.** There is no APNs, FCM, or Web Push transport, and no device enrollment: nothing wakes a device where your app isn't running. A person sees what arrived while a signed-in screen of theirs was up — or the next time one opens, when the inbox is already there.
- **No per-device state.** Read, dismissed, and cancelled state is per person, shared across their devices by design.
- **No delivery receipts.** The service knows what it stored; it doesn't track who saw a banner.

The engineering notes in the repository describe the transport work that follows.

## Deploying

The service ships the way [services](32-services.md#deploying) do: `leptoskit build` builds its binary, and the deploy targets run it beside your app with its own database. Its `healthz` and `readyz` come from the service framework, and its migrations apply when it starts.
