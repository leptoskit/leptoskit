//! A minimal editor app for the size measurement: one `CanvistState`
//! attached to a `<div>`, built as a cdylib with
//! `cargo build --release --target wasm32-unknown-unknown --features web
//! --example editor`.

/// The editor host, exported for the wasm build.
#[cfg(all(feature = "web", target_arch = "wasm32"))]
#[unsafe(no_mangle)]
pub extern "C" fn canvist_editor_start() -> u32 {
	let window = web_sys::window().expect("window");
	let document = window.document().expect("document");
	let container: web_sys::HtmlElement = document
		.create_element("div")
		.expect("div")
		.dyn_into()
		.expect("element");
	document.body().expect("body").append_child(&container).ok();

	let note = leptoskit_canvist::CanvistState::new(leptoskit_canvist::Extensions::paragraphs());
	note.load(leptoskit_canvist::Format::Text, "Hello, canvist.");
	let _attachment = leptoskit_canvist::attach(&container, note);
	42
}

/// The example builds for every target; only the wasm build exports the host.
fn main() {}