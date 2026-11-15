//! Browser-side behavior of the notifications push client: what real
//! Chrome answers about availability and a page with no worker, and what
//! the receiver script — the exact bytes an app serves — does with the
//! envelope the service sends and with a click.
//!
//! The script runs in this page the way a build runs it: its text is
//! injected as a script element, which defines `window.lkn` and no
//! listeners. Its handlers are then driven with plain objects standing in
//! for push and click events, and `window.registration` and
//! `window.clients` stand in for the worker global's own. That is
//! **simulated service-worker machinery around real script execution**:
//! the parsing, the fallback, the tagging, the URL rules, and the
//! waitUntil lifetimes are the shipped script's real behavior, while the
//! browser's own `push` dispatch, notification rendering, and coalescing
//! stay unexercised here — those need a registered worker and a real
//! push service, and the enrolled-push half of this suite stops at the
//! browser's own refusals.
//!
//! Runs under `wasm-bindgen-test-runner` with a webdriver, as
//! `test:browser` does.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use leptoskit_notifications::browser as lkn;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_test::wasm_bindgen_test;

wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

/// The route the suite's scripts are built with.
const ROUTE: &str = "/inbox";

/// The scopes the lifecycle suite's real workers register under. The
/// runner serves files from the package root, so these are the browser's
/// own registrations on its own origin — real install, activation, and
/// scope matching — not mocks.
const FIXTURE_SCRIPT: &str = "/tests/fixtures/sw.js";
const FIXTURE_SCOPE: &str = "/tests/fixtures/";
const SLOW_SCRIPT: &str = "/tests/fixtures/slow/sw.js";
const SLOW_SCOPE: &str = "/tests/fixtures/slow/";
const BAD_SCRIPT: &str = "/tests/fixtures/bad/sw.js";
const BAD_SCOPE: &str = "/tests/fixtures/bad/";
/// The receiver template itself, served from source: registering it runs
/// the exact bytes an app ships.
const RECEIVER_SCRIPT_URL: &str = "/src/browser/receiver.js";
const RECEIVER_SCOPE: &str = "/src/browser/";

/// Where the mocked network answers the service the enrollment calls
/// really go to.
const SERVICE_PREFIX: &str = "/_leptoskit/services/notifications/rpc";

fn window() -> web_sys::Window {
	web_sys::window().expect("the test runs in a window")
}

/// Injects the built script as a page script, the way a build would
/// evaluate it, so `window.lkn` is the shipped code and nothing attaches
/// listeners.
fn install_script() {
	let script = lkn::receiver_script(ROUTE).expect("the route builds");
	let element = window()
		.document()
		.expect("a document")
		.create_element("script")
		.expect("a script element");
	element.set_text_content(Some(&script));
	window()
		.document()
		.expect("a document")
		.body()
		.expect("a body")
		.append_child(&element)
		.expect("script appended");
}

/// Puts one same-origin window client and one foreign one in the click
/// handler's way, each recording what it is asked to do.
fn install_clients() {
	eval(
		r#"
		(() => {
			const foreign = {
				url: "https://evil.test/app",
				focus: async () => { window.__lknFocused.push("foreign"); },
				navigate: async (url) => { window.__lknNavigated.push(url); },
			};
			const mine = {
				url: window.location.origin + "/app",
				focus: async () => { window.__lknFocused.push("mine"); },
				navigate: async (url) => { window.__lknNavigated.push(url); },
			};
			window.__lknClients.push(foreign, mine);
		})();
	"#,
	);
}

/// The page's `lkn` namespace, as the injected script defined it.
fn lkn_namespace() -> js_sys::Object {
	js_sys::Reflect::get(&window().into(), &JsValue::from_str("lkn"))
		.expect("the script defined lkn")
		.dyn_into()
		.expect("lkn is an object")
}

/// Installs the stand-ins for a worker global: a registration that
/// records what it is told to show, a clients list the click reads, and
/// the counters the assertions read.
fn install_fakes() {
	eval(
		r#"
		(() => {
			const w = window;
			w.__lknShown = [];
			w.__lknOpened = [];
			w.__lknFocused = [];
			w.__lknNavigated = [];
			w.__lknClosed = 0;
			w.__lknPending = [];
			w.__lknClients = [];
			w.__lknRegistration = {
				// One microtask of real asynchrony, so the banner lands
				// inside the lifetime rather than during the call.
				showNotification: async (title, options) => {
					await Promise.resolve();
					w.__lknShown.push({ title, options });
				},
			};
			w.registration = w.__lknRegistration;
			w.clients = {
				matchAll: async () => w.__lknClients,
				openWindow: async (url) => {
					w.__lknOpened.push(url);
					return null;
				},
			};
			w.__lknPushEvent = (text, byteLength) => {
				const pending = [];
				const event = {
					data: byteLength == null ? null : {
						arrayBuffer: () => new ArrayBuffer(byteLength),
						text: () => text,
					},
					waitUntil: (promise) => {
						pending.push(promise);
						w.__lknPending.push(promise);
					},
				};
				return event;
			};
			w.__lknDispatchPush = (text, byteLength) => {
				// The registered listener, driven the way the browser
				// drives it: a dispatchable event carrying the payload
				// and the waitUntil the wiring registers with.
				const event = w.__lknPushEvent(text, byteLength);
				const dispatchable = new Event("push");
				dispatchable.data = event.data;
				dispatchable.waitUntil = event.waitUntil;
				w.dispatchEvent(dispatchable);
			};
			w.__lknClickEvent = (data) => {
				const pending = [];
				const event = {
					notification: {
						// The browser hands the click a structured clone
						// of the options' data: an object, not text.
						data: data == null ? undefined : JSON.parse(data),
						close: () => {
							w.__lknClosed += 1;
						},
					},
					waitUntil: (promise) => {
						pending.push(promise);
						w.__lknPending.push(promise);
					},
				};
				return event;
			};
		})();
	"#,
	);
}

/// Restores the page to its no-worker state between tests.
fn remove_fakes() {
	eval(
		r#"
		(() => {
			const w = window;
			for (const name of [
				"__lknShown", "__lknOpened", "__lknFocused", "__lknNavigated",
				"__lknClosed", "__lknPending", "__lknClients",
				"__lknRegistration", "__lknPushEvent", "__lknClickEvent",
			]) {
				delete w[name];
			}
			delete w.registration;
			delete w.clients;
		})();
	"#,
	);
}

fn eval(script: &str) {
	js_sys::eval(script).expect("the page helpers evaluate");
}

fn counter(name: &str) -> js_sys::Array {
	js_sys::Reflect::get(&window().into(), &JsValue::from_str(name))
		.expect("the counter exists")
		.dyn_into()
		.expect("the counter is an array")
}

/// The number counters the click fakes keep: banners closed, so far.
fn count(name: &str) -> u32 {
	js_sys::Reflect::get(&window().into(), &JsValue::from_str(name))
		.expect("the count exists")
		.as_f64()
		.expect("a number") as u32
}

async fn wait_for_handlers() {
	let pending = counter("__lknPending");
	let all = js_sys::Promise::all(&pending.into());
	as_promise(all.into()).await.expect("the handlers settled");
}

fn as_promise(value: JsValue) -> wasm_bindgen_futures::JsFuture {
	wasm_bindgen_futures::JsFuture::from(value.dyn_into::<js_sys::Promise>().expect("a promise"))
}

fn shown() -> Vec<js_sys::Object> {
	counter("__lknShown")
		.iter()
		.map(|value| value.dyn_into().expect("a shown record"))
		.collect()
}

/// The last shown record's `title` and `options` as readable values.
fn shown_title(record: &js_sys::Object) -> String {
	js_sys::Reflect::get(record, &JsValue::from_str("title"))
		.expect("a title")
		.as_string()
		.expect("a title string")
}

fn shown_options(record: &js_sys::Object) -> js_sys::Object {
	js_sys::Reflect::get(record, &JsValue::from_str("options"))
		.expect("options")
		.dyn_into()
		.expect("an options object")
}

fn option(options: &js_sys::Object, name: &str) -> JsValue {
	js_sys::Reflect::get(options, &JsValue::from_str(name)).unwrap_or(JsValue::UNDEFINED)
}

fn push_event(text: &str, byte_length: Option<u32>) -> js_sys::Object {
	let make = js_sys::Reflect::get(&window().into(), &JsValue::from_str("__lknPushEvent"))
		.expect("the push factory")
		.dyn_into::<js_sys::Function>()
		.expect("a function");
	let call = make
		.call2(
			&JsValue::UNDEFINED,
			&JsValue::from_str(text),
			&byte_length.map_or(JsValue::UNDEFINED, |length| {
				JsValue::from_f64(f64::from(length))
			}),
		)
		.expect("an event");
	call.dyn_into().expect("an event object")
}

fn click_event(data: &str) -> js_sys::Object {
	let make = js_sys::Reflect::get(&window().into(), &JsValue::from_str("__lknClickEvent"))
		.expect("the click factory")
		.dyn_into::<js_sys::Function>()
		.expect("a function");
	let call = make
		.call1(&JsValue::UNDEFINED, &JsValue::from_str(data))
		.expect("an event");
	call.dyn_into().expect("an event object")
}

async fn deliver(text: &str, byte_length: Option<u32>) {
	let deliver = js_sys::Reflect::get(&lkn_namespace(), &JsValue::from_str("deliver"))
		.expect("deliver")
		.dyn_into::<js_sys::Function>()
		.expect("a function");
	let promise = deliver
		.call1(
			&lkn_namespace().into(),
			&push_event(text, byte_length).into(),
		)
		.expect("a promise");
	as_promise(promise).await.expect("the push shown");
}

async fn click(data: &str) {
	let open = js_sys::Reflect::get(&lkn_namespace(), &JsValue::from_str("open"))
		.expect("open")
		.dyn_into::<js_sys::Function>()
		.expect("a function");
	let promise = open
		.call1(&lkn_namespace().into(), &click_event(data).into())
		.expect("a promise");
	as_promise(promise).await.expect("the click settled");
}

fn opened() -> Vec<String> {
	counter("__lknOpened")
		.iter()
		.map(|value| value.as_string().expect("a url"))
		.collect()
}

fn focused_markers() -> Vec<String> {
	counter("__lknFocused")
		.iter()
		.map(|value| value.as_string().expect("a marker"))
		.collect()
}

fn navigated() -> Vec<String> {
	counter("__lknNavigated")
		.iter()
		.map(|value| value.as_string().expect("a url"))
		.collect()
}

fn origin() -> String {
	window().location().origin().expect("an origin")
}

/// A UUID v7 the service would have minted, for envelope tests.
fn notification_id() -> String {
	"0198c0de-7ead-7ec0-9dbe-c0ffee000001".to_owned()
}

#[wasm_bindgen_test]
async fn a_secure_chrome_reports_ready() {
	let available = lkn::availability();
	assert_eq!(available, lkn::Availability::Ready);
}

#[wasm_bindgen_test]
async fn enrolling_without_a_worker_answers_no_worker_and_prompts_nothing() {
	// This harness page has no registration: the whole enrollment stops
	// before a permission prompt or a subscription could exist.
	let error = lkn::enroll(&lkn::EnrollOptions::new("BNc6fWb6vUjEow7md8yuWQ")).await;
	assert_eq!(error, Err(lkn::EnrollError::NoWorker));
}

#[wasm_bindgen_test]
async fn a_full_envelope_shows_its_text_tag_and_data() {
	install_fakes();
	install_script();

	let id = notification_id();
	let envelope = format!(
		r#"{{"title":"Invoice paid","body":"See the inbox","lkn":"{id}","lk":{{"order":"7"}}}}"#
	);
	deliver(&envelope, Some(envelope.len() as u32)).await;

	let records = shown();
	assert_eq!(records.len(), 1, "one push, one banner");
	assert_eq!(shown_title(&records[0]), "Invoice paid");
	let options = shown_options(&records[0]);
	assert_eq!(
		option(&options, "body").as_string().expect("a body"),
		"See the inbox"
	);
	assert_eq!(option(&options, "tag").as_string().expect("a tag"), id);
	let data = option(&options, "data")
		.dyn_into::<js_sys::Object>()
		.expect("data");
	assert_eq!(
		js_sys::Reflect::get(&data, &JsValue::from_str("lkn"))
			.expect("lkn")
			.as_string()
			.expect("an id"),
		id
	);
	assert_eq!(
		js_sys::Reflect::get(
			&js_sys::Reflect::get(&data, &JsValue::from_str("lk"))
				.expect("lk")
				.dyn_into::<js_sys::Object>()
				.expect("an object"),
			&JsValue::from_str("order")
		)
		.expect("order")
		.as_string()
		.expect("a string"),
		"7"
	);

	// The same notification pushed again carries the same tag, so
	// wherever the platform coalesces, the repeat replaces.
	deliver(&envelope, Some(envelope.len() as u32)).await;
	assert_eq!(shown().len(), 2);
	assert_eq!(
		option(&shown_options(&shown()[1]), "tag")
			.as_string()
			.expect("a tag"),
		id,
		"the tag is stable across repeats"
	);

	remove_fakes();
}

#[wasm_bindgen_test]
async fn the_id_only_envelope_shows_without_data() {
	install_fakes();
	install_script();

	// The sender's minimal envelope: title and body, the id, and no data.
	let id = notification_id();
	let envelope = format!(r#"{{"title":"t","body":"b","lkn":"{id}"}}"#);
	deliver(&envelope, Some(envelope.len() as u32)).await;

	let records = shown();
	assert_eq!(records.len(), 1);
	let options = shown_options(&records[0]);
	assert_eq!(option(&options, "tag").as_string().expect("a tag"), id);
	let data = option(&options, "data")
		.dyn_into::<js_sys::Object>()
		.expect("the id rides on the banner's data for the click");
	assert_eq!(
		js_sys::Reflect::get(&data, &JsValue::from_str("lkn"))
			.expect("lkn")
			.as_string()
			.expect("an id"),
		id
	);
	assert!(
		js_sys::Reflect::get(&data, &JsValue::from_str("lk"))
			.unwrap_or(JsValue::UNDEFINED)
			.is_undefined(),
		"an envelope without data leaves none on the banner"
	);

	remove_fakes();
}

#[wasm_bindgen_test]
async fn a_missing_title_still_shows_a_truthful_banner() {
	install_fakes();
	install_script();

	// The id alone: nothing trustworthy to display, one generic banner.
	let id = notification_id();
	let envelope = format!(r#"{{"lkn":"{id}"}}"#);
	deliver(&envelope, Some(envelope.len() as u32)).await;

	let records = shown();
	assert_eq!(records.len(), 1, "the delivery is never swallowed");
	assert_eq!(shown_title(&records[0]), "New notification");
	let options = shown_options(&records[0]);
	assert_eq!(option(&options, "body").as_string().expect("a body"), "");
	assert_eq!(option(&options, "tag").as_string().expect("a tag"), id);

	// An empty title is the same refusal to invent content.
	let empty = format!(r#"{{"title":"","body":"x","lkn":"{id}"}}"#);
	deliver(&empty, Some(empty.len() as u32)).await;
	assert_eq!(shown_title(&shown()[1]), "New notification");
	assert_eq!(
		option(&shown_options(&shown()[1]), "body")
			.as_string()
			.expect("a body"),
		"",
		"an unattributable body is not shown under a generic title"
	);

	remove_fakes();
}

#[wasm_bindgen_test]
async fn broken_envelopes_still_show_one_banner_under_the_fallback_tag() {
	install_fakes();
	install_script();

	for (text, length) in [
		("not json at all", Some(15_u32)),
		("[1, 2, 3]", Some(9)),
		(r#"{"title":42,"body":{},"lkn":7}"#, Some(30)),
		(r#"{"title":"late","body":"b","lkn":"id"}"#, Some(5_000)),
		("", Some(0)),
	] {
		deliver(text, length).await;
	}

	let records = shown();
	assert_eq!(records.len(), 5, "every push answers one banner");

	for record in &records {
		assert_eq!(shown_title(record), "New notification");
		assert_eq!(
			option(&shown_options(record), "tag")
				.as_string()
				.expect("a tag"),
			"leptoskit",
			"broken pushes collapse under the fallback tag"
		);
	}

	// The oversized one is not parsed at all, so its honest-looking text
	// never shows.
	assert_eq!(
		option(&shown_options(&records[3]), "body")
			.as_string()
			.expect("a body"),
		""
	);

	remove_fakes();
}

#[wasm_bindgen_test]
async fn long_text_is_clipped_not_dropped() {
	install_fakes();
	install_script();

	let long_title = "x".repeat(300);
	let long_body = "y".repeat(2_500);
	let envelope = format!(
		r#"{{"title":"{long_title}","body":"{long_body}","lkn":"{}"}}"#,
		notification_id()
	);
	deliver(&envelope, Some(envelope.len() as u32)).await;

	let record = &shown()[0];
	let title = shown_title(record);
	assert_eq!(title.chars().count(), 200, "the title clips to its bound");
	assert!(title.ends_with('…'), "the clip is visible");
	let body = option(&shown_options(record), "body")
		.as_string()
		.expect("a body");
	assert_eq!(body.chars().count(), 2_000, "the body clips to its bound");

	remove_fakes();
}

#[wasm_bindgen_test]
async fn a_click_focuses_the_same_origin_window_on_the_inbox_route() {
	install_fakes();
	install_script();
	install_clients();

	let id = notification_id();
	click(&format!(r#"{{"lkn":"{id}"}}"#)).await;
	wait_for_handlers().await;

	assert_eq!(count("__lknClosed"), 1, "the banner closes");
	let focused = focused_markers();
	assert_eq!(focused, vec!["mine"], "the same-origin window focuses");
	assert!(opened().is_empty(), "no second window opens");
	assert_eq!(
		navigated(),
		vec![format!("{}/inbox?lkn={}", origin(), id)],
		"the focused window lands on the inbox route with the id"
	);

	remove_fakes();
}

#[wasm_bindgen_test]
async fn a_click_without_a_window_opens_the_route_with_the_id() {
	install_fakes();
	install_script();

	let id = notification_id();
	click(&format!(r#"{{"lkn":"{id}"}}"#)).await;
	wait_for_handlers().await;

	assert_eq!(count("__lknClosed"), 1);
	assert_eq!(opened(), vec![format!("{}/inbox?lkn={}", origin(), id)]);

	remove_fakes();
}

#[wasm_bindgen_test]
async fn payload_content_cannot_choose_where_a_click_goes() {
	install_fakes();
	install_script();
	install_clients();

	// The envelope's data carries URLs and routes of its own; the id is
	// not a UUID; a `link` is smuggled in. None of it may reach the
	// address line.
	click(
		r#"{"lkn":"not-a-uuid","lk":{"url":"https://evil.test/x","route":"/elsewhere","link":"javascript:alert(1)"}}"#,
	)
	.await;
	wait_for_handlers().await;

	assert_eq!(focused_markers(), vec!["mine"]);
	assert_eq!(
		navigated(),
		vec![format!("{}/inbox", origin())],
		"no id on the address line, and no foreign route"
	);
	assert_eq!(opened(), Vec::<String>::new());

	remove_fakes();
}

#[wasm_bindgen_test]
async fn foreign_windows_are_never_focused() {
	install_fakes();
	install_script();
	install_clients();

	click(r#"{"lkn":"0198c0de-7ead-7ec0-9dbe-c0ffee000001"}"#).await;
	wait_for_handlers().await;

	assert_eq!(
		focused_markers(),
		vec!["mine"],
		"only this origin's window is used, foreign first or not"
	);
	assert_eq!(navigated().len(), 1, "one navigation, on this origin");

	remove_fakes();
}

#[wasm_bindgen_test]
async fn a_mis_built_route_is_refused_at_click_time() {
	// The builder refuses these routes; this simulates a hand-edited
	// build that baked one in anyway, so the script's own origin check is
	// what stands.
	for baked in [
		"//evil.test/inbox",
		"javascript:alert(1)",
		"https://evil.test/x",
	] {
		install_fakes();
		let script = lkn::RECEIVER_SCRIPT.replace(
			"\"__LKN_INBOX_ROUTE__\"",
			&serde_json::to_string(baked).expect("a literal"),
		);
		let element = window()
			.document()
			.expect("a document")
			.create_element("script")
			.expect("a script element");
		element.set_text_content(Some(&script));
		window()
			.document()
			.expect("a document")
			.body()
			.expect("a body")
			.append_child(&element)
			.expect("appended");

		click(r#"{"lkn":"0198c0de-7ead-7ec0-9dbe-c0ffee000001"}"#).await;
		wait_for_handlers().await;

		assert_eq!(count("__lknClosed"), 1, "{baked}: the banner closes");
		assert!(opened().is_empty(), "{baked}: nothing opens");
		assert!(navigated().is_empty(), "{baked}: nothing navigates");

		remove_fakes();
	}
}

#[wasm_bindgen_test]
async fn the_wait_until_promise_covers_the_whole_show() {
	install_fakes();
	install_script();

	let id = notification_id();
	let envelope = format!(r#"{{"title":"t","body":"b","lkn":"{id}"}}"#);

	// Dispatch through the registered listener, as the browser would, so
	// the wiring's `event.waitUntil(deliver(event))` is what registers
	// the lifetime.
	let dispatch = js_sys::Reflect::get(&window().into(), &JsValue::from_str("__lknDispatchPush"))
		.expect("the dispatch helper")
		.dyn_into::<js_sys::Function>()
		.expect("a function");
	dispatch
		.call2(
			&JsValue::UNDEFINED,
			&JsValue::from_str(&envelope),
			&JsValue::from_f64(f64::from(envelope.len() as u32)),
		)
		.expect("dispatched");

	let event_promise = counter("__lknPending")
		.pop()
		.dyn_into::<js_sys::Promise>()
		.expect("the registered lifetime");

	assert!(
		shown().is_empty(),
		"nothing is shown before the lifetime resolves"
	);
	as_promise(event_promise.into())
		.await
		.expect("the lifetime resolves");
	assert_eq!(shown().len(), 1, "the banner is inside the lifetime");

	remove_fakes();
}

// ---------------------------------------------------------------------------
// The enrollment lifecycle: real workers, mocked boundaries.
//
// Two boundaries stand between this suite and a live push setup, and
// both are replaced here, clearly marked:
//
// - **The provider boundary.** `ServiceWorkerRegistration.prototype
//   .pushManager` answers a fake keyed by registration scope: behind the real
//   one stand a permission prompt and a push service this suite has no provider
//   for. The fake answers the same calls (`getSubscription`, `subscribe`,
//   per-subscription `unsubscribe`) with scripted results and records what was
//   asked.
// - **The network boundary.** `window.fetch` answers a recorder that scripts
//   the RPC routes the enrollment calls really go to — the app's
//   `rpc.service_token`, and the notifications service's register, list, and
//   revoke operations — with the JSON the real ones give. The calls themselves
//   are the browser client's real ones: same-origin fetch, the service's route
//   prefix, the typed credential, the forwarded identity's bearer token.
//
// Everything between those boundaries is the browser's real machinery:
// the workers are registered, installed, activated, and matched by scope
// by the browser itself, over the runner's own localhost origin.
// ---------------------------------------------------------------------------

/// Installs both boundaries and their helpers, from the browser's own
/// zero: any registration a previous test left behind — a panic can skip
/// its cleanup — is unregistered first, and the client's service
/// identity is forgotten, so the app is asked for the person's token
/// again exactly as a fresh launch would be.
async fn install_boundaries() {
	let _ = js("navigator.serviceWorker
			.getRegistrations()
			.then((all) => Promise.all(all.map((registration) => registration.unregister())))")
	.await;
	leptoskit_rpc::client::reauthenticate();
	install_fake_push_manager();
	install_fake_fetch();
}

/// The provider boundary: a fake `pushManager` keyed by registration
/// scope.
fn install_fake_push_manager() {
	eval(
		r#"
		(() => {
			const w = window;

			// The suite's application server key: 65 bytes, exposed as the
			// base64url the enroll options take.
			const keyBytes = new Uint8Array(65);
			keyBytes[0] = 4;
			for (let i = 1; i < 65; i += 1) {
				keyBytes[i] = (i * 31 + 7) & 255;
			}
			const otherKey = new Uint8Array(65);
			otherKey[0] = 4;
			for (let i = 1; i < 65; i += 1) {
				otherKey[i] = (i * 13 + 5) & 255;
			}
			w.__lknVapidKey = btoa(String.fromCharCode(...keyBytes))
				.replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");

			// The fake PushManager's state, keyed by the registration's
			// absolute scope, so every read of `pushManager` on one
			// registration sees the same subscription.
			w.__lknPushStates = new Map();
			w.__lknUnsubscribed = [];
			let serial = 0;
			const state = (scope) => {
				if (!w.__lknPushStates.has(scope)) {
					w.__lknPushStates.set(scope, { subscription: null, subscribes: 0 });
				}
				return w.__lknPushStates.get(scope);
			};
			const bytes = (array) => new Uint8Array(array).buffer;
			const subscription = (endpoint, appKeyBytes) => {
				const p256dh = new Uint8Array(65);
				p256dh[0] = 2;
				const auth = new Uint8Array(16).fill(7);
				return {
					endpoint,
					expirationTime: null,
					options: {
						userVisibleOnly: true,
						applicationServerKey: bytes(appKeyBytes),
					},
					getKey: (name) =>
						name === "p256dh" ? p256dh.slice().buffer
						: name === "auth" ? auth.slice().buffer
						: null,
					unsubscribe: async () => {
						w.__lknUnsubscribed.push(endpoint);
						return true;
					},
				};
			};
			const decode = (keyB64Url) => {
				const binary = atob(keyB64Url.replace(/-/g, "+").replace(/_/g, "/"));
				const key = new Uint8Array(binary.length);
				for (let i = 0; i < binary.length; i += 1) {
					key[i] = binary.charCodeAt(i);
				}
				return key;
			};
			w.__lknSeed = (scope, endpoint, keyB64Url) => {
				state(scope).subscription = subscription(endpoint, decode(keyB64Url));
			};
			w.__lknSeedOtherKey = (scope, endpoint) => {
				state(scope).subscription = subscription(endpoint, otherKey);
			};
			w.__lknSubscriptionAt = (scope) =>
				state(scope).subscription && state(scope).subscription.endpoint;
			w.__lknSubscribesAt = (scope) => state(scope).subscribes;

			if (!w.__lknRestorePushManager) {
				const descriptor = Object.getOwnPropertyDescriptor(
					ServiceWorkerRegistration.prototype,
					"pushManager",
				);
				Object.defineProperty(ServiceWorkerRegistration.prototype, "pushManager", {
					configurable: true,
					get() {
						const held = state(this.scope);
						return {
							getSubscription: async () => held.subscription,
							subscribe: async (options) => {
								held.subscribes += 1;
								if (w.__lknSubscribeRefusal) {
									throw w.__lknSubscribeRefusal;
								}
								serial += 1;
								held.subscription = subscription(
									`https://push.test/push/${serial}`,
									new Uint8Array(options.applicationServerKey),
								);
								return held.subscription;
							},
						};
					},
				});
				w.__lknRestorePushManager = () => {
					Object.defineProperty(
						ServiceWorkerRegistration.prototype,
						"pushManager",
						descriptor,
					);
				};
			}
		})();
	"#,
	);
}

/// The network boundary: a recording `fetch` that answers the RPC routes
/// the enrollment calls really go to.
fn install_fake_fetch() {
	eval(
		r#"
		(() => {
			const w = window;
			const real = w.fetch.bind(w);
			const json = (status, value) => new Response(JSON.stringify(value), { status });
			const summary = {
				id: "6ec0f4b1-0000-7000-8000-0000000000f1",
				platform: "web_push",
				created_at: "2026-11-15T00:00:00Z",
				invalidated_at: null,
			};
			w.__lknRequests = [];
			w.__lknRouteAnswers = {};
			const defaults = {
				"/rpc/rpc.service_token": () =>
					json(200, { token: "lkn-test-token", expires_in: 300 }),
				"/_leptoskit/services/notifications/rpc/register_push_endpoint": () =>
					json(200, summary),
				"/_leptoskit/services/notifications/rpc/push_endpoints": () =>
					json(200, { items: [], end: null, has_more: false }),
				"/_leptoskit/services/notifications/rpc/revoke_push_endpoint": () =>
					new Response("null", { status: 200 }),
			};
			w.fetch = async (input, init) => {
				const url = typeof input === "string" ? input : input.url;
				const headers = init && init.headers;
				w.__lknRequests.push({
					url,
					method: init && init.method,
					authorization: headers && headers.get
						? headers.get("authorization")
						: undefined,
					idempotency_key: headers && headers.get
						? headers.get("idempotency-key")
						: undefined,
					body: init && init.body,
				});
				const answer = w.__lknRouteAnswers[url] || defaults[url];
				return answer ? answer(init && init.body) : real(input, init);
			};
			w.__lknRestoreFetch = () => {
				w.fetch = real;
			};
		})();
	"#,
	);
}

/// Restores the page between lifecycle tests: both boundaries off, the
/// recorded state gone, and every registration the suite made
/// unregistered, so the next test starts from the browser's own zero.
async fn remove_boundaries() {
	eval("window.__lknRestorePushManager(); window.__lknRestoreFetch();");
	let _ = js("navigator.serviceWorker
			.getRegistrations()
			.then((all) => Promise.all(all.map((registration) => registration.unregister())))")
	.await;
	eval(
		r#"
		for (const name of [
			"__lknVapidKey", "__lknPushStates", "__lknUnsubscribed",
			"__lknSeed", "__lknSeedOtherKey", "__lknSubscriptionAt",
			"__lknSubscribesAt", "__lknRestorePushManager", "__lknRequests",
			"__lknRouteAnswers", "__lknRestoreFetch", "__lknSubscribeRefusal",
		]) {
			delete window[name];
		}
	"#,
	);
}

/// The suite's application server key, as the enroll options take it.
fn vapid_key() -> String {
	eval_string("window.__lknVapidKey")
}

/// One script's absolute scope on this origin.
fn at(path: &str) -> String {
	format!("{}{}", origin(), path)
}

/// A synchronous page expression's string.
fn eval_string(script: &str) -> String {
	js_sys::eval(script)
		.expect("the expression evaluates")
		.as_string()
		.expect("a string")
}

/// A page expression that settles: the script must evaluate to a promise.
async fn js(script: &str) -> JsValue {
	let value = js_sys::eval(script).expect("the script evaluates");
	as_promise(value).await.expect("the script settles")
}

/// Registers a real worker from the served fixtures.
async fn register_fixture(script_url: &str) -> lkn::Registration {
	lkn::register(script_url, lkn::RegisterOptions::default())
		.await
		.expect("the fixture registers")
}

/// Seeds a fake subscription holding `endpoint` under this app's key at
/// `scope` (the registration's absolute scope URL).
fn seed(scope: &str, endpoint: &str) {
	eval(&format!(
		"window.__lknSeed({}, {}, window.__lknVapidKey);",
		serde_json::to_string(scope).expect("a scope literal"),
		serde_json::to_string(endpoint).expect("an endpoint literal"),
	));
}

/// The recorded requests, parsed.
fn requests() -> serde_json::Value {
	serde_json::from_str(&eval_string("JSON.stringify(window.__lknRequests)"))
		.expect("the recorder holds JSON")
}

/// The recorded requests to one route.
fn requests_to(route: &str) -> Vec<serde_json::Value> {
	let suffix = format!("{SERVICE_PREFIX}/{route}");
	requests()
		.as_array()
		.expect("an array")
		.iter()
		.filter(|request| request["url"].as_str() == Some(suffix.as_str()))
		.cloned()
		.collect()
}

/// Scripts a revoke acknowledgement or explicit failure.
fn answer_revoke(status: u16, body: &str) {
	eval(&format!(
		r#"window.__lknRouteAnswers["{SERVICE_PREFIX}/revoke_push_endpoint"] = async () =>
		new Response({body:?}, {{ status: {status} }});"#,
	));
}

/// Enrollment over a real registration, a real activation wait, and the
/// real browser transport: the typed credential reaches the service's
/// own route same-origin, signed with the identity the app's server
/// minted for the signed-in person.
#[wasm_bindgen_test]
async fn enrollment_rides_a_real_worker_and_reaches_the_service_over_rpc() {
	install_boundaries().await;
	let registration = register_fixture(FIXTURE_SCRIPT).await;

	let mut options = lkn::EnrollOptions::new(vapid_key());
	options.worker_scope = Some(FIXTURE_SCOPE.to_owned());
	let enrollment = lkn::enroll(&options).await.expect("the browser enrolls");

	assert_eq!(
		enrollment.endpoint.to_string(),
		"6ec0f4b1-0000-7000-8000-0000000000f1"
	);

	// The app minted the person's token first, then the service's own
	// route took the typed credential, signed with it.
	let token: Vec<_> = requests()
		.as_array()
		.expect("an array")
		.iter()
		.filter(|request| request["url"].as_str() == Some("/rpc/rpc.service_token"))
		.cloned()
		.collect();
	assert_eq!(token.len(), 1, "the app was asked for one service token");
	assert_eq!(token[0]["method"], "POST");

	let register = requests_to("register_push_endpoint");
	assert_eq!(register.len(), 1);
	assert_eq!(register[0]["method"], "POST");
	assert_eq!(register[0]["authorization"], "Bearer lkn-test-token");
	assert!(
		register[0]["idempotency_key"]
			.as_str()
			.is_some_and(|key| !key.is_empty())
	);
	let body: serde_json::Value =
		serde_json::from_str(register[0]["body"].as_str().expect("a body")).expect("JSON");
	assert_eq!(
		body["credential"]["web_push"]["endpoint"],
		"https://push.test/push/1"
	);
	let keys = body["credential"]["web_push"]["keys"]
		.as_object()
		.expect("keys");
	assert!(keys.contains_key("p256dh") && keys.contains_key("auth"));

	// The browser was asked to subscribe exactly once, with this app's
	// key, and the registration's worker is the active one waited for.
	assert_eq!(
		eval_string(&format!(
			"String(window.__lknSubscribesAt({}))",
			serde_json::to_string(&at(FIXTURE_SCOPE)).expect("a scope literal")
		)),
		"1"
	);
	assert!(registration.active().expect("readable").is_some());

	remove_boundaries().await;
}

/// A second enroll replays the subscription the browser already holds:
/// no new prompt-shaped call, the same endpoint id, the person's own
/// idempotent re-registration.
#[wasm_bindgen_test]
async fn a_second_enroll_replays_the_same_subscription_and_endpoint() {
	install_boundaries().await;
	let _registration = register_fixture(FIXTURE_SCRIPT).await;
	seed(&at(FIXTURE_SCOPE), "https://push.test/push/held");

	let mut options = lkn::EnrollOptions::new(vapid_key());
	options.worker_scope = Some(FIXTURE_SCOPE.to_owned());
	let first = lkn::enroll(&options).await.expect("the first enrolls");
	let second = lkn::enroll(&options).await.expect("the replay enrolls");

	assert_eq!(first, second, "the same browser, the same endpoint");
	assert_eq!(
		eval_string(&format!(
			"String(window.__lknSubscribesAt({}))",
			serde_json::to_string(&at(FIXTURE_SCOPE)).expect("a scope literal")
		)),
		"0",
		"the held subscription was reused, never re-subscribed"
	);
	assert_eq!(requests_to("register_push_endpoint").len(), 2);

	remove_boundaries().await;
}

/// A denied origin is mapped through the `Notification` constructor — a
/// function, not an object — and enrollment stops before anything is
/// asked of the worker or the network.
#[wasm_bindgen_test]
async fn a_denied_origin_answers_denied_and_touches_nothing() {
	install_boundaries().await;
	eval(
		r#"
		(() => {
			const fake = function Notification() {};
			fake.permission = "denied";
			window.__lknRealNotification = Object.getOwnPropertyDescriptor(
				window,
				"Notification",
			);
			Object.defineProperty(window, "Notification", {
				value: fake,
				configurable: true,
			});
		})();
	"#,
	);

	assert_eq!(lkn::availability(), lkn::Availability::Denied);

	let mut options = lkn::EnrollOptions::new(vapid_key());
	options.worker_scope = Some(FIXTURE_SCOPE.to_owned());
	assert_eq!(
		lkn::enroll(&options).await,
		Err(lkn::EnrollError::Denied),
		"the standing refusal answers before any worker or route is touched"
	);
	assert_eq!(requests().as_array().expect("an array").len(), 0);

	eval(
		r#"
		Object.defineProperty(
			window,
			"Notification",
			window.__lknRealNotification,
		);
		delete window.__lknRealNotification;
	"#,
	);
	remove_boundaries().await;
}

/// A refused subscription surfaces the browser's own error, and nothing
/// is enrolled or sent.
#[wasm_bindgen_test]
async fn a_refused_subscription_surfaces_the_browser_error() {
	install_boundaries().await;
	let _registration = register_fixture(FIXTURE_SCRIPT).await;
	eval(r#"window.__lknSubscribeRefusal = new Error("NotAllowedError: the gesture ran out");"#);

	let mut options = lkn::EnrollOptions::new(vapid_key());
	options.worker_scope = Some(FIXTURE_SCOPE.to_owned());
	let error = lkn::enroll(&options)
		.await
		.expect_err("the browser refuses");
	assert!(
		matches!(&error, lkn::EnrollError::Subscribe(message) if message.contains("gesture")),
		"the browser's refusal is the error: {error:?}"
	);
	assert_eq!(requests_to("register_push_endpoint").len(), 0);

	remove_boundaries().await;
}

/// A subscription created for another sender's key is refused, not
/// enrolled: claiming it would end in delivery that can never arrive,
/// and replacing it would silently destroy whatever sender holds it.
#[wasm_bindgen_test]
async fn a_subscription_for_another_sender_is_refused() {
	install_boundaries().await;
	let _registration = register_fixture(FIXTURE_SCRIPT).await;
	eval(&format!(
		"window.__lknSeedOtherKey({}, \"https://push.test/push/other-sender\");",
		serde_json::to_string(&at(FIXTURE_SCOPE)).expect("a scope literal"),
	));

	let mut options = lkn::EnrollOptions::new(vapid_key());
	options.worker_scope = Some(FIXTURE_SCOPE.to_owned());
	assert_eq!(
		lkn::enroll(&options).await,
		Err(lkn::EnrollError::KeyMismatch)
	);
	assert_eq!(requests_to("register_push_endpoint").len(), 0);
	assert_eq!(
		eval_string(&format!(
			"window.__lknSubscriptionAt({})",
			serde_json::to_string(&at(FIXTURE_SCOPE)).expect("a scope literal")
		)),
		"https://push.test/push/other-sender",
		"the other sender's subscription is left exactly where it was"
	);

	remove_boundaries().await;
}

/// `getRegistration` answers any scope it covers, so a broader worker
/// answers for a scope it doesn't cover; enrollment refuses the
/// stand-in, and a scope nothing covers answers `NoWorker`.
#[wasm_bindgen_test]
async fn a_broader_worker_never_stands_in_for_a_missing_scope() {
	install_boundaries().await;
	let _registration = register_fixture(FIXTURE_SCRIPT).await;

	let mut options = lkn::EnrollOptions::new(vapid_key());
	options.worker_scope = Some("/tests/fixtures/deeper/".to_owned());
	assert_eq!(
		lkn::enroll(&options).await,
		Err(lkn::EnrollError::Scope(
			"/tests/fixtures/deeper/".to_owned()
		)),
		"the fixture at /tests/fixtures/ must not be enrolled as /tests/fixtures/deeper/"
	);

	options.worker_scope = Some("/nowhere/".to_owned());
	assert_eq!(
		lkn::enroll(&options).await,
		Err(lkn::EnrollError::NoWorker),
		"a scope no registration covers answers no worker"
	);
	assert_eq!(requests_to("register_push_endpoint").len(), 0);

	remove_boundaries().await;
}

/// Enrollment waits for the selected worker's own activation rather than
/// racing its install.
#[wasm_bindgen_test]
async fn enrollment_waits_for_the_selected_workers_activation() {
	install_boundaries().await;
	let registration = register_fixture(SLOW_SCRIPT).await;

	let mut options = lkn::EnrollOptions::new(vapid_key());
	options.worker_scope = Some(SLOW_SCOPE.to_owned());
	let started = js_sys::Date::now();
	let _enrollment = lkn::enroll(&options)
		.await
		.expect("the slow worker enrolls");
	let waited = js_sys::Date::now() - started;

	assert!(
		waited >= 500.0,
		"the 700 ms install was waited out, not raced: {waited} ms"
	);
	assert!(
		registration.active().expect("readable").is_some(),
		"the worker is active by the time enrollment answers"
	);

	remove_boundaries().await;
}

/// A worker whose install fails errors the enrollment in bounded time
/// instead of hanging the gesture. The browser clears a registration
/// whose only worker went redundant without activating — the service
/// workers spec's answer to a failed install — so the enrollment sees
/// no registration at that scope, and says so.
#[wasm_bindgen_test]
async fn a_worker_that_never_activates_errors_instead_of_hanging() {
	install_boundaries().await;
	let _registration = register_fixture(BAD_SCRIPT).await;

	let mut options = lkn::EnrollOptions::new(vapid_key());
	options.worker_scope = Some(BAD_SCOPE.to_owned());
	let started = js_sys::Date::now();
	let error = lkn::enroll(&options).await.expect_err("the failed install");
	let took = js_sys::Date::now() - started;

	assert_eq!(error, lkn::EnrollError::NoWorker);
	assert!(took < 10_000.0, "the failure is bounded: {took} ms");
	assert_eq!(requests_to("register_push_endpoint").len(), 0);

	remove_boundaries().await;
}

/// Reconcile registers what the browser already holds and never
/// subscribes; with nothing held it answers `None` and touches no route.
#[wasm_bindgen_test]
async fn reconcile_registers_what_is_held_and_never_prompts() {
	install_boundaries().await;
	let _held = register_fixture(FIXTURE_SCRIPT).await;
	let _fresh = register_fixture(RECEIVER_SCRIPT_URL).await;
	seed(&at(FIXTURE_SCOPE), "https://push.test/push/held");

	let mut held = lkn::EnrollOptions::new(vapid_key());
	held.worker_scope = Some(FIXTURE_SCOPE.to_owned());
	let _enrollment = lkn::reconcile(&held)
		.await
		.expect("reconcile runs")
		.expect("held");
	assert_eq!(
		eval_string(&format!(
			"String(window.__lknSubscribesAt({}))",
			serde_json::to_string(&at(FIXTURE_SCOPE)).expect("a scope literal")
		)),
		"0",
		"reading a subscription prompts nothing"
	);
	assert_eq!(requests_to("register_push_endpoint").len(), 1);

	// The receiver's own scope holds no subscription: `None` is the app's
	// cue to offer the gesture that `enroll` can then act on.
	let mut fresh = lkn::EnrollOptions::new(vapid_key());
	fresh.worker_scope = Some(RECEIVER_SCOPE.to_owned());
	assert_eq!(
		lkn::reconcile(&fresh).await.expect("reconcile runs"),
		None,
		"no subscription held: the app's cue to offer the gesture"
	);
	assert_eq!(
		requests_to("register_push_endpoint").len(),
		1,
		"nothing was registered for an absent subscription"
	);

	remove_boundaries().await;
}

/// Disable revokes the enrollment and preserves both browser subscriptions.
#[wasm_bindgen_test]
async fn disable_revokes_without_unsubscribing() {
	install_boundaries().await;
	let _enrolled = register_fixture(FIXTURE_SCRIPT).await;
	let _other = register_fixture(RECEIVER_SCRIPT_URL).await;
	seed(&at(FIXTURE_SCOPE), "https://push.test/push/mine");
	seed(&at(RECEIVER_SCOPE), "https://push.test/push/theirs");

	let mut options = lkn::EnrollOptions::new(vapid_key());
	options.worker_scope = Some(FIXTURE_SCOPE.to_owned());
	let enrollment = lkn::enroll(&options).await.expect("the browser enrolls");

	lkn::disable(&enrollment)
		.await
		.expect("the disable completes");

	let revoke = requests_to("revoke_push_endpoint");
	assert_eq!(revoke.len(), 1);
	let body: serde_json::Value =
		serde_json::from_str(revoke[0]["body"].as_str().expect("a body")).expect("JSON");
	assert_eq!(body["endpoint"], enrollment.endpoint.to_string());
	assert_eq!(
		eval_string("JSON.stringify(window.__lknUnsubscribed)"),
		"[]",
		"disable never unsubscribes"
	);
	assert_eq!(
		eval_string(&format!(
			"window.__lknSubscriptionAt({})",
			serde_json::to_string(&at(RECEIVER_SCOPE)).expect("a scope literal")
		)),
		"https://push.test/push/theirs",
		"the other scope's subscription stayed"
	);

	remove_boundaries().await;
}

/// A stale handle whose subscription the browser has replaced leaves the
/// replacement alone: the revoke still lands, nothing is unsubscribed.
#[wasm_bindgen_test]
async fn a_stale_handle_leaves_a_replaced_subscription_alone() {
	install_boundaries().await;
	let _registration = register_fixture(FIXTURE_SCRIPT).await;
	seed(&at(FIXTURE_SCOPE), "https://push.test/push/old");

	let mut options = lkn::EnrollOptions::new(vapid_key());
	options.worker_scope = Some(FIXTURE_SCOPE.to_owned());
	let enrollment = lkn::enroll(&options).await.expect("the browser enrolls");

	// The browser rotated the subscription out from under the handle.
	seed(&at(FIXTURE_SCOPE), "https://push.test/push/new");
	lkn::disable(&enrollment)
		.await
		.expect("the stale disable still completes");

	assert_eq!(requests_to("revoke_push_endpoint").len(), 1);
	assert_eq!(
		eval_string("JSON.stringify(window.__lknUnsubscribed)"),
		"[]",
		"the replacement subscription was never touched"
	);
	assert_eq!(
		eval_string(&format!(
			"window.__lknSubscriptionAt({})",
			serde_json::to_string(&at(FIXTURE_SCOPE)).expect("a scope literal")
		)),
		"https://push.test/push/new"
	);

	remove_boundaries().await;
}

/// A missing endpoint is unconfirmed, even absent from the first list page.
#[wasm_bindgen_test]
async fn a_missing_endpoint_is_not_a_revoke_acknowledgement() {
	install_boundaries().await;
	let _registration = register_fixture(FIXTURE_SCRIPT).await;
	seed(&at(FIXTURE_SCOPE), "https://push.test/push/held");

	let mut options = lkn::EnrollOptions::new(vapid_key());
	options.worker_scope = Some(FIXTURE_SCOPE.to_owned());
	let enrollment = lkn::enroll(&options).await.expect("the browser enrolls");

	answer_revoke(404, "ServerError|no such endpoint");
	lkn::disable(&enrollment)
		.await
		.expect_err("a 404 is not an acknowledgement");
	answer_revoke(500, "ServerError|the service is down");
	assert!(matches!(
		lkn::disable(&enrollment).await,
		Err(lkn::DisableError::Revoke(_))
	));
	assert_eq!(requests_to("push_endpoints").len(), 0);

	assert_eq!(
		eval_string("JSON.stringify(window.__lknUnsubscribed)"),
		"[]",
		"the browser subscription remains reusable"
	);

	remove_boundaries().await;
}

/// Failure stays explicit; retries use a stable key through the real client.
#[wasm_bindgen_test]
async fn a_refused_revoke_stays_explicit_and_touches_nothing() {
	install_boundaries().await;
	let _registration = register_fixture(FIXTURE_SCRIPT).await;
	seed(&at(FIXTURE_SCOPE), "https://push.test/push/held");

	let mut options = lkn::EnrollOptions::new(vapid_key());
	options.worker_scope = Some(FIXTURE_SCOPE.to_owned());
	let enrollment = lkn::enroll(&options).await.expect("the browser enrolls");

	eval(&format!(
		r#"window.__lknRouteAnswers["{SERVICE_PREFIX}/revoke_push_endpoint"] = async () => {{ throw new TypeError("response lost"); }};"#
	));
	let error = lkn::disable(&enrollment)
		.await
		.expect_err("the refused revoke");
	assert!(matches!(error, lkn::DisableError::Revoke(_)));
	assert_eq!(
		eval_string("JSON.stringify(window.__lknUnsubscribed)"),
		"[]",
		"a failed revoke leaves the browser subscription for the retry"
	);

	assert_eq!(requests_to("push_endpoints").len(), 0);
	let restored: lkn::Enrollment =
		serde_json::from_str(&serde_json::to_string(&enrollment).expect("serialize handle"))
			.expect("reload handle");
	// The service recovers: the same disable now runs end to end.
	answer_revoke(200, "null");
	lkn::disable(&restored)
		.await
		.expect("the retried disable completes");
	let calls = requests_to("revoke_push_endpoint");
	assert_eq!(calls[0]["idempotency_key"], calls[1]["idempotency_key"]);
	assert_eq!(
		calls[0]["idempotency_key"],
		format!("notifications:disable:{}", enrollment.endpoint)
	);
	assert_eq!(
		eval_string("JSON.stringify(window.__lknUnsubscribed)"),
		"[]"
	);

	remove_boundaries().await;
}

/// The next account reuses the subscription, and a stale handle cannot remove
/// it.
#[wasm_bindgen_test]
async fn confirmed_revoke_reuses_subscription_and_stale_handle_never_unsubscribes() {
	install_boundaries().await;
	let _registration = register_fixture(FIXTURE_SCRIPT).await;
	seed(&at(FIXTURE_SCOPE), "https://push.test/push/reused");
	let mut options = lkn::EnrollOptions::new(vapid_key());
	options.worker_scope = Some(FIXTURE_SCOPE.to_owned());
	let old = lkn::enroll(&options).await.expect("first account");
	lkn::disable(&old).await.expect("confirmed revoke");
	// The service returns a fresh id when the next account enrolls the same
	// URL.
	eval(&format!(
		r#"
	window.__lknRouteAnswers["{SERVICE_PREFIX}/register_push_endpoint"] = async () =>
		new Response(JSON.stringify({{
			id: "6ec0f4b1-0000-7000-8000-0000000000f2", platform: "web_push",
			created_at: "2026-11-15T00:00:00Z", invalidated_at: null
		}}), {{status: 200}});
	"#
	));
	let new = lkn::enroll(&options).await.expect("next account");
	assert_ne!(old.endpoint, new.endpoint);
	answer_revoke(404, "ServerError|no such endpoint");
	assert!(matches!(
		lkn::disable(&old).await,
		Err(lkn::DisableError::Revoke(_))
	));
	assert_eq!(requests_to("push_endpoints").len(), 0);
	for call in requests_to("revoke_push_endpoint") {
		let body: serde_json::Value = serde_json::from_str(call["body"].as_str().unwrap()).unwrap();
		assert_eq!(body["endpoint"], old.endpoint.to_string());
	}
	assert_eq!(
		eval_string("JSON.stringify(window.__lknUnsubscribed)"),
		"[]"
	);
	let calls = requests_to("register_push_endpoint");
	assert_eq!(calls[0]["body"], calls[1]["body"]);
	assert_eq!(
		eval_string(&format!(
			"String(window.__lknSubscribesAt({:?}))",
			at(FIXTURE_SCOPE)
		)),
		"0"
	);
	remove_boundaries().await;
}
