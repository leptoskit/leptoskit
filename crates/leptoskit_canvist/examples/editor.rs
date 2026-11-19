//! A page with one editor and nothing else.
//!
//! It is the smallest program that puts an editor on screen, which makes it
//! the one whose Wasm size the readme reports. Build it with:
//!
//! ```sh
//! cargo build --release --target wasm32-unknown-unknown \
//!   --package leptoskit_canvist --features web --example editor
//! ```
//!
//! An app would not mount an editor this way: the `leptoskit` element that
//! does it for you comes with the next change.
#![cfg(target_arch = "wasm32")]

use leptoskit_canvist::CanvistError;
use leptoskit_canvist::CanvistState;
use leptoskit_canvist::Extensions;
use leptoskit_canvist::Format;
use leptoskit_canvist::web;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::wasm_bindgen;
use web_sys::HtmlElement;

/// How the page's one editor looks. The editor takes its font, colours and
/// width from here.
const STYLE: &str = "max-width:40rem;margin:2rem auto;padding:1rem;font:1rem/1.5 \
                     system-ui,sans-serif;border:1px solid;border-radius:0.5rem";

/// Put an editor in the page when the module loads.
///
/// # Errors
///
/// When there is no page to put it in, or the browser has no 2D canvas.
#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
	let document = web_sys::window()
		.and_then(|window| window.document())
		.ok_or_else(|| JsValue::from_str("the example needs a page"))?;
	let body = document
		.body()
		.ok_or_else(|| JsValue::from_str("the example needs a page with a body"))?;
	let container: HtmlElement = document.create_element("div")?.unchecked_into();
	container.style().set_css_text(STYLE);
	container.set_attribute("aria-label", "Note")?;
	body.append_child(&container)?;

	let note = CanvistState::new(Extensions::paragraphs());
	note.set_placeholder("Write a note…");
	note.load(
		Format::Text,
		"Hello from Rust.\nNo script was written for this page.",
	)
	.map_err(|error| describe(&error))?;
	let attachment = web::attach(&container, note).map_err(|error| describe(&error))?;
	note.focus();
	// The page shows this editor for as long as it is open.
	std::mem::forget(attachment);

	Ok(())
}

fn describe(error: &CanvistError) -> JsValue {
	JsValue::from_str(&error.to_string())
}
