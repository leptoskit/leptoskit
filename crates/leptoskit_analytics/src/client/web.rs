//! The client in the browser: one per page, recording as the person the
//! browser remembers, and sending to the app's own origin.
//!
//! [`init`] starts it; the functions below record through it, and do
//! nothing before it. Batches are sent every few seconds with `fetch`, and
//! with `sendBeacon` when the page is hidden or closed, so the last events
//! aren't lost. The person's id and whether they opted out are kept in
//! `localStorage`, when the page may use it.

use std::cell::RefCell;

use js_sys::Uint8Array;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::JsFuture;

use super::Client;
use super::Config;
use super::Identity;
use super::Outcome;
use super::Platform;
use crate::SERVICE;
use crate::wire::Properties;
use crate::wire::Value;

/// How the browser's client sends.
#[derive(Clone, Debug)]
pub struct Options {
	pub config: Config,
	/// Where batches go: the service's route on the app's origin.
	pub endpoint: String,
	/// How often waiting events are sent, in milliseconds.
	pub interval: i32,
	/// Whether the person's id and consent are kept in `localStorage`.
	/// Without it, every page load is someone new.
	pub persist: bool,
}

impl Default for Options {
	fn default() -> Self {
		Self {
			config: Config::default(),
			endpoint: format!("/_leptoskit/services/{SERVICE}/v1/batch"),
			interval: 3_000,
			persist: true,
		}
	}
}

/// The browser's clock and `crypto.getRandomValues`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Browser;

impl Platform for Browser {
	fn now(&self) -> i64 {
		js_sys::Date::now() as i64
	}

	fn fill_random(&self, bytes: &mut [u8]) {
		let filled = web_sys::window()
			.and_then(|window| window.crypto().ok())
			.is_some_and(|crypto| crypto.get_random_values_with_u8_array(bytes).is_ok());
		if !filled {
			for byte in bytes {
				*byte = (js_sys::Math::random() * 256.0) as u8;
			}
		}
	}
}

struct State {
	client: Client<Browser>,
	options: Options,
}

thread_local! {
	static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

const ID_KEY: &str = "lk_analytics_id";
const OPT_OUT_KEY: &str = "lk_analytics_opt_out";

/// Starts the page's client. Calling it again does nothing.
pub fn init(options: Options) {
	if STATE.with_borrow(Option::is_some) {
		return;
	}
	let Some(window) = web_sys::window() else {
		return;
	};

	let stored = options.persist.then(|| read(ID_KEY)).flatten();
	let identity = stored.and_then(|stored| {
		let (kind, id) = stored.split_once(':')?;
		(!id.is_empty()).then(|| {
			Identity {
				distinct_id: id.into(),
				identified: kind == "i",
			}
		})
	});
	let context = context(&window);
	let mut client = match identity {
		Some(identity) => Client::restore(Browser, options.config.clone(), context, identity),
		None => Client::new(Browser, options.config.clone(), context),
	};
	if options.persist && read(OPT_OUT_KEY).as_deref() == Some("1") {
		client.opt_out();
	}
	let interval = options.interval;
	STATE.with_borrow_mut(|state| *state = Some(State { client, options }));
	save_identity();

	let tick = Closure::<dyn FnMut()>::new(|| wasm_bindgen_futures::spawn_local(send_waiting()));
	let _ = window.set_interval_with_callback_and_timeout_and_arguments_0(
		tick.as_ref().unchecked_ref(),
		interval,
	);
	tick.forget();

	let hidden = Closure::<dyn FnMut()>::new(|| {
		let hidden = web_sys::window()
			.and_then(|window| window.document())
			.is_some_and(|document| {
				document.visibility_state() == web_sys::VisibilityState::Hidden
			});
		if hidden {
			beacon();
		}
	});
	let _ = window
		.add_event_listener_with_callback("visibilitychange", hidden.as_ref().unchecked_ref());
	hidden.forget();
	let leaving = Closure::<dyn FnMut()>::new(beacon);
	let _ = window.add_event_listener_with_callback("pagehide", leaving.as_ref().unchecked_ref());
	leaving.forget();
}

/// Records that `name` happened.
pub fn capture(name: &str, properties: Properties) {
	with(|client| client.capture(name, properties));
}

/// Records a typed event with its declared name and fields.
pub fn capture_event<E: crate::Event>(event: E) {
	with(|client| client.capture_event(event));
}

/// Records a page view of the current page.
pub fn capture_pageview(properties: Properties) {
	let mut properties = properties;
	if let Some(window) = web_sys::window() {
		let location = window.location();
		let mut add = |name: &str, value: Result<String, JsValue>| {
			if let Ok(value) = value
				&& !value.is_empty()
				&& !properties.iter().any(|(have, _)| have == name)
			{
				properties.push((name.into(), Value::Text(value)));
			}
		};
		add("$current_url", location.href());
		add("$pathname", location.pathname());
		add("$host", location.host());
		if let Some(document) = window.document() {
			add("$title", Ok(document.title()));
			add("$referrer", Ok(document.referrer()));
		}
	}
	capture("$pageview", properties);
}

/// The person signed in as `distinct_id`; see [`Client::identify`].
pub fn identify(distinct_id: &str, set: Properties) {
	with(|client| client.identify(distinct_id, set));
	save_identity();
}

/// `alias` is another id for the person.
pub fn alias(alias: &str) {
	with(|client| client.alias(alias));
}

/// Sets the person's properties; see [`Client::set`].
pub fn set(set: Properties, set_once: Properties) {
	with(|client| client.set(set, set_once));
}

/// Events from now on are about this group too; see [`Client::group`].
pub fn group(group_type: &str, key: &str, set: Properties) {
	with(|client| client.group(group_type, key, set));
}

/// Adds `properties` to every event from now on.
pub fn register(properties: Properties) {
	with(|client| client.register(properties));
}

/// [`register`], but only names not already registered.
pub fn register_once(properties: Properties) {
	with(|client| client.register_once(properties));
}

/// Stops adding `name` to events.
pub fn unregister(name: &str) {
	with(|client| client.unregister(name));
}

/// The person signed out: someone new from now on.
pub fn reset() {
	with(Client::reset);
	save_identity();
}

/// Stops recording, drops what's waiting, and remembers the choice.
pub fn opt_out() {
	with(Client::opt_out);
	write(OPT_OUT_KEY, Some("1"));
}

/// Records again after [`opt_out`].
pub fn opt_in() {
	with(Client::opt_in);
	write(OPT_OUT_KEY, None);
}

/// Who events are recorded as, once [`init`] has run.
pub fn distinct_id() -> Option<String> {
	STATE.with_borrow(|state| {
		state
			.as_ref()
			.map(|state| state.client.identity().distinct_id.clone())
	})
}

/// Sends what's waiting now, rather than on the next tick.
pub async fn flush() {
	send_waiting().await;
}

fn with(f: impl FnOnce(&mut Client<Browser>)) {
	STATE.with_borrow_mut(|state| {
		if let Some(state) = state {
			f(&mut state.client);
		}
	});
}

fn save_identity() {
	let saved = STATE.with_borrow(|state| {
		state
			.as_ref()
			.filter(|state| state.options.persist)
			.map(|state| {
				let identity = state.client.identity();
				let kind = if identity.identified { "i" } else { "a" };
				format!("{kind}:{}", identity.distinct_id)
			})
	});
	if let Some(saved) = saved {
		write(ID_KEY, Some(&saved));
	}
}

fn storage() -> Option<web_sys::Storage> {
	web_sys::window()?.local_storage().ok().flatten()
}

fn read(key: &str) -> Option<String> {
	storage()?.get_item(key).ok().flatten()
}

fn write(key: &str, value: Option<&str>) {
	let persist =
		STATE.with_borrow(|state| state.as_ref().is_some_and(|state| state.options.persist));
	let Some(storage) = storage().filter(|_| persist) else {
		return;
	};
	let _ = match value {
		Some(value) => storage.set_item(key, value),
		None => storage.remove_item(key),
	};
}

/// What's the same for every event from this page.
fn context(window: &web_sys::Window) -> Properties {
	let mut context: Properties = vec![
		("$lib".into(), Value::Text("leptoskit-web".into())),
		(
			"$lib_version".into(),
			Value::Text(env!("CARGO_PKG_VERSION").into()),
		),
	];
	let mut number = |name: &str, value: Result<JsValue, JsValue>| {
		if let Some(value) = value.ok().and_then(|value| value.as_f64()) {
			context.push((name.into(), Value::Int(value as i64)));
		}
	};
	if let Ok(screen) = window.screen() {
		number("$screen_width", screen.width().map(JsValue::from));
		number("$screen_height", screen.height().map(JsValue::from));
	}
	number("$viewport_width", window.inner_width());
	number("$viewport_height", window.inner_height());
	let navigator = window.navigator();
	if let Some(locale) = navigator.language() {
		context.push(("$locale".into(), Value::Text(locale)));
	}
	if let Ok(agent) = navigator.user_agent() {
		context.push(("$raw_user_agent".into(), Value::Text(agent)));
	}
	let options = js_sys::Intl::DateTimeFormat::new(&js_sys::Array::new(), &js_sys::Object::new())
		.resolved_options();
	if let Some(zone) = js_sys::Reflect::get(&options, &"timeZone".into())
		.ok()
		.and_then(|zone| zone.as_string())
	{
		context.push(("$timezone".into(), Value::Text(zone)));
	}
	context
}

/// Takes the next batch, if one may be sent now.
fn take() -> Option<(super::Pending, String)> {
	STATE.with_borrow_mut(|state| {
		let state = state.as_mut()?;
		let now = Browser.now();
		let pending = state.client.queue().take(now, super::random(&Browser))?;
		Some((pending, state.options.endpoint.clone()))
	})
}

fn settle(outcome: Outcome) {
	let mut jitter = [0_u8; 1];
	Browser.fill_random(&mut jitter);
	with(|client| {
		client
			.queue()
			.settle(outcome, Browser.now(), f64::from(jitter[0]) / 255.0);
	});
}

/// The largest body sent with `keepalive`: browsers refuse more than 64 KiB
/// of keepalive requests in flight.
const KEEPALIVE_BYTES: usize = 60 << 10;

/// Sends batches with `fetch` until nothing waits or the service asks to
/// wait.
async fn send_waiting() {
	while let Some((pending, endpoint)) = take() {
		let outcome = post(&endpoint, &pending.body).await;
		settle(outcome);
		if matches!(outcome, Outcome::Retry { .. }) {
			return;
		}
	}
}

async fn post(endpoint: &str, body: &[u8]) -> Outcome {
	let Some(window) = web_sys::window() else {
		return Outcome::Retry { after: None };
	};
	let init = web_sys::RequestInit::new();
	init.set_method("POST");
	init.set_body(&Uint8Array::from(body));
	if body.len() <= KEEPALIVE_BYTES {
		let _ = js_sys::Reflect::set(&init, &"keepalive".into(), &JsValue::TRUE);
	}
	let response = JsFuture::from(window.fetch_with_str_and_init(endpoint, &init))
		.await
		.ok()
		.and_then(|response| response.dyn_into::<web_sys::Response>().ok());
	match response {
		Some(response) => {
			let retry_after = response.headers().get("retry-after").ok().flatten();
			Outcome::from_status(response.status(), retry_after.as_deref())
		}
		None => Outcome::Retry { after: None },
	}
}

/// Hands every waiting batch to `sendBeacon`, which sends it even as the
/// page goes away. The answer never comes back, so a batch the browser
/// accepted counts as stored.
fn beacon() {
	let Some(navigator) = web_sys::window().map(|window| window.navigator()) else {
		return;
	};
	while let Some((pending, endpoint)) = take() {
		let queued = navigator
			.send_beacon_with_opt_js_u8_array(&endpoint, Some(&Uint8Array::from(&pending.body[..])))
			.unwrap_or(false);
		if queued {
			settle(Outcome::Stored);
		} else {
			settle(Outcome::Retry { after: None });
			return;
		}
	}
}
