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

fn option<'a>(options: &'a js_sys::Object, name: &str) -> JsValue {
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
	assert!(opened() == vec![format!("{}/inbox?lkn={}", origin(), id)]);

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
	assert!(opened().is_empty());

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
