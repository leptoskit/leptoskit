//! The HTML launch screen `[app.launch_screen]` injects into `index.html`:
//! lifted once the first render is painted, unless the app holds it.
//!
//! A test binary of its own, because `leptoskit::splash` keeps its state for
//! the whole page.

#![cfg(all(feature = "web", target_arch = "wasm32"))]

use leptos::mount::mount_to;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;
use leptoskit_native::splash;
use leptoskit_native::web::render;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
async fn the_launch_screen_fades_out_once_the_first_render_is_painted() {
	fresh_page();
	// The page's own fade-out, as the CLI injects it: it marks the splash
	// and leaves its removal for later.
	let fade = js_sys::Function::new_no_args(&format!(
		"document.getElementById('{}').classList.add('{}');",
		splash::HTML_ID,
		splash::HTML_HIDE_CLASS
	));
	js_sys::Reflect::set(
		&web_sys::window().unwrap(),
		&splash::HTML_DISMISS.into(),
		&fade,
	)
	.unwrap();
	let launch = launch_screen();
	let parent = container();
	let mount = mount_to(parent.clone(), || {
		render(View().child(Text("The first screen")))
	});
	next_frame().await;
	next_frame().await;

	assert!(
		launch.class_list().contains(splash::HTML_HIDE_CLASS),
		"the page's fade-out ran"
	);
	drop(mount);
	parent.remove();
	launch.remove();
	js_sys::Reflect::delete_property(&web_sys::window().unwrap(), &splash::HTML_DISMISS.into())
		.unwrap();
}

#[wasm_bindgen_test]
async fn a_held_launch_screen_stays_until_it_is_dismissed() {
	fresh_page();
	let launch = launch_screen();
	splash::hold();
	let parent = container();
	let mount = mount_to(parent.clone(), || {
		render(View().child(Text("Loading the session")))
	});
	next_frame().await;
	next_frame().await;
	assert!(launch.is_connected(), "held over the first render");

	// Without a fade-out of the page's own, the splash is removed.
	splash::dismiss();
	assert!(!launch.is_connected());
	assert!(!splash::visible());
	drop(mount);
	parent.remove();
}

/// Clears what a test that failed may have left: its launch screen and its
/// fade-out.
fn fresh_page() {
	while let Some(left) = document().get_element_by_id(splash::HTML_ID) {
		left.remove();
	}
	js_sys::Reflect::delete_property(&web_sys::window().unwrap(), &splash::HTML_DISMISS.into())
		.unwrap();
}

/// Puts the launch screen over the page, as `index.html` has it.
fn launch_screen() -> web_sys::HtmlElement {
	let launch = document()
		.create_element("div")
		.unwrap()
		.dyn_into::<web_sys::HtmlElement>()
		.unwrap();
	launch.set_id(splash::HTML_ID);
	document().body().unwrap().append_child(&launch).unwrap();
	launch
}

fn document() -> web_sys::Document {
	web_sys::window().unwrap().document().unwrap()
}

fn container() -> web_sys::HtmlElement {
	let parent = document()
		.create_element("div")
		.unwrap()
		.dyn_into::<web_sys::HtmlElement>()
		.unwrap();
	document().body().unwrap().append_child(&parent).unwrap();
	parent
}

/// Waits for the browser's next animation frame.
async fn next_frame() {
	let frame = js_sys::Promise::new(&mut |resolve, _| {
		web_sys::window()
			.unwrap()
			.request_animation_frame(&resolve)
			.unwrap();
	});
	wasm_bindgen_futures::JsFuture::from(frame).await.unwrap();
}
