# The inbox

A push is gone once it is swiped away, and a person with three devices sees it three times. The inbox is the part that lasts: each person has one list of what they were sent, and reading an item on one device reads it on all of them. This page shows how a screen reads that list, shows an unread badge, marks items read or dismissed, and lets a person opt out of a category. It works on the web, iOS, Android, macOS, Windows, Linux, the terminal, and (experimentally) HarmonyOS, because it is ordinary service calls.

## Read the inbox on any platform

The signed-in person reads their own inbox, newest first, one bounded page at a time, from the web, iOS, Android, macOS, Windows, Linux, the terminal, or (experimentally) HarmonyOS — wherever your screens run. Each page is a [live query](../24-real-time.md), and what reaches an open screen is everything that changes its answer: a send that reaches this person, a read or dismissal from their other device, a category they opt out of, a scheduled send becoming due, and an expiring one leaving. The service runs the wake jobs that make time visible: when a send's schedule turns, the screens it reaches re-run with a fresh clock, so a scheduled item appears when its time comes and an expired one goes when its time is up — on every platform the live connection reaches.

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

Only visible rows are in the inbox: scheduled ones wait for their time, cancelled ones never arrive, expired ones have gone, and categories the person opted out of are hidden whatever was sent before. Each `InboxItem` carries its `id`, `category`, `title`, `body`, `data` (the send's JSON object, as text), `created_at`, and `read_at` and `dismissed_at`, which are empty until the person reads or dismisses it. A page holds at most 100 items.

`show` picks which rows a page lists:

| `InboxShow` | Lists                                                     |
| ----------- | --------------------------------------------------------- |
| `Active`    | what is visible and not dismissed: the usual inbox        |
| `Unread`    | what is visible, not dismissed, and not yet read          |
| `All`       | everything visible, dismissed or not                      |
| `Dismissed` | only the dismissed rows, for a screen that offers an undo |

`unread_count` returns what a badge shows, live:

```rust
let unread = use_live(|| leptoskit_notifications::UnreadCount {});

let badge = move || unread.data().unwrap_or(0);
```

Marking read or dismissed is a mutation on one item, and it lands on every device at once, because the state is the row's, not the device's:

```rust
leptoskit_rpc::client::call(leptoskit_notifications::MarkRead { notification: id }).await?;
leptoskit_rpc::client::call(leptoskit_notifications::MarkDismissed { notification: id }).await?;
```

Dismissing an item takes it out of the `Active` list and marks it read. Neither call can be undone through the service: there is no "mark unread". When the person arrived by tapping a push, call [`mark_opened`](receiving.md#record-the-open) instead of `MarkRead`: it marks the row read and counts the open in the send's report.

Reading someone else's notification by id — guessing, replaying, whatever — answers "not found", exactly as if it didn't exist. The service resolves the owner from the forwarded identity, never from a request argument, so there is nothing to forge.

## Categories and opt-outs

A category is a short label a send carries, such as `billing` or `social`. The person opts in or out of each one:

```rust
leptoskit_rpc::client::call(leptoskit_notifications::SetCategoryPreference {
    category: "marketing".into(),
    opted_out: true,
})
.await?;
```

An opt-out works twice. At **send** time the service skips recipients who opted out of the category and counts them in `skipped_opted_out` — no row is created. At **visibility** time the inbox hides rows whose category its reader has since opted out of, so an opt-out reaches what was already delivered; opting back in shows it again. A person holds at most 100 preferences.

A settings screen needs to show the switches as they stand. `category_preferences` answers the person's own list, one entry per category they have set, each with `category` and `opted_out`. A category the person never touched is not in the list and counts as opted in:

```rust
let preferences = use_live(|| leptoskit_notifications::CategoryPreferences {});

let opted_out = move |category: &str| {
    preferences
        .data()
        .unwrap_or_default()
        .iter()
        .any(|preference| preference.category == category && preference.opted_out)
};
```

The service keeps no list of your categories for the person to choose from: your app names the ones it offers. A category is 1 to 64 bytes.

## Composing with local notifications

A received inbox item is a durable record, not an OS notification. Where you want a banner while the app runs, compose it with the [local notifications plugin](../../platforms/plugin-inventory.md): ask for permission once, configure the channel Android posts through, and post each new item the live page reports.

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
	"Inbox",
	"New items in your inbox.",
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

This shows a banner only while your app runs, and only where the plugin is implemented: iOS and macOS through `UNUserNotificationCenter`, Android 26+ through `NotificationManager`, the browser through the `Notification` API, and Linux through D-Bus. **Windows returns `Unavailable`** — there is no Windows toast backend yet. On Android the channel must exist before a post (posts to a channel that doesn't are dropped), and Android 13+ also needs `POST_NOTIFICATIONS` — declared in your app's `android/app/src/main/AndroidManifest.xml` ([the `android/` folder is yours](../../platforms/android.md)) and granted at runtime through `request_permission`, which uses the host's permission broker. On every platform a successful post doesn't prove the person saw anything: Focus modes, notification permissions, channel settings, and foreground presentation policy decide that. **iOS and macOS are the sharp case**: the system shows no banner for an app that is in the foreground unless the app's notification-centre delegate asks for one. The generated hosts install that delegate, so a post made while your app is open shows its banner; an app whose `hosts/ios/App.swift` or `hosts/macos/App.swift` was generated before this needs [three lines added](receiving.md#apps-generated-before-receiving), and a menu-bar app has no delegate at all. A tap on a local notification reaches your app on iOS, macOS, and Android as `Received::Opened` with the notification's own id in `local` ([below](receiving.md#know-what-was-tapped-and-what-arrived)). Scheduled **local** notifications and badges from the plugin are not available everywhere — see the [plugin inventory](../../platforms/plugin-inventory.md) for what each platform does. None of this posting reaches a device where your app isn't running; that is what server push, below, is for.
