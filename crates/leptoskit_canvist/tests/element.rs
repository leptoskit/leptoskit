//! The editor in a leptoskit `CanvistEditor` element, in a real browser.
//!
//! The app only renders the element with the editor's id. These tests check
//! that the editor then appears in it, follows the element's settings, and
//! leaves with it, on a page built in the browser and on one the server
//! rendered.
#![cfg(all(feature = "web", target_arch = "wasm32"))]

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use leptos::mount::hydrate_from;
use leptos::mount::mount_to;
use leptos::prelude::*;
use leptos::task::tick;
use leptoskit_canvist::CanvistState;
use leptoskit_canvist::Extensions;
use leptoskit_canvist::Format;
use leptoskit_native::Element;
use leptoskit_native::EventOutcome;
use leptoskit_native::NativeEventSlot;
use leptoskit_native::components::CanvistEditor;
use leptoskit_native::components::View;
use leptoskit_native::web::render;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;
use web_sys::HtmlElement;
use web_sys::HtmlTextAreaElement;
use web_sys::InputEvent;
use web_sys::InputEventInit;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
	web_sys::window().unwrap().document().unwrap()
}

fn parent() -> HtmlElement {
	let parent: HtmlElement = document()
		.create_element("div")
		.unwrap()
		.dyn_into()
		.unwrap();
	document().body().unwrap().append_child(&parent).unwrap();
	parent
}

/// Let the page notice what changed and paint: the watcher hears of new
/// elements after the code that added them returns, and the editor paints
/// on the next animation frame.
async fn settle() {
	tick().await;

	for _ in 0..2 {
		let promise = js_sys::Promise::new(&mut |resolve, _| {
			web_sys::window()
				.unwrap()
				.request_animation_frame(&resolve)
				.unwrap();
		});
		JsFuture::from(promise).await.unwrap();
	}
}

/// The elements in `parent` that hold an editor's place.
fn containers(parent: &HtmlElement) -> Vec<HtmlElement> {
	let found = parent
		.query_selector_all("[data-lk-canvist-editor]")
		.unwrap();

	(0..found.length())
		.map(|index| found.item(index).unwrap().dyn_into().unwrap())
		.collect()
}

/// The text field of the editor in `container`, when an editor is there.
fn field(container: &HtmlElement) -> Option<HtmlTextAreaElement> {
	container
		.query_selector("textarea[data-canvist=input]")
		.unwrap()
		.map(|field| field.dyn_into().unwrap())
}

fn has_canvas(container: &HtmlElement) -> bool {
	container
		.query_selector("canvas[data-canvist=canvas]")
		.unwrap()
		.is_some()
}

/// Type `text` the way a browser reports it to a text field.
fn type_text(field: &HtmlTextAreaElement, text: &str) {
	let init = InputEventInit::new();
	init.set_input_type("insertText");
	init.set_data(Some(text));
	init.set_bubbles(true);
	init.set_cancelable(true);
	let event = InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();

	assert!(
		!field.dispatch_event(&event).unwrap(),
		"the editor takes typing over"
	);
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

fn text(note: CanvistState) -> String {
	now(note.export(Format::Text)).expect("a live editor")
}

#[wasm_bindgen_test]
async fn an_element_shows_the_editor_it_names_and_lets_it_go() {
	let note = CanvistState::new(Extensions::paragraphs());
	let other = CanvistState::new(Extensions::paragraphs());
	let parent = parent();
	let screen = move || {
		render(
			View().children([
				CanvistEditor(note.id())
					.accessibility_label("Note")
					.class("w-96 text-base"),
				CanvistEditor(other.id())
					.accessibility_label("Other")
					.class("w-96 text-base"),
			]),
		)
	};
	let mount = mount_to(parent.clone(), screen);
	settle().await;

	let found = containers(&parent);
	assert_eq!(found.len(), 2);
	assert!(found.iter().all(has_canvas), "each element shows an editor");

	// Each element shows its own editor.
	type_text(&field(&found[0]).unwrap(), "first");
	type_text(&field(&found[1]).unwrap(), "second");
	assert_eq!(text(note), "first");
	assert_eq!(text(other), "second");

	// The element leaves: so does everything the editor built.
	drop(mount);
	settle().await;
	assert_eq!(containers(&parent).len(), 0);
	assert!(
		document()
			.query_selector("[data-canvist]")
			.unwrap()
			.is_none(),
		"nothing of the editors is left on the page"
	);

	// The document stayed with the handle, and shows again with the element.
	let mount = mount_to(parent.clone(), screen);
	settle().await;
	let found = containers(&parent);
	assert!(found.iter().all(has_canvas));
	type_text(&field(&found[0]).unwrap(), " again");
	assert!(text(note).contains("first"), "{}", text(note));
	assert!(text(note).contains("again"), "{}", text(note));

	// A disposed editor leaves its element, which stays.
	other.dispose();
	assert!(!has_canvas(&found[1]));
	assert!(has_canvas(&found[0]));

	drop(mount);
	note.dispose();
	parent.remove();
}

#[wasm_bindgen_test]
async fn the_element_s_settings_reach_the_editor_and_follow_it() {
	let note = CanvistState::new(Extensions::paragraphs());
	let parent = parent();
	let step = ArcRwSignal::new(0);
	let shown = step.clone();
	let mount = mount_to(parent.clone(), move || {
		move || {
			let editor = CanvistEditor(note.id()).class("w-96 text-base");

			render(
				View().child(match shown.get() {
					0 => {
						editor
							.accessibility_label("Note")
							.placeholder("Write a note…")
					}
					1 => {
						editor
							.accessibility_label("Locked note")
							.placeholder("Nothing yet")
							.read_only(true)
					}
					_ => editor.accessibility_label("Note"),
				}),
			)
		}
	});
	settle().await;

	let container = containers(&parent).remove(0);
	let input = field(&container).expect("the editor is in its element");
	let attribute = |name: &str| input.get_attribute(name);
	assert_eq!(attribute("aria-label").as_deref(), Some("Note"));
	assert_eq!(
		attribute("aria-placeholder").as_deref(),
		Some("Write a note…")
	);
	assert_eq!(attribute("aria-readonly").as_deref(), Some("false"));
	assert!(!note.is_read_only());

	step.set(1);
	settle().await;
	assert_eq!(
		field(&containers(&parent).remove(0)),
		Some(input.clone()),
		"the editor stays where it is while its settings change"
	);
	assert_eq!(attribute("aria-label").as_deref(), Some("Locked note"));
	assert_eq!(
		attribute("aria-placeholder").as_deref(),
		Some("Nothing yet")
	);
	assert_eq!(attribute("aria-readonly").as_deref(), Some("true"));
	assert!(note.is_read_only());

	step.set(2);
	settle().await;
	assert_eq!(attribute("aria-label").as_deref(), Some("Note"));
	assert_eq!(attribute("aria-placeholder"), None);
	assert_eq!(attribute("aria-readonly").as_deref(), Some("false"));
	assert!(!note.is_read_only());

	drop(mount);
	note.dispose();
	parent.remove();
}

#[wasm_bindgen_test]
async fn an_element_that_sets_nothing_leaves_the_handle_s_settings() {
	let note = CanvistState::new(Extensions::paragraphs());
	note.set_placeholder("From the app");
	note.set_read_only(true);
	let parent = parent();
	let mount = mount_to(parent.clone(), move || {
		render(View().child(CanvistEditor(note.id()).accessibility_label("Note")))
	});
	settle().await;

	let input = field(&containers(&parent).remove(0)).unwrap();
	assert_eq!(
		input.get_attribute("aria-placeholder").as_deref(),
		Some("From the app")
	);
	assert!(note.is_read_only());

	drop(mount);
	note.dispose();
	parent.remove();
}

#[wasm_bindgen_test]
async fn the_editor_takes_its_text_style_from_the_element_s_classes() {
	let note = CanvistState::new(Extensions::paragraphs());
	note.load(Format::Text, "One line").unwrap();
	let parent = parent();
	let large = ArcRwSignal::new(false);
	let shown = large.clone();
	let mount = mount_to(parent.clone(), move || {
		move || {
			let size = if shown.get() {
				"w-96 text-3xl"
			} else {
				"w-96 text-sm"
			};

			render(
				View().child(
					CanvistEditor(note.id())
						.accessibility_label("Note")
						.class(size),
				),
			)
		}
	});
	settle().await;

	let container = containers(&parent).remove(0);
	let canvas: HtmlElement = container
		.query_selector("canvas[data-canvist=canvas]")
		.unwrap()
		.unwrap()
		.dyn_into()
		.unwrap();
	let small = canvas.get_bounding_client_rect().height();
	assert!(small > 0.0);

	large.set(true);
	settle().await;
	let big = canvas.get_bounding_client_rect().height();
	assert!(
		big > small,
		"a line of larger text is taller: {small} then {big}"
	);

	drop(mount);
	note.dispose();
	parent.remove();
}

#[wasm_bindgen_test]
async fn focus_handlers_hear_the_editor_and_focused_gives_it_the_keyboard() {
	let note = CanvistState::new(Extensions::paragraphs());
	let parent = parent();
	let focuses = Arc::new(AtomicUsize::new(0));
	let blurs = Arc::new(AtomicUsize::new(0));
	let (heard_focus, heard_blur) = (Arc::clone(&focuses), Arc::clone(&blurs));
	let mount = mount_to(parent.clone(), move || {
		render(
			View().child(
				CanvistEditor(note.id())
					.accessibility_label("Note")
					.class("w-96 text-base")
					.focused(true)
					.on(NativeEventSlot::Focus, move |_| {
						heard_focus.fetch_add(1, Ordering::SeqCst);
						EventOutcome::Handled
					})
					.on(NativeEventSlot::Blur, move |_| {
						heard_blur.fetch_add(1, Ordering::SeqCst);
						EventOutcome::Handled
					}),
			),
		)
	});
	settle().await;

	let input = field(&containers(&parent).remove(0)).unwrap();
	assert_eq!(
		document().active_element(),
		Some(input.clone().into()),
		"an element that asks for focus gives its editor the keyboard"
	);
	assert_eq!(focuses.load(Ordering::SeqCst), 1);
	assert_eq!(blurs.load(Ordering::SeqCst), 0);

	input.blur().unwrap();
	assert_eq!(blurs.load(Ordering::SeqCst), 1);

	drop(mount);
	note.dispose();
	parent.remove();
}

/// The server's markup names the server's editor, 57. The page's own editor
/// has another id, and it is the one that has to appear.
#[wasm_bindgen_test]
async fn a_server_rendered_element_shows_the_page_s_editor_once_hydrated() {
	let parent = parent();
	parent.set_inner_html(include_str!("fixtures/element_hydration.html"));
	let server_container = containers(&parent).remove(0);
	assert_eq!(
		server_container
			.get_attribute("data-lk-canvist-editor")
			.as_deref(),
		Some("57")
	);

	let note = CanvistState::new(Extensions::paragraphs());
	assert_ne!(note.id(), 57);
	note.load(Format::Text, "Saved").unwrap();
	let mount = hydrate_from(parent.clone(), move || hydration_screen(note.id()));
	settle().await;

	let container = containers(&parent).remove(0);
	assert_eq!(
		container, server_container,
		"hydration keeps the server's element"
	);
	assert!(has_canvas(&container), "the editor is in it");
	let input = field(&container).unwrap();
	assert_eq!(input.get_attribute("aria-label").as_deref(), Some("Note"));
	assert_eq!(
		input.get_attribute("aria-placeholder").as_deref(),
		Some("Write a note…")
	);
	type_text(&input, "!");
	assert!(text(note).contains('!'), "{}", text(note));
	assert!(text(note).contains("Saved"), "{}", text(note));

	drop(mount);
	note.dispose();
	parent.remove();
}

/// The screen the hydration fixture was rendered from, showing `editor`.
fn hydration_screen(editor: u32) -> AnyView {
	render(
		Element::view().child(
			CanvistEditor(editor)
				.accessibility_label("Note")
				.placeholder("Write a note…")
				.class("w-96 text-base"),
		),
	)
}
