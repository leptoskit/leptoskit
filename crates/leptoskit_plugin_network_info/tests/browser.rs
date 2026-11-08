#![cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]

use leptoskit_bridge::Error;
use leptoskit_plugin_network_info::web;
use wasm_bindgen_test::wasm_bindgen_test;

wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn browsers_expose_no_network_details() {
	assert_eq!(web::snapshot(), Err(Error::Unavailable));
}
