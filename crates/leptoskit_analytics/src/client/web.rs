//! The client in the browser: one per page, recording as the person the
//! browser remembers, and sending to the app's own origin.
//!
//! [`init`] starts it; the functions below record through it, and do
//! nothing before it. Batches are sent every few seconds with `fetch`, and
//! with `sendBeacon` when the page is hidden or closed, so the last events
//! aren't lost. The person's id and whether they opted out are kept in
//! `localStorage`, when the page may use it.

use std::cell::RefCell;

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
		fill_random(bytes);
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
	if !in_page() {
		return;
	}

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
	let context = context();
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

	// Sends on the interval, and hands what's left to `sendBeacon` when the
	// page is hidden or closed.
	let tick = Closure::<dyn FnMut()>::new(|| wasm_bindgen_futures::spawn_local(send_waiting()));
	let leave = Closure::<dyn FnMut()>::new(beacon);
	watch_page(interval, &tick, &leave);
	tick.forget();
	leave.forget();
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
pub fn capture_pageview(mut properties: Properties) {
	let page = page_view();
	let names = ["$current_url", "$pathname", "$host", "$title", "$referrer"];
	for (name, value) in names.into_iter().zip(page.split('\n')) {
		if !value.is_empty() && !properties.iter().any(|(have, _)| have == name) {
			properties.push((name.into(), Value::Text(value.into())));
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

fn read(key: &str) -> Option<String> {
	storage_get(key)
}

fn write(key: &str, value: Option<&str>) {
	let persist =
		STATE.with_borrow(|state| state.as_ref().is_some_and(|state| state.options.persist));
	if persist {
		storage_set(key, value);
	}
}

/// What's the same for every event from this page.
fn context() -> Properties {
	let mut context: Properties = vec![
		("$lib".into(), Value::Text("leptoskit-web".into())),
		(
			"$lib_version".into(),
			Value::Text(env!("CARGO_PKG_VERSION").into()),
		),
	];
	let page = page_context();
	let mut values = page.split('\n');
	for name in [
		"$screen_width",
		"$screen_height",
		"$viewport_width",
		"$viewport_height",
	] {
		if let Some(Ok(number)) = values.next().map(str::parse::<i64>) {
			context.push((name.into(), Value::Int(number)));
		}
	}
	for name in ["$locale", "$timezone", "$raw_user_agent"] {
		if let Some(text) = values.next().filter(|text| !text.is_empty()) {
			context.push((name.into(), Value::Text(text.into())));
		}
	}
	context
}

// The browser APIs the client uses, as small functions: one call each is
// less code, in both the wasm and its JavaScript glue, than web-sys's
// bindings and their error handling. The page's values are joined by
// newlines, which none can hold. Storage and sending never throw: storage
// that isn't there is skipped, and a failed request answers "", a retry.
// It's served as written, with nothing to minify it, so it's written small:
// `g` is `globalThis`, `t` turns a value into a line of text, and `c` runs
// a function, answering `e` if it throws.
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
const g=globalThis,t=v=>v==null?"":String(v).replace(/\n/g," "),j=a=>a.map(t).join("\n"),c=(f,e)=>{try{return f()}catch(_){return e}},s=()=>g.localStorage;
export const page_context=()=>{const w=g.screen||{},n=g.navigator||{};return j([w.width,w.height,g.innerWidth,g.innerHeight,n.language,c(()=>Intl.DateTimeFormat().resolvedOptions().timeZone,""),n.userAgent])};
export const page_view=()=>{const l=g.location||{},d=g.document||{};return j([l.href,l.pathname,l.host,d.title,d.referrer])};
export const in_page=()=>typeof window!=="undefined";
export const watch_page=(ms,tick,leave)=>{setInterval(tick,ms);addEventListener("visibilitychange",()=>{if(g.document?.visibilityState==="hidden")leave()});addEventListener("pagehide",()=>leave())};
export const fill_random=b=>{if(!c(()=>(g.crypto.getRandomValues(b),1),0))for(let i=0;i<b.length;i++)b[i]=Math.random()*256};
export const storage_get=k=>c(()=>s().getItem(k),null);
export const storage_set=(k,v)=>c(()=>v==null?s().removeItem(k):s().setItem(k,v));
export const fetch_batch=(url,body,keepalive)=>c(()=>fetch(url,{method:"POST",body,keepalive}).then(async r=>{const h=`${r.status} ${r.headers.get("retry-after")||""}|`;if(!r.ok)return h;const i=await unknown_schemas(r);return i==null?"":h+i},()=>""),Promise.resolve(""));
export const send_beacon=(u,b)=>c(()=>g.navigator.sendBeacon(u,b),false);
export async function unknown_schemas(r){try{const a=await r.json();if(!a||typeof a!=="object"||Array.isArray(a))return null;const f=a.refused??[];if(!Array.isArray(f))return null;const i=[];for(const x of f){if(!Number.isInteger(x.index)||x.index<0||x.index>4294967295||typeof x.reason!=="string")return null;if(x.reason==="unknown_schema")i.push(x.index)}return i.join(",")}catch(_){return null}}
"#)]
extern "C" {
	fn page_context() -> String;
	fn page_view() -> String;
	fn in_page() -> bool;
	fn watch_page(interval: i32, tick: &Closure<dyn FnMut()>, leave: &Closure<dyn FnMut()>);
	fn fill_random(bytes: &mut [u8]);
	fn storage_get(key: &str) -> Option<String>;
	fn storage_set(key: &str, value: Option<&str>);
	fn fetch_batch(url: &str, body: &[u8], keepalive: bool) -> js_sys::Promise;
	fn send_beacon(url: &str, body: &[u8]) -> bool;
}

/// Takes the next batch, if one may be sent now.
fn take(named: bool) -> Option<(super::Pending, String)> {
	STATE.with_borrow_mut(|state| {
		let state = state.as_mut()?;
		let now = Browser.now();
		let random = super::random(&Browser);
		let pending = if named {
			state.client.queue().take_named(now, random)?
		} else {
			state.client.queue().take(now, random)?
		};
		Some((pending, state.options.endpoint.clone()))
	})
}

fn settle(outcome: Outcome, refused: &[(u32, String)]) {
	let mut jitter = [0_u8; 1];
	Browser.fill_random(&mut jitter);
	with(|client| {
		let jitter = f64::from(jitter[0]) / 255.0;
		if outcome == Outcome::Stored {
			let refused: Vec<(u32, &str)> = refused
				.iter()
				.map(|(index, reason)| (*index, reason.as_str()))
				.collect();
			client
				.queue()
				.settle_with_refusals(&refused, Browser.now(), jitter);
		} else {
			client.queue().settle(outcome, Browser.now(), jitter);
		}
	});
}

/// The largest body sent with `keepalive`: browsers refuse more than 64 KiB
/// of keepalive requests in flight.
const KEEPALIVE_BYTES: usize = 60 << 10;

/// Sends batches with `fetch` until nothing waits or the service asks to
/// wait.
async fn send_waiting() {
	while let Some((pending, endpoint)) = take(false) {
		let (outcome, refused) = post(&endpoint, &pending.body).await;
		settle(outcome, &refused);
		if matches!(outcome, Outcome::Retry { .. }) {
			return;
		}
	}
}

async fn post(endpoint: &str, body: &[u8]) -> (Outcome, Vec<(u32, String)>) {
	let answer = JsFuture::from(fetch_batch(endpoint, body, body.len() <= KEEPALIVE_BYTES))
		.await.ok().and_then(|answer| answer.as_string()).unwrap_or_default();
	let Some((head, indices)) = answer.split_once('|') else {
		return (Outcome::Retry { after: None }, Vec::new());
	};
	let Some((status, retry_after)) = head.split_once(' ') else {
		return (Outcome::Retry { after: None }, Vec::new());
	};
	let Ok(status) = status.parse() else {
		return (Outcome::Retry { after: None }, Vec::new());
	};
	let outcome = Outcome::from_status(status, Some(retry_after).filter(|after| !after.is_empty()));
	let refused = indices.split(',').filter_map(|index| index.parse().ok())
		.map(|index| (index, "unknown_schema".to_owned())).collect();
	(outcome, refused)
}

/// Hands every waiting batch to `sendBeacon`, which sends it even as the
/// page goes away. The answer never comes back, so a batch the browser
/// accepted counts as stored.
fn beacon() {
	while let Some((pending, endpoint)) = take(true) {
		if send_beacon(&endpoint, &pending.body) {
			settle(Outcome::Stored, &[]);
		} else {
			settle(Outcome::Retry { after: None }, &[]);
			return;
		}
	}
}
