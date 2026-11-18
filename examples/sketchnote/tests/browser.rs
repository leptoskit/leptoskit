//! The notebook in a real browser: the page shows its sketch, the pointer
//! draws on it, and the buttons and the status line follow.
#![cfg(all(target_arch = "wasm32", feature = "web"))]

use std::future::Future;

use leptoskit_canvist::Format;
use sketchnote_example::PAGE;
use sketchnote_example::Summary;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;
use web_sys::Element;
use web_sys::HtmlElement;
use web_sys::PointerEvent;
use web_sys::PointerEventInit;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
	web_sys::window().unwrap().document().unwrap()
}

/// Wait for the browser to run one animation frame.
async fn frame() {
	let promise = js_sys::Promise::new(&mut |resolve, _| {
		web_sys::window()
			.unwrap()
			.request_animation_frame(&resolve)
			.unwrap();
	});
	JsFuture::from(promise).await.unwrap();
}

/// Wait until `ready` answers, giving up after a few seconds of frames.
async fn until<T>(what: &str, ready: impl Fn() -> Option<T>) -> T {
	for _ in 0..300 {
		if let Some(value) = ready() {
			return value;
		}

		frame().await;
	}

	panic!("{what}, but after 300 frames it still had not");
}

/// Run a future that is ready at once, as exports are on the web.
fn now<T>(future: impl Future<Output = T>) -> T {
	let mut future = std::pin::pin!(future);
	let mut context = std::task::Context::from_waker(std::task::Waker::noop());

	match future.as_mut().poll(&mut context) {
		std::task::Poll::Ready(value) => value,
		std::task::Poll::Pending => panic!("an export on the web is ready at once"),
	}
}

/// The button a person would call `name`: its text, or its label.
fn button(name: &str) -> HtmlElement {
	let buttons = document().query_selector_all("button").unwrap();

	(0..buttons.length())
		.filter_map(|index| buttons.item(index)?.dyn_into::<HtmlElement>().ok())
		.find(|button| {
			button.get_attribute("aria-label").as_deref() == Some(name)
				|| button
					.text_content()
					.is_some_and(|text| text.trim() == name)
		})
		.unwrap_or_else(|| panic!("the screen has a {name} button"))
}

/// Whether the page shows `text` anywhere a person can read it.
fn shows(text: &str) -> Option<()> {
	document()
		.body()
		.unwrap()
		.inner_text()
		.contains(text)
		.then_some(())
}

/// Send a mouse pointer event at `(x, y)` from the top left of `canvas`.
fn pointer(canvas: &Element, kind: &str, (x, y): (f64, f64)) {
	let bounds = canvas.get_bounding_client_rect();
	let init = PointerEventInit::new();
	init.set_client_x((bounds.left() + x) as i32);
	init.set_client_y((bounds.top() + y) as i32);
	init.set_button(0);
	init.set_pointer_id(1);
	init.set_pointer_type("mouse");
	init.set_bubbles(true);
	init.set_cancelable(true);
	let event = PointerEvent::new_with_event_init_dict(kind, &init).unwrap();
	canvas.dispatch_event(&event).unwrap();
}

#[wasm_bindgen_test]
async fn a_person_draws_on_the_page_and_the_screen_keeps_count() {
	let notebook = sketchnote_example::mount();
	let saved = Summary::of(PAGE);
	let summary = || {
		let json = now(notebook.note.export(Format::Json)).expect("a live editor");
		Summary::of(&json)
	};

	// The editor finds its element and draws itself there.
	let canvas = until("the editor shows in its element", || {
		document()
			.query_selector("[data-lk-canvist-editor] [data-canvist=canvas]")
			.unwrap()
	})
	.await;
	until("the status line counts the page", || {
		shows(&saved.to_string())
	})
	.await;

	// Text above and below, and between them a sketch two fifths as tall
	// as the page is wide: the middle of the canvas is on the sketch.
	button("Rose pen").click();
	button("Marker").click();
	assert_eq!(notebook.note.pen(), sketchnote_example::pen(1, 1));

	let bounds = canvas.get_bounding_client_rect();
	let middle = (bounds.width() / 2.0, bounds.height() / 2.0);
	pointer(&canvas, "pointerdown", middle);
	pointer(&canvas, "pointermove", (middle.0 + 60.0, middle.1 + 20.0));
	pointer(&canvas, "pointerup", (middle.0 + 60.0, middle.1 + 20.0));

	let drawn = Summary {
		strokes: saved.strokes + 1,
		..saved
	};
	assert_eq!(summary(), drawn, "the stroke is in the document");
	let json = now(notebook.note.export(Format::Json)).expect("a live editor");
	assert!(
		json.contains(r##""color":"#e11d48""##) && json.contains(r#""width":14.0"#),
		"in the pen that was picked"
	);
	until("the status line counts the stroke", || {
		shows(&drawn.to_string())
	})
	.await;

	button("Add a sketch").click();
	let added = Summary {
		sketches: 2,
		..drawn
	};
	assert_eq!(summary(), added);
	until("the status line counts the new sketch", || {
		shows(&added.to_string())
	})
	.await;

	button("Undo").click();
	button("Undo").click();
	assert_eq!(summary(), saved);
	until("the status line counts the page as it was", || {
		shows(&saved.to_string())
	})
	.await;

	// None of that rebuilt the screen: the editor is where it was.
	assert!(canvas.is_connected(), "the editor kept its canvas");
	notebook.note.dispose();
}
