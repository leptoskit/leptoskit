# Notifications

You want to tell someone something after they have left the screen: an invoice was paid, a ride is arriving, someone replied. The usual route hands your users and your sending patterns to a third-party push platform, and binds delivery to OS credentials you have to enrol for. leptoskit has a notifications **service** you add with one table in `leptoskit.toml`: a small program with its own database that runs beside your app's server, stores one durable notification per recipient, and serves each person their own inbox. When you configure a push transport, the same send also wakes the devices its recipients enrolled — APNs, FCM, and Web Push, first-party, with your app's own credentials and no management vendor in between.

This page shows how to add it, send from your server, read the inbox on every platform, manage categories and schedules, and stand up push delivery. It also says plainly what it does not do yet: **the client half of enrollment** (platform token acquisition and the browser service-worker handlers), and any claim that a provider's acceptance means a person saw a banner.

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

The signed-in person enrolls through the service's RPC — `register_push_endpoint` with one typed credential. What the credential is and how your app obtains it is platform work: request the OS's push permission, get the token or subscription, and hand it over. (The generated token-acquisition plumbing and the browser service-worker handler that receives the decrypted push are still to come; today your app brings its own credential.)

```rust,ignore
leptoskit_rpc::call(leptoskit_notifications::RegisterPushEndpoint {
    credential: leptoskit_notifications::PushCredential::Apns {
        device_token: token_from_the_os,
    },
})
.await?;
```

The service answers an opaque endpoint id — the credential itself never comes back, and never appears in a list, a status row, or an error. Registering a destination that account already holds answers the same endpoint again, so a retried enrollment makes no duplicate; a Web Push subscription whose browser rotated its keys updates the stored credential in place, keeping the endpoint and its queued deliveries. A person holds at most ten live endpoints. A destination another **live** account holds is **refused, not reassigned**: signing out of the app on a device revokes that device's endpoints, and then the next account on it enrolls cleanly. A destination the provider has invalidated is nobody's — the device presenting it takes it over — and its previous owner's re-enrollment reactivates it. That is the account-switch story, and it is why revocation matters.

List and revoke are the person's own. The list is one bounded page at a time, oldest first, and it includes invalidated endpoints with their state — a device list that hides them can't explain them:

```rust,ignore
let page = leptoskit_rpc::call(leptoskit_notifications::PushEndpoints {
    page: Default::default(),
}).await?;
leptoskit_rpc::call(leptoskit_notifications::RevokePushEndpoint { endpoint: id }).await?;
```

Revoking deletes the endpoint and skips everything still queued for it as `endpoint_revoked`. Another person's endpoint — or an id nobody holds — is `404`, exactly as if it didn't exist.

### What a send does

When a recipient of a send has enrolled endpoints and your server has configured that platform's transport, the send enqueues one durable delivery per notification and endpoint in the send's own transaction. The push job attempts each one when the notification becomes visible — a scheduled send pushes at its `visible_at`, not before — and re-checks, at the moment of the attempt, everything that could have changed since the send: cancellation, expiry, the category opt-out the person may have flipped meanwhile, and the endpoint's liveness. A cancelled send never pushes; an expired one is skipped; an opt-out made after the send reaches the queue still stops the push.

Transient failures — a 429, a 5xx, a network error — retry with the provider's `Retry-After` honored when given, up to eight attempts (eight is eight: the ninth never goes out), and then end as rejected with `attempts_exhausted`. Permanent failures end at once with a reason code, and only the provider's own "unregistered" answer invalidates an endpoint — a plain 404 from FCM is a project-naming problem on your side, not a dead device — so nothing further is pushed to a dead credential and its other queued deliveries are skipped. A failure of _your_ credentials (a refused OAuth exchange, a token the provider rejects when fresh) is neither: the delivery waits and retries without spending its attempts, so a misconfigured account shows up as deliveries stuck `retrying` rather than quietly failed. Failure of one endpoint never rolls back the inbox row or the other endpoints' deliveries: the inbox row committed with the send, and push is best-effort on top of it.

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

Push reaches a device through its platform's service: iOS and macOS through APNs, Android through FCM, and the web through the browser's own push service (Web Push with VAPID). **Windows and Linux have no enrollment story in leptoskit yet**, and HarmonyOS and the watch surfaces are out of scope for push — their inbox is durable retrieval, as everywhere. Background delivery also needs the app's own OS setup on each platform (capabilities, `GoogleService-Info.plist` or `google-services.json`, a browser service worker) — the native token-acquisition plumbing that wires this up is still to come, so treat today's push as server-complete and client-plumbing-outstanding.

## What it doesn't do yet

- **No client enrollment plumbing.** The service side of enrollment is done; the generated native token acquisition and the browser service-worker handler that receives decrypted pushes are later work. Today an app brings its own platform-issued credential to `register_push_endpoint`, and a web client decrypts the push with its own worker.
- **No delivery receipts.** The service knows what it stored and what the provider accepted; it doesn't track who saw a banner. `accepted` is as far as status goes.
- **No per-device state.** Read, dismissed, and cancelled state is per person, shared across their devices by design.

The engineering notes in the repository describe the transport work that follows.

## Deploying

The service ships the way [services](32-services.md#deploying) do: `leptoskit build` builds its binary, and the deploy targets run it beside your app with its own database. Its `healthz` and `readyz` come from the service framework, and its migrations apply when it starts.
