# Receiving on the device

A push that only shows a banner leaves your app guessing. This page shows how your code learns which notification was tapped, what arrived while the app was open, and what a silent data message said, and how to tell the service that a person opened a notification so the send's report can count it. It applies to iOS, macOS, Android, and the web. Windows, Linux, and the terminal receive no push, so there is nothing to receive there.

## Know what was tapped and what arrived

Three questions come up as soon as a push can arrive. Which notification did the person tap, so the right screen opens? What arrived while the app was already open, so the screen in front of them is current? And what did a silent data message say? One receiver answers all three, on iOS, macOS, Android, and the web.

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

					// Count the open, and mark it read everywhere.
					let _ = opened.mark_opened().await;
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

	navigation.render()
}
```

In a browser the loop is the same with `browser::Receiver`, which re-exports the same types (`leptoskit_notifications::browser::Received`). An app that builds for both picks the module with `#[cfg(target_arch = "wasm32")]` the way it already does for `enroll`.

**A tap** is `Received::Opened`. It carries:

- `notification`: the id the inbox lists the item under, so you can open it or [mark it read](inbox.md#read-the-inbox-on-any-platform).
- `link`: the send's `options.link`, when it had one.
- `action`: the id of the action button that was pressed, or `None` for a tap on the notification itself.
- `data`: the send's `data`.
- `local`: the id of a local notification your app posted with the [plugin](inbox.md#composing-with-local-notifications), when that is what was tapped.
- `launched`: `true` for the tap that started the app.

`opened.route("/inbox")` answers where to go: the send's link when it is a path on your app, such as `/invoice/42`, and otherwise the route you name with the notification's id added as `lkn` — the same address the web receiver opens. A link that is a full `https` URL is left for you to read from `link`, because only your app knows whether that URL is its own.

### Record the open

A tap tells your app where to go, but it tells the service nothing: the person's other devices still show the notification as unread, and your [send's report](sending.md#see-what-a-send-came-to) has no idea anyone opened it. One call fixes both:

```rust
if let Received::Opened(opened) = received {
	let _ = opened.mark_opened().await;
}
```

`mark_opened` tells the service, as the signed-in person, that they opened this notification. It works the same on iOS, macOS, Android, and the web. The service then:

- records the open on the person's inbox row, once. The first open is the one kept; tapping again, on this device or another, changes nothing and is not an error.
- marks the notification read, if it wasn't already, so the unread badge drops on every device.
- counts it in the send's report as `opened`.

It answers `Ok(true)` when the service recorded the open and `Ok(false)` when there was nothing to record, which is the case for a local notification your app posted itself and for a push whose payload carried no id. A tap on an action button counts as an open too.

It can fail: nobody is signed in, the notification was deleted or belongs to someone else, or the network is down. Nothing retries it for you. An open that was not recorded is safe to ignore, as the example does, and safe to send again.

On the web, the page makes this call after the receiver hands it the tap. The service worker that shows the push never calls the service, because it holds no sign-in. So a click that opens your site is counted once your page has loaded and created its `Receiver`.

If you call the service yourself, the operation is `leptoskit_notifications::MarkOpened { notification }`, called like [`MarkRead`](inbox.md#read-the-inbox-on-any-platform).

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

**A silent data message** is `Received::Data`, described [above](shaping-messages.md#silent-data-messages).

### What each platform does

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

### Apps generated before receiving

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
