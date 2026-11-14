//! The browser's client in a real browser, with `fetch` replaced so the
//! batches it sends can be read back.

#![cfg(all(target_arch = "wasm32", feature = "web"))]

use leptoskit_analytics::client::web;
use leptoskit_analytics::props;
use leptoskit_analytics::wire;
use leptoskit_analytics::wire::Body;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen(inline_js = r#"
const sent = [];
const original = window.fetch;
const originalBeacon = navigator.sendBeacon;
export function restoreFetch() { window.fetch = original; navigator.sendBeacon = originalBeacon; }
export function interceptFetch(status) {
  sent.length = 0;
  window.fetch = async (url, init) => {
    sent.push({ url, keepalive: init.keepalive === true, body: new Uint8Array(init.body) });
    return new Response('{}', { status, headers: status === 503 ? { 'retry-after': '1' } : {} });
  };
}
export function sentCount() { return sent.length; }
export function sentUrl(index) { return sent[index].url; }
export function sentKeepalive(index) { return sent[index].keepalive; }
export function sentBody(index) { return sent[index].body; }
export function clearStorage() { localStorage.clear(); }
export function stored(key) { return localStorage.getItem(key); }
export function interceptTyped(mode) {
  sent.length = 0;
  window.fetch = async (url, init) => {
    sent.push({ url, body: new Uint8Array(init.body) });
    const first = sent.length === 1;
    const body = mode === 'broken' ? 'broken' : mode === 'unknown' && first
      ? '{"refused":[{"index":0,"reason":"unknown_schema"}]}' : '{}';
    return new Response(body, { status: mode === 'old' && first ? 400 : 200 });
  };
}
export function sendOnHide() {
  sent.length = 0;
  navigator.sendBeacon = (url, body) => { sent.push({url, body:new Uint8Array(body)}); return true; };
  window.dispatchEvent(new Event('pagehide'));
}
export function pushStateUrl(url) { history.pushState(null, "", url); }
export function replaceStateUrl(url) { history.replaceState(null, "", url); }

"#)]
extern "C" {
	#[wasm_bindgen(js_name = interceptFetch)]
	fn intercept_fetch(status: u16);
	#[wasm_bindgen(js_name = sentCount)]
	fn sent_count() -> u32;
	#[wasm_bindgen(js_name = sentUrl)]
	fn sent_url(index: u32) -> String;
	#[wasm_bindgen(js_name = sentKeepalive)]
	fn sent_keepalive(index: u32) -> bool;
	#[wasm_bindgen(js_name = sentBody)]
	fn sent_body(index: u32) -> Vec<u8>;
	#[wasm_bindgen(js_name = clearStorage)]
	fn clear_storage();
	fn stored(key: &str) -> Option<String>;
	#[wasm_bindgen(js_name = restoreFetch)]
	fn restore_fetch();
	#[wasm_bindgen(js_name = interceptTyped)]
	fn intercept_typed(mode: &str);
	#[wasm_bindgen(js_name = sendOnHide)]
	fn send_on_hide();
	#[wasm_bindgen(js_name = pushStateUrl)]
	fn push_state_url(url: &str);
	#[wasm_bindgen(js_name = replaceStateUrl)]
	fn replace_state_url(url: &str);
}

#[cfg(all(feature = "derive", feature = "typed"))]
#[derive(leptoskit_analytics::Event)]
struct TypedCheckout {
	plan: String,
}

fn sent(index: u32) -> wire::Batch {
	wire::decode(&sent_body(index), &wire::Limits::default()).unwrap()
}

fn name(entry: &wire::Entry) -> String {
	match &entry.body {
		Body::Event { name, .. } => name.clone(),
		Body::Identify { .. } => "$identify".into(),
		_ => String::new(),
	}
}

// One test, because the client is one per page.
#[wasm_bindgen_test]
async fn the_browser_records_identifies_and_remembers() {
	clear_storage();
	intercept_fetch(503);
	web::init(web::Options {
		interval: 60_000,
		pageviews: false,
		..web::Options::default()
	});
	let anonymous = web::distinct_id().unwrap();
	assert_eq!(stored("lk_analytics_id"), Some(format!("a:{anonymous}")));

	web::capture_pageview(props! {});
	web::capture("clicked", props! { "button" => "buy" });
	web::flush().await;
	assert_eq!(sent_count(), 1);
	assert_eq!(
		sent_url(0),
		"/_leptoskit/services/analytics/v1/batch",
		"the app's own origin"
	);
	assert!(sent_keepalive(0));

	// The service said to retry later, so nothing more goes until then.
	web::flush().await;
	assert_eq!(sent_count(), 1);

	intercept_fetch(200);
	web::identify("user-7", props! { "plan" => "pro" });
	assert_eq!(stored("lk_analytics_id").as_deref(), Some("i:user-7"));
	web::capture("bought", props! {});

	// The retry waits about a second.
	gloo_timers_sleep(2_100).await;
	web::flush().await;
	assert_eq!(sent_count(), 1);
	let batch = sent(0);
	let names: Vec<String> = batch.entries.iter().map(name).collect();
	assert_eq!(names, ["$pageview", "clicked", "$identify", "bought"]);
	assert_eq!(batch.entries[0].distinct_id, anonymous);
	assert_eq!(batch.entries[3].distinct_id, "user-7");
	let Body::Event { properties, .. } = &batch.entries[0].body else {
		panic!("not an event");
	};
	assert!(properties.iter().any(|(name, _)| name == "$pathname"));
	let context = &batch.contexts[0];
	assert!(context.iter().any(|(name, value)| {
		name == "$lib" && *value == wire::Value::Text("leptoskit-web".into())
	}));
	assert!(context.iter().any(|(name, _)| name == "$timezone"));

	web::opt_out();
	assert_eq!(stored("lk_analytics_opt_out").as_deref(), Some("1"));
	web::capture("ignored", props! {});
	web::flush().await;
	assert_eq!(sent_count(), 1, "nothing recorded after opting out");
	web::opt_in();
	assert_eq!(stored("lk_analytics_opt_out"), None);

	web::reset();
	assert_ne!(web::distinct_id().unwrap(), "user-7");
	assert!(stored("lk_analytics_id").unwrap().starts_with("a:"));
	#[cfg(all(feature = "derive", feature = "typed"))]
	{
		for mode in ["unknown", "old"] {
			intercept_typed(mode);
			web::capture_event(TypedCheckout { plan: "pro".into() });
			web::capture("untyped", props! {});
			web::flush().await;
			assert_eq!(sent_count(), 2, "{mode}");
			let first = sent(0);
			let retry = sent(1);
			assert!(matches!(first.entries[0].body, Body::TypedEvent { .. }));
			assert_eq!(retry.entries[0].uuid, first.entries[0].uuid);
			assert!(
				matches!(&retry.entries[0].body, Body::Event { properties, .. }
				if properties.contains(&("plan".into(), wire::Value::Text("pro".into()))))
			);
			assert_eq!(retry.entries.len(), if mode == "old" { 2 } else { 1 });
		}
		intercept_typed("broken");
		web::capture_event(TypedCheckout {
			plan: "retry".into(),
		});
		web::flush().await;
		assert_eq!(sent_count(), 1);
		let waiting = sent(0).entries[0].uuid;
		intercept_fetch(200);
		gloo_timers_sleep(2_100).await;
		web::flush().await;
		assert_eq!(sent(0).entries[0].uuid, waiting);
		web::capture_event(TypedCheckout {
			plan: "beacon".into(),
		});
		send_on_hide();
		assert_eq!(sent_count(), 1);
		assert!(
			matches!(sent(0).entries[0].body, Body::Event { .. }),
			"beacons cannot read schema refusals"
		);
	}

	restore_fetch();
}

async fn gloo_timers_sleep(millis: i32) {
	let promise = js_sys::Promise::new(&mut |resolve, _| {
		web_sys::window()
			.unwrap()
			.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, millis)
			.unwrap();
	});
	wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

/// The automatic views: the session's first `$pageview` carries the UTM
/// parameters and the referrer, every navigation pairs a `$pageleave`
/// with the next `$pageview`, and opting the page out stops all of it.
#[wasm_bindgen_test]
async fn views_are_recorded_across_navigations_and_can_be_turned_off() {
	clear_storage();
	push_state_url("/landing?utm_source=google&utm_medium=organic");
	intercept_fetch(200);
	web::init(web::Options {
		interval: 60_000,
		..web::Options::default()
	});

	// The view of the landing page, with where the session arrived from.
	web::flush().await;
	assert_eq!(sent_count(), 1);
	let batch = sent(0);
	assert_eq!(batch.entries.len(), 1);
	assert_eq!(name(&batch.entries[0]), "$pageview");
	let Body::Event { properties, .. } = &batch.entries[0].body else {
		panic!("not an event");
	};
	let property = |wanted: &str| {
		properties
			.iter()
			.find(|(name, _)| name == wanted)
			.map(|(_, value)| value.clone())
	};
	assert_eq!(
		property("utm_source"),
		Some(wire::Value::Text("google".into()))
	);
	assert_eq!(
		property("utm_medium"),
		Some(wire::Value::Text("organic".into()))
	);
	assert!(property("$pathname").is_some());

	// A history navigation ends the page and starts the next one, without
	// repeating the session's origin.
	push_state_url("/pricing");
	web::flush().await;
	assert_eq!(sent_count(), 2);
	let batch = sent(1);
	assert_eq!(batch.entries.len(), 2);
	assert_eq!(name(&batch.entries[0]), "$pageleave");
	let Body::Event { properties, .. } = &batch.entries[0].body else {
		panic!("not an event");
	};
	assert!(
		properties
			.iter()
			.any(|(name, value)| name == "$duration" && matches!(value, wire::Value::Float(_)))
	);
	assert_eq!(name(&batch.entries[1]), "$pageview");
	let Body::Event { properties, .. } = &batch.entries[1].body else {
		panic!("not an event");
	};
	assert_eq!(
		properties
			.iter()
			.find(|(name, _)| name == "$pathname")
			.map(|(_, value)| value.clone()),
		Some(wire::Value::Text("/pricing".into()))
	);
	assert!(
		!properties.iter().any(|(name, _)| name == "utm_source"),
		"the session's origin belongs to its first view only"
	);

	// `replaceState` navigates too.
	replace_state_url("/checkout");
	web::flush().await;
	assert_eq!(sent_count(), 3);

	// Opted out, the page records no views of its own.
	web::opt_out();
	push_state_url("/ignored");
	web::flush().await;
	assert_eq!(sent_count(), 3, "nothing recorded after opting out");
	web::opt_in();

	restore_fetch();
}
