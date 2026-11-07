#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use leptoskit_bridge::Error;
use leptoskit_plugin_badge::MAX_COUNT;
use leptoskit_plugin_badge::web;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::Closure;
use wasm_bindgen_test::wasm_bindgen_test;
use wasm_bindgen_test::wasm_bindgen_test_configure;

wasm_bindgen_test_configure!(run_in_browser);

fn install_badging(succeed: bool) {
	let navigator = web_sys::window().unwrap().navigator();
	let set = Closure::wrap(Box::new(move |_count: JsValue| {
		if succeed {
			js_sys::Promise::resolve(&JsValue::UNDEFINED)
		} else {
			js_sys::Promise::reject(&js_sys::Error::new("denied").into())
		}
	}) as Box<dyn FnMut(JsValue) -> js_sys::Promise>);
	let clear = Closure::wrap(Box::new(move || {
		if succeed {
			js_sys::Promise::resolve(&JsValue::UNDEFINED)
		} else {
			js_sys::Promise::reject(&js_sys::Error::new("denied").into())
		}
	}) as Box<dyn FnMut() -> js_sys::Promise>);
	js_sys::Reflect::set(
		navigator.as_ref(),
		&JsValue::from_str("setAppBadge"),
		set.as_ref().unchecked_ref(),
	)
	.unwrap();
	js_sys::Reflect::set(
		navigator.as_ref(),
		&JsValue::from_str("clearAppBadge"),
		clear.as_ref().unchecked_ref(),
	)
	.unwrap();
	set.forget();
	clear.forget();
}

#[wasm_bindgen_test]
async fn set_get_and_clear_track_local_count() {
	install_badging(true);
	web::set(3).await.expect("set badge");
	assert_eq!(web::get().expect("read badge"), 3);
	web::clear().await.expect("clear badge");
	assert_eq!(web::get().expect("read cleared badge"), 0);
	assert_eq!(web::set(MAX_COUNT + 1).await, Err(Error::InvalidArgument));
}

#[wasm_bindgen_test]
fn missing_badging_api_is_unavailable() {
	let navigator = web_sys::window().unwrap().navigator();
	js_sys::Reflect::delete_property(navigator.as_ref(), &JsValue::from_str("setAppBadge"))
		.unwrap();
	js_sys::Reflect::delete_property(navigator.as_ref(), &JsValue::from_str("clearAppBadge"))
		.unwrap();
	assert_eq!(web::get(), Err(Error::Unavailable));
}
