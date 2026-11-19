//! What a device hands your app about a notification after the service
//! has pushed it: the person tapped it, it arrived while the app was open,
//! or a silent data message came in. One set of types for every platform;
//! [`crate::native::Receiver`] delivers them in an iOS, macOS, or Android
//! app and [`crate::browser::Receiver`] in a web app.
//!
//! Everything here is plain reading of the envelope the service's own
//! transports write — `lkn` for the notification's id, `lklink` for the
//! send's link, `lk` (or, on Android, the data map itself) for the send's
//! `data`, `lkbadge` for a data message's badge — so the hosts stay thin:
//! they pass the provider's message through as text and decide nothing.
//! A payload that is missing or malformed never loses a tap: the tap is
//! still delivered, with whatever could be read.

#[cfg(any(feature = "native", test))]
use std::collections::VecDeque;

use serde_json::Map;
use serde_json::Value;
use uuid::Uuid;

/// One thing the device received.
#[derive(Debug, Clone, PartialEq)]
pub enum Received {
	/// The person tapped a notification or one of its action buttons.
	Opened(Opened),
	/// A notification arrived while the app was open. The system showed
	/// it; this is the moment to refresh what the screen shows.
	Shown(Shown),
	/// A silent data message arrived. Nothing was shown.
	Data(DataMessage),
}

/// A tap on a notification.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Opened {
	/// The notification's id, as the inbox lists it. `None` for a local
	/// notification, and for a push whose payload carried no readable id.
	pub notification: Option<Uuid>,
	/// The send's `options.link`, when it had one: a path on your app
	/// (`/invoice/42`) or an `https` URL. On the web it is always a path
	/// on your own origin. [`Opened::route`] turns it into where to go.
	pub link: Option<String>,
	/// The id of the action button that was pressed; `None` for a tap on
	/// the notification itself.
	pub action: Option<String>,
	/// The send's `data`. On Android every value is a string, because
	/// that is how FCM carries it; a nested value arrives as its JSON
	/// text.
	pub data: Map<String, Value>,
	/// The id of the local notification that was tapped, when it was one
	/// your app posted with the local notifications plugin.
	pub local: Option<String>,
	/// Whether the tap was already waiting when your app first asked:
	/// the tap that started the app, or brought a closed page back.
	pub launched: bool,
}

impl Opened {
	/// Where in your app the tap should go: the send's link when it is a
	/// path on your app, and otherwise `inbox_route` with the
	/// notification's id as `lkn`, the same address the web receiver
	/// opens. An `https` link is left to you (read [`Opened::link`]): the
	/// app decides whether a full URL is its own.
	#[must_use]
	pub fn route(&self, inbox_route: &str) -> String {
		if let Some(link) = self.link.as_deref()
			&& link.starts_with('/')
		{
			return link.to_owned();
		}

		match self.notification {
			Some(id) => {
				let separator = if inbox_route.contains('?') { '&' } else { '?' };
				format!("{inbox_route}{separator}lkn={id}")
			}
			None => inbox_route.to_owned(),
		}
	}

	/// Tells the service the person opened this notification, so the
	/// send's report counts it and the notification is marked read on
	/// every device. Call it where you handle the tap:
	///
	/// ```rust,ignore
	/// if let Received::Opened(opened) = received {
	///     navigate(&opened.route("/inbox"));
	///     let _ = opened.mark_opened().await;
	/// }
	/// ```
	///
	/// It is one call to [`mark_opened`](crate::mark_opened) as the
	/// signed-in person, the same on every platform. Answers `true` when
	/// the service recorded it — the first time or again — and `false`
	/// without calling anything when there is nothing to record: a local
	/// notification, or a push whose payload carried no readable id. A tap
	/// on an action button counts as an open too.
	///
	/// # Errors
	///
	/// What the call failed with: nobody is signed in, the notification is
	/// not this person's or is no longer stored, or the service could not
	/// be reached. Nothing is retried for you; an open that could not be
	/// recorded is safe to drop, and safe to send again.
	pub async fn mark_opened(&self) -> Result<bool, leptoskit_rpc::FnError> {
		let Some(notification) = self.notification else {
			return Ok(false);
		};

		leptoskit_rpc::client::call(crate::MarkOpened { notification }).await?;

		Ok(true)
	}
}

/// A notification that arrived while the app was open.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Shown {
	/// The notification's id, as the inbox lists it.
	pub notification: Option<Uuid>,
	/// The send's `data`, as [`Opened::data`] describes it.
	pub data: Map<String, Value>,
}

/// A silent data message.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DataMessage {
	/// The id of the send's row for this person. A data message is never
	/// in the inbox; the id tells two deliveries of one message apart.
	pub notification: Option<Uuid>,
	/// The send's `data`, as [`Opened::data`] describes it.
	pub data: Map<String, Value>,
	/// The badge count the send asked for, for the app to apply.
	pub badge: Option<u64>,
}

/// Why nothing can be received here.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReceiveError {
	/// This platform delivers nothing: a native call outside an iOS,
	/// macOS, or Android app, a host older than the receiving method, or
	/// a browser without service workers.
	#[error("this platform does not deliver received notifications")]
	Unavailable,
	/// The notifications plugin is not in `[native] plugins`.
	#[error("the notifications plugin is not installed in this app")]
	NoPlugin,
	/// The host or the browser refused; the text is its own short reason.
	#[error("receiving failed: {0}")]
	Host(String),
}

/// Which host callback an item came from, as the notifications plugin's
/// `ReceivedKind` names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
	/// A tap.
	Opened,
	/// A notification the host showed while the app was open.
	Presented,
	/// A message handed to the app without a tap.
	Message,
}

/// Reads one item a native host received. `payload` is the provider's
/// message as JSON text: the APNs payload on iOS and macOS — the send's
/// `data` under `lk` — or the FCM data map on Android, where the send's
/// `data` is the map's own keys. A tap is always delivered, whatever its
/// payload; a message that carries nothing readable is dropped (`None`).
#[must_use]
pub fn from_host(source: Source, action: &str, local: &str, payload: &str) -> Option<Received> {
	let object = serde_json::from_str::<Value>(payload)
		.ok()
		.and_then(|value| value.as_object().cloned());
	let readable = object.is_some();
	let object = object.unwrap_or_default();
	let notification = object
		.get("lkn")
		.and_then(Value::as_str)
		.and_then(|id| Uuid::parse_str(id).ok());
	let alert = object.get("aps").and_then(|aps| aps.get("alert")).is_some();

	match source {
		Source::Opened => {
			Some(Received::Opened(Opened {
				notification,
				link: object.get("lklink").and_then(Value::as_str).and_then(link),
				action: (!action.is_empty()).then(|| action.to_owned()),
				data: data(&object),
				local: (!local.is_empty()).then(|| local.to_owned()),
				launched: false,
			}))
		}
		Source::Message if !alert => {
			readable.then(|| {
				Received::Data(DataMessage {
					notification,
					badge: badge(object.get("lkbadge")),
					data: data(&object),
				})
			})
		}
		Source::Presented | Source::Message => {
			readable.then(|| {
				Received::Shown(Shown {
					notification,
					data: data(&object),
				})
			})
		}
	}
}

/// A link a tap may carry: a path on the app, or an `https` URL — the
/// same two shapes the service accepts as `options.link`. Anything else
/// is not a link.
fn link(value: &str) -> Option<String> {
	if value.len() > crate::options::MAX_OPTION_URL_LEN {
		return None;
	}

	let allowed = if value.starts_with('/') {
		own_path(value)
	} else {
		url::Url::parse(value).is_ok_and(|url| url.scheme() == "https" && url.host().is_some())
	};

	allowed.then(|| value.to_owned())
}

/// Whether `value` is a path on the app: one leading slash, no backslash,
/// no control characters.
fn own_path(value: &str) -> bool {
	value.starts_with('/')
		&& !value.starts_with("//")
		&& !value.contains('\\')
		&& !value.chars().any(char::is_control)
}

/// The send's `data` out of a provider message. APNs carries it as the
/// `lk` object beside `aps`; FCM's data map is the data itself, with the
/// envelope's own keys and the platform's reserved ones beside it.
fn data(object: &Map<String, Value>) -> Map<String, Value> {
	if object.contains_key("aps") || object.get("lk").is_some_and(Value::is_object) {
		return object
			.get("lk")
			.and_then(Value::as_object)
			.cloned()
			.unwrap_or_default();
	}

	object
		.iter()
		.filter(|(name, _)| !reserved(name))
		.map(|(name, value)| (name.clone(), value.clone()))
		.collect()
}

/// A key in an Android message that is the envelope's or the platform's,
/// not the send's.
fn reserved(name: &str) -> bool {
	matches!(
		name,
		"lkn" | "lklink" | "lkimg" | "lkbadge" | "from" | "message_type" | "collapse_key"
	) || ["google.", "gcm.", "leptoskit."]
		.iter()
		.any(|prefix| name.starts_with(prefix))
}

/// A badge count: a number on Apple platforms, its text on Android.
fn badge(value: Option<&Value>) -> Option<u64> {
	match value? {
		Value::Number(number) => number.as_u64(),
		Value::String(text) => text.parse().ok(),
		_ => None,
	}
}

/// Reads the address a web app was opened at. When the receiver worker
/// opened it for a click it carries the notification's id as `lkn`, and
/// the pressed button's id as `lka`. Answers the tap — with the address
/// itself, cleaned, as its link — and that cleaned address, for the page
/// to put back so a reload doesn't report the tap again. `None` when the
/// address names no notification.
#[must_use]
pub fn from_address(href: &str) -> Option<(Opened, String)> {
	let mut url = url::Url::parse(href).ok()?;
	let mut notification = None;
	let mut action = None;
	let mut kept = Vec::new();

	for (name, value) in url.query_pairs() {
		match name.as_ref() {
			"lkn" => notification = Uuid::parse_str(&value).ok(),
			"lka" => action = Some(value.into_owned()).filter(|action| !action.is_empty()),
			_ => kept.push((name.into_owned(), value.into_owned())),
		}
	}

	let notification = notification?;

	if kept.is_empty() {
		url.set_query(None);
	} else {
		url.query_pairs_mut().clear().extend_pairs(kept);
	}

	let mut clean = url.path().to_owned();

	if let Some(query) = url.query() {
		clean.push('?');
		clean.push_str(query);
	}

	if let Some(fragment) = url.fragment() {
		clean.push('#');
		clean.push_str(fragment);
	}

	Some((
		Opened {
			notification: Some(notification),
			link: Some(clean.clone()),
			action,
			launched: true,
			..Opened::default()
		},
		clean,
	))
}

/// Reads one message the receiver worker posted to the page: the event,
/// and the token the page answers so the worker knows the app took the
/// tap and doesn't reload the page. `None` for a message that isn't the
/// receiver's.
#[must_use]
pub fn from_worker(message: &Value) -> Option<(Received, Option<String>)> {
	let event = message.get("lkn_event")?.as_object()?;
	let notification = event
		.get("id")
		.and_then(Value::as_str)
		.and_then(|id| Uuid::parse_str(id).ok());
	let data = event
		.get("data")
		.and_then(Value::as_object)
		.cloned()
		.unwrap_or_default();
	let text = |name: &str| {
		event
			.get(name)
			.and_then(Value::as_str)
			.filter(|value| !value.is_empty())
			.map(str::to_owned)
	};

	let received = match event.get("kind")?.as_str()? {
		"opened" => {
			Received::Opened(Opened {
				notification,
				// The worker only ever forwards a path on its own origin;
				// anything else here is not its message's link.
				link: text("link").filter(|link| own_path(link)),
				action: text("action"),
				data,
				local: None,
				launched: false,
			})
		}
		"shown" => Received::Shown(Shown { notification, data }),
		"data" => {
			Received::Data(DataMessage {
				notification,
				data,
				badge: badge(event.get("badge")),
			})
		}
		_ => return None,
	};

	Some((received, text("token")))
}

/// The order events reach the app in, and which taps were the launch's:
/// everything the first batch holds was waiting before the app asked.
#[cfg(any(feature = "native", test))]
#[cfg_attr(
	not(target_arch = "wasm32"),
	allow(dead_code, reason = "only a guest asks a host")
)]
#[derive(Debug, Default)]
pub(crate) struct Intake {
	asked: bool,
	queue: VecDeque<Received>,
}

#[cfg(any(feature = "native", test))]
#[cfg_attr(
	not(target_arch = "wasm32"),
	allow(dead_code, reason = "only a guest asks a host")
)]
impl Intake {
	/// Whether nothing has been asked of the host yet. The first ask
	/// never waits, so what it answers is exactly what was already there.
	pub(crate) fn first(&self) -> bool {
		!self.asked
	}

	/// Queues one batch from the host, oldest first.
	pub(crate) fn accept(&mut self, batch: impl IntoIterator<Item = Received>) {
		let launched = !self.asked;
		self.asked = true;

		for mut received in batch {
			if let Received::Opened(opened) = &mut received {
				opened.launched = launched;
			}

			self.queue.push_back(received);
		}
	}

	/// The next event, oldest first.
	pub(crate) fn pop(&mut self) -> Option<Received> {
		self.queue.pop_front()
	}

	/// Everything queued, oldest first.
	pub(crate) fn drain(&mut self) -> Vec<Received> {
		self.queue.drain(..).collect()
	}
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;

	const ID: &str = "0190a1b2-0000-7000-8000-000000000001";

	fn id() -> Uuid {
		Uuid::parse_str(ID).unwrap()
	}

	fn object(value: Value) -> Map<String, Value> {
		match value {
			Value::Object(map) => map,
			other => panic!("not an object: {other}"),
		}
	}

	#[test]
	fn an_apple_tap_carries_the_id_the_link_the_action_and_the_data() {
		let payload = json!({
			"aps": { "alert": { "title": "Paid", "body": "Invoice 42" } },
			"lkn": ID,
			"lklink": "/invoice/42",
			"lk": { "invoice": 42, "tags": ["a"] },
		})
		.to_string();

		assert_eq!(
			from_host(Source::Opened, "approve", "", &payload),
			Some(Received::Opened(Opened {
				notification: Some(id()),
				link: Some("/invoice/42".into()),
				action: Some("approve".into()),
				data: object(json!({ "invoice": 42, "tags": ["a"] })),
				local: None,
				launched: false,
			}))
		);
	}

	#[test]
	fn an_android_tap_reads_the_data_map_and_drops_the_reserved_keys() {
		let payload = json!({
			"lkn": ID,
			"lklink": "https://example.test/invoice/42",
			"invoice": "42",
			"tags": "[\"a\"]",
			"google.message_id": "0:1",
			"google.sent_time": "1",
			"gcm.n.e": "1",
			"from": "123",
			"collapse_key": "x",
			"leptoskit.notifications.open": "1",
		})
		.to_string();

		assert_eq!(
			from_host(Source::Opened, "", "", &payload),
			Some(Received::Opened(Opened {
				notification: Some(id()),
				link: Some("https://example.test/invoice/42".into()),
				action: None,
				data: object(json!({ "invoice": "42", "tags": "[\"a\"]" })),
				local: None,
				launched: false,
			}))
		);
	}

	#[test]
	fn a_tap_is_delivered_whatever_its_payload() {
		for payload in ["", "not json", "[1]", "{\"lkn\":7}"] {
			assert_eq!(
				from_host(Source::Opened, "", "", payload),
				Some(Received::Opened(Opened::default())),
				"{payload:?}"
			);
		}

		// A local notification's tap names the plugin's own id.
		assert_eq!(
			from_host(Source::Opened, "", "job-1", ""),
			Some(Received::Opened(Opened {
				local: Some("job-1".into()),
				..Opened::default()
			}))
		);
	}

	#[test]
	fn a_link_is_a_path_on_the_app_or_an_https_url_and_nothing_else() {
		let linked = |link: &str| {
			match from_host(
				Source::Opened,
				"",
				"",
				&json!({ "lklink": link }).to_string(),
			) {
				Some(Received::Opened(opened)) => opened.link,
				other => panic!("{other:?}"),
			}
		};

		assert_eq!(linked("/a?b=1#c").as_deref(), Some("/a?b=1#c"));
		assert_eq!(
			linked("https://example.test/a").as_deref(),
			Some("https://example.test/a")
		);

		for refused in [
			"//evil.test/a",
			"/a\\b",
			"/a\nb",
			"http://example.test/a",
			"javascript:alert(1)",
			"invoice/42",
			"",
		] {
			assert_eq!(linked(refused), None, "{refused:?}");
		}

		assert_eq!(linked(&format!("/{}", "a".repeat(2048))), None);
	}

	#[test]
	fn a_route_is_the_apps_own_path_or_the_inbox_with_the_id() {
		let mut opened = Opened {
			notification: Some(id()),
			link: Some("/invoice/42".into()),
			..Opened::default()
		};
		assert_eq!(opened.route("/inbox"), "/invoice/42");

		// A full URL is the app's to judge: the route is the inbox.
		opened.link = Some("https://example.test/invoice/42".into());
		assert_eq!(opened.route("/inbox"), format!("/inbox?lkn={ID}"));
		assert_eq!(
			opened.route("/inbox?tab=all"),
			format!("/inbox?tab=all&lkn={ID}")
		);

		opened.link = None;
		opened.notification = None;
		assert_eq!(opened.route("/inbox"), "/inbox");
	}

	#[test]
	fn a_data_message_reaches_the_app_with_its_data_and_badge() {
		let apple = json!({
			"aps": { "content-available": 1 },
			"lkn": ID,
			"lk": { "sync": "orders" },
			"lkbadge": 3,
		})
		.to_string();
		let android = json!({
			"lkn": ID,
			"sync": "orders",
			"lkbadge": "3",
			"google.message_id": "0:1",
		})
		.to_string();
		let expected = Some(Received::Data(DataMessage {
			notification: Some(id()),
			data: object(json!({ "sync": "orders" })),
			badge: Some(3),
		}));

		assert_eq!(from_host(Source::Message, "", "", &apple), expected);
		assert_eq!(from_host(Source::Message, "", "", &android), expected);

		// Nothing readable is nothing to hand over.
		assert_eq!(from_host(Source::Message, "", "", ""), None);
		assert_eq!(from_host(Source::Message, "", "", "not json"), None);
	}

	#[test]
	fn a_notification_that_arrived_while_open_is_shown_not_data() {
		let apple = json!({
			"aps": { "alert": { "title": "Paid", "body": "" } },
			"lkn": ID,
			"lk": { "invoice": 42 },
		})
		.to_string();
		let expected = Some(Received::Shown(Shown {
			notification: Some(id()),
			data: object(json!({ "invoice": 42 })),
		}));

		assert_eq!(from_host(Source::Presented, "", "", &apple), expected);
		// A visible push that also woke the app is still a notification.
		assert_eq!(from_host(Source::Message, "", "", &apple), expected);

		let android = json!({ "lkn": ID, "invoice": "42" }).to_string();
		assert_eq!(
			from_host(Source::Presented, "", "", &android),
			Some(Received::Shown(Shown {
				notification: Some(id()),
				data: object(json!({ "invoice": "42" })),
			}))
		);
	}

	#[test]
	fn the_first_batch_is_what_launched_the_app() {
		let tap = || Received::Opened(Opened::default());
		let mut intake = Intake::default();

		assert!(intake.first());
		intake.accept([tap(), Received::Shown(Shown::default())]);
		assert!(!intake.first());
		intake.accept([tap()]);

		assert_eq!(
			intake.pop(),
			Some(Received::Opened(Opened {
				launched: true,
				..Opened::default()
			}))
		);
		assert_eq!(intake.pop(), Some(Received::Shown(Shown::default())));
		assert_eq!(intake.drain(), vec![tap()]);
		assert_eq!(intake.pop(), None);

		// An empty first answer still counts as having asked.
		let mut quiet = Intake::default();
		quiet.accept([]);
		quiet.accept([tap()]);
		assert_eq!(quiet.pop(), Some(tap()));
	}

	#[test]
	fn a_launch_address_names_the_tap_and_is_cleaned() {
		let (opened, clean) = from_address(&format!(
			"https://app.test/invoice/42?tab=a&lkn={ID}&lka=approve#top"
		))
		.unwrap();

		assert_eq!(clean, "/invoice/42?tab=a#top");
		assert_eq!(
			opened,
			Opened {
				notification: Some(id()),
				link: Some("/invoice/42?tab=a#top".into()),
				action: Some("approve".into()),
				launched: true,
				..Opened::default()
			}
		);
		assert_eq!(opened.route("/inbox"), "/invoice/42?tab=a#top");

		let (opened, clean) = from_address(&format!("https://app.test/inbox?lkn={ID}")).unwrap();
		assert_eq!(clean, "/inbox");
		assert_eq!(opened.action, None);

		for plain in [
			"https://app.test/inbox",
			"https://app.test/inbox?lkn=nope",
			"https://app.test/inbox?lka=approve",
			"not a url",
		] {
			assert_eq!(from_address(plain), None, "{plain}");
		}
	}

	/// Recording an open is one typed call naming the notification and
	/// nothing else — the person is the forwarded identity, never an
	/// argument — and a tap with no id to name makes no call at all. The
	/// call itself runs in a compiled guest and in a browser, in
	/// `crates/leptoskit_wasm/tests/notifications_guest.rs` and
	/// `tests/browser.rs`.
	#[test]
	#[cfg(not(target_arch = "wasm32"))]
	fn an_open_is_one_call_naming_the_notification_or_none_at_all() {
		assert_eq!(
			serde_json::to_value(crate::MarkOpened { notification: id() }).unwrap(),
			json!({ "notification": ID })
		);

		// No id: a local notification, or a payload nobody could read.
		// Nothing here can reach a service, so an answer at all — and this
		// one — says no call was attempted.
		let local = Opened {
			local: Some("reminder-1".into()),
			..Opened::default()
		};
		assert_eq!(
			futures::executor::block_on(local.mark_opened()).ok(),
			Some(false)
		);
	}

	#[test]
	fn a_worker_message_becomes_its_event_and_its_token() {
		let opened = json!({ "lkn_event": {
			"kind": "opened",
			"id": ID,
			"link": "/invoice/42",
			"action": "approve",
			"data": { "invoice": 42 },
			"token": "t-1",
		} });

		assert_eq!(
			from_worker(&opened),
			Some((
				Received::Opened(Opened {
					notification: Some(id()),
					link: Some("/invoice/42".into()),
					action: Some("approve".into()),
					data: object(json!({ "invoice": 42 })),
					local: None,
					launched: false,
				}),
				Some("t-1".into())
			))
		);

		// A link off the page's own paths is not the worker's.
		let foreign = json!({ "lkn_event": { "kind": "opened", "link": "https://evil.test/" } });
		assert_eq!(
			from_worker(&foreign),
			Some((Received::Opened(Opened::default()), None))
		);

		assert_eq!(
			from_worker(&json!({ "lkn_event": { "kind": "shown", "id": ID } })),
			Some((
				Received::Shown(Shown {
					notification: Some(id()),
					data: Map::new(),
				}),
				None
			))
		);
		assert_eq!(
			from_worker(
				&json!({ "lkn_event": { "kind": "data", "data": { "a": 1 }, "badge": 2 } })
			),
			Some((
				Received::Data(DataMessage {
					notification: None,
					data: object(json!({ "a": 1 })),
					badge: Some(2),
				}),
				None
			))
		);

		for other in [
			json!({ "type": "cache" }),
			json!({ "lkn_event": { "kind": "other" } }),
			json!({ "lkn_event": "opened" }),
			json!("lkn_event"),
		] {
			assert_eq!(from_worker(&other), None, "{other}");
		}
	}
}
