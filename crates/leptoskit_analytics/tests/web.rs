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

#[wasm_bindgen(inline_js = r"
const sent = [];
const original = window.fetch;
export function restoreFetch() { window.fetch = original; }
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
")]
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
