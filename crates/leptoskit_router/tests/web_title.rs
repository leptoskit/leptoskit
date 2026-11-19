#![cfg(all(feature = "web", target_arch = "wasm32"))]

use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn set_title_names_the_browser_tab() {
	let document = web_sys::window().unwrap().document().unwrap();

	leptoskit_router::set_title("Plans · Admin");
	assert_eq!(document.title(), "Plans · Admin");

	leptoskit_router::set_title("People · Admin");
	assert_eq!(document.title(), "People · Admin");
}
