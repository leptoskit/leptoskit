//! The browser client's automatic views in a real browser, with `fetch`
//! replaced so the batches it sends can be read back.
//!
//! A page has one client and `init` starts it once, so this test has a
//! page of its own: beside `web.rs`'s test, whichever ran second found the
//! other's client already started with the other's options.

#![cfg(all(target_arch = "wasm32", feature = "web"))]

use leptoskit_analytics::client::web;
use leptoskit_analytics::wire;
use leptoskit_analytics::wire::Body;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen(inline_js = r#"
const sent = [];
const original = window.fetch;
export function restoreFetch() { window.fetch = original; }
export function interceptFetch(status) {
  sent.length = 0;
  window.fetch = async (url, init) => {
    sent.push({ url, body: new Uint8Array(init.body) });
    return new Response('{}', { status });
  };
}
export function sentCount() { return sent.length; }
export function sentBody(index) { return sent[index].body; }
export function clearStorage() { localStorage.clear(); }
export function pushStateUrl(url) { history.pushState(null, "", url); }
export function replaceStateUrl(url) { history.replaceState(null, "", url); }
"#)]
extern "C" {
	#[wasm_bindgen(js_name = interceptFetch)]
	fn intercept_fetch(status: u16);
	#[wasm_bindgen(js_name = sentCount)]
	fn sent_count() -> u32;
	#[wasm_bindgen(js_name = sentBody)]
	fn sent_body(index: u32) -> Vec<u8>;
	#[wasm_bindgen(js_name = clearStorage)]
	fn clear_storage();
	#[wasm_bindgen(js_name = restoreFetch)]
	fn restore_fetch();
	#[wasm_bindgen(js_name = pushStateUrl)]
	fn push_state_url(url: &str);
	#[wasm_bindgen(js_name = replaceStateUrl)]
	fn replace_state_url(url: &str);
}

fn sent(index: u32) -> wire::Batch {
	wire::decode(&sent_body(index), &wire::Limits::default()).unwrap()
}

fn name(entry: &wire::Entry) -> String {
	match &entry.body {
		Body::Event { name, .. } => name.clone(),
		_ => String::new(),
	}
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
