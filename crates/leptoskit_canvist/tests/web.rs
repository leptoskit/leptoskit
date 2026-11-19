//! The editor in a real browser: what people do to it, and what they see.
//!
//! The tests drive the editor the way a browser does, with DOM events on its
//! text field and canvas, and read it the way an app and a person can: the
//! exported document, the clipboard, the page's elements, and the canvas's
//! pixels.
#![cfg(all(feature = "web", target_arch = "wasm32"))]

use std::cell::RefCell;
use std::future::Future;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::Mutex;

use leptoskit_canvist::CanvistError;
use leptoskit_canvist::CanvistState;
use leptoskit_canvist::Change;
use leptoskit_canvist::Extensions;
use leptoskit_canvist::Format;
use leptoskit_canvist::web;
use leptoskit_canvist::web::Attachment;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;
use web_sys::CanvasRenderingContext2d;
use web_sys::ClipboardEvent;
use web_sys::ClipboardEventInit;
use web_sys::CompositionEvent;
use web_sys::CompositionEventInit;
use web_sys::DataTransfer;
use web_sys::Element;
use web_sys::Event;
use web_sys::HtmlCanvasElement;
use web_sys::HtmlElement;
use web_sys::HtmlTextAreaElement;
use web_sys::InputEvent;
use web_sys::InputEventInit;
use web_sys::KeyboardEvent;
use web_sys::KeyboardEventInit;
use web_sys::PointerEvent;
use web_sys::PointerEventInit;

wasm_bindgen_test_configure!(run_in_browser);

/// A container 400 px wide with black 16 px text on white, 24 px to a line.
/// Paragraphs are 12 px apart, with no space above the first or below the
/// last, so the first line spans 0 to 24 px and the second paragraph starts
/// at 36 px.
const PLAIN: &str =
	"width:400px;font:16px/1.5 monospace;color:rgb(0, 0, 0);background-color:rgb(255, 255, 255)";

/// The middle of the first line, and of the second paragraph's first line.
const FIRST_LINE: i32 = 12;
const SECOND_PARAGRAPH: i32 = 48;

const WHITE: [u8; 3] = [255, 255, 255];

fn document() -> web_sys::Document {
	web_sys::window().unwrap().document().unwrap()
}

/// An editor on the page, taken off it again when the test ends.
struct Fixture {
	container: HtmlElement,
	state: CanvistState,
	attachment: Option<Attachment>,
}

impl Fixture {
	fn new(style: &str) -> Self {
		let container = container(style);
		let state = CanvistState::new(Extensions::paragraphs());
		let attachment = web::attach(&container, state).expect("a browser with a canvas");

		Self {
			container,
			state,
			attachment: Some(attachment),
		}
	}

	fn with_text(style: &str, text: &str) -> Self {
		let fixture = Self::new(style);
		fixture.state.load(Format::Text, text).expect("plain text");
		fixture.render();
		fixture
	}

	fn render(&self) {
		self.attachment.as_ref().expect("still attached").render();
	}

	fn part<T: JsCast>(&self, name: &str) -> T {
		self.container
			.query_selector(&format!("[data-canvist={name}]"))
			.unwrap()
			.unwrap_or_else(|| panic!("the editor has a {name}"))
			.dyn_into()
			.unwrap()
	}

	fn field(&self) -> HtmlTextAreaElement {
		self.part("input")
	}

	fn canvas(&self) -> HtmlCanvasElement {
		self.part("canvas")
	}

	fn text(&self) -> String {
		now(self.state.export(Format::Text)).expect("a live editor")
	}

	/// Press `key` on the text field. Returns whether the editor handled
	/// it, which it shows by preventing the browser's own handling.
	fn key(&self, key: &str, modifiers: Modifiers) -> bool {
		let init = KeyboardEventInit::new();
		init.set_key(key);
		init.set_bubbles(true);
		init.set_cancelable(true);
		init.set_shift_key(modifiers.shift);
		init.set_ctrl_key(modifiers.command && !apple());
		init.set_meta_key(modifiers.command && apple());
		let event = KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();

		!self.field().dispatch_event(&event).unwrap()
	}

	/// Send text input as the browser does before changing a text field.
	/// Returns whether the editor took it over.
	fn input(&self, kind: &str, data: Option<&str>) -> bool {
		let init = InputEventInit::new();
		init.set_input_type(kind);
		init.set_data(data);
		init.set_bubbles(true);
		init.set_cancelable(true);
		let event = InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();

		!self.field().dispatch_event(&event).unwrap()
	}

	fn type_text(&self, text: &str) {
		assert!(self.input("insertText", Some(text)), "typing is taken over");
	}

	fn compose(&self, kind: &str, data: &str) {
		let init = CompositionEventInit::new();
		init.set_data(data);
		init.set_bubbles(true);
		let event = CompositionEvent::new_with_event_init_dict(kind, &init).unwrap();
		self.field().dispatch_event(&event).unwrap();
	}

	/// Send a clipboard event carrying `data`. Returns the event's plain
	/// text when the editor handled it.
	fn clipboard(&self, kind: &str, data: Option<&str>) -> Option<String> {
		let transfer = DataTransfer::new().unwrap();

		if let Some(data) = data {
			transfer.set_data("text/plain", data).unwrap();
		}

		let init = ClipboardEventInit::new();
		init.set_clipboard_data(Some(&transfer));
		init.set_bubbles(true);
		init.set_cancelable(true);
		let event = ClipboardEvent::new_with_event_init_dict(kind, &init).unwrap();
		let handled = !self.field().dispatch_event(&event).unwrap();

		handled.then(|| transfer.get_data("text/plain").unwrap())
	}

	fn copy(&self) -> Option<String> {
		self.clipboard("copy", None)
	}

	/// Send a pointer event at `(x, y)` from the canvas's top left corner.
	fn pointer(&self, kind: &str, (x, y): (i32, i32), shift: bool) {
		let canvas = self.canvas();
		let bounds = canvas.get_bounding_client_rect();
		let init = PointerEventInit::new();
		init.set_client_x(bounds.left() as i32 + x);
		init.set_client_y(bounds.top() as i32 + y);
		init.set_button(0);
		init.set_pointer_id(1);
		init.set_shift_key(shift);
		init.set_bubbles(true);
		init.set_cancelable(true);
		let event = PointerEvent::new_with_event_init_dict(kind, &init).unwrap();
		canvas.dispatch_event(&event).unwrap();
	}

	fn click(&self, at: (i32, i32)) {
		self.pointer("pointerdown", at, false);
		self.pointer("pointerup", at, false);
	}

	/// The canvas's pixels, as `[r, g, b]` each.
	fn pixels(&self) -> Vec<[u8; 3]> {
		let canvas = self.canvas();
		let context: CanvasRenderingContext2d = canvas
			.get_context("2d")
			.unwrap()
			.unwrap()
			.dyn_into()
			.unwrap();
		let (width, height) = (f64::from(canvas.width()), f64::from(canvas.height()));

		if width * height == 0.0 {
			return Vec::new();
		}

		context
			.get_image_data(0.0, 0.0, width, height)
			.unwrap()
			.data()
			.as_chunks::<4>()
			.0
			.iter()
			.map(|pixel| [pixel[0], pixel[1], pixel[2]])
			.collect()
	}

	/// How many pixels of the canvas are not `background`: text, the caret,
	/// the selection.
	fn ink(&self, background: [u8; 3]) -> usize {
		self.pixels()
			.iter()
			.filter(|pixel| **pixel != background)
			.count()
	}

	/// Record every change the editor reports.
	fn changes(&self) -> Arc<Mutex<Vec<Change>>> {
		let changes = Arc::new(Mutex::new(Vec::new()));
		self.state.on_change({
			let changes = Arc::clone(&changes);
			move |change| changes.lock().unwrap().push(change)
		});
		changes
	}
}

impl Drop for Fixture {
	fn drop(&mut self) {
		self.attachment.take();
		self.state.dispose();
		self.container.remove();
	}
}

#[derive(Clone, Copy, Default)]
struct Modifiers {
	shift: bool,
	/// Cmd on Apple platforms and Ctrl elsewhere.
	command: bool,
}

const NONE: Modifiers = Modifiers {
	shift: false,
	command: false,
};
const SHIFT: Modifiers = Modifiers {
	shift: true,
	command: false,
};
const COMMAND: Modifiers = Modifiers {
	shift: false,
	command: true,
};

fn apple() -> bool {
	let platform = web_sys::window().unwrap().navigator().platform().unwrap();
	["Mac", "iPhone", "iPad"]
		.iter()
		.any(|name| platform.contains(name))
}

fn container(style: &str) -> HtmlElement {
	let container: HtmlElement = document()
		.create_element("div")
		.unwrap()
		.dyn_into()
		.unwrap();
	container.style().set_css_text(style);
	document().body().unwrap().append_child(&container).unwrap();
	container
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

/// Wait for the browser to run `count` animation frames, which is when the
/// editor paints and when resize observers report.
async fn frames(count: usize) {
	for _ in 0..count {
		let promise = js_sys::Promise::new(&mut |resolve, _| {
			web_sys::window()
				.unwrap()
				.request_animation_frame(&resolve)
				.unwrap();
		});
		JsFuture::from(promise).await.unwrap();
	}
}

fn pixels_of(style: &web_sys::CssStyleDeclaration, name: &str) -> f64 {
	style
		.get_property_value(name)
		.unwrap()
		.trim_end_matches("px")
		.parse()
		.unwrap_or_else(|_| panic!("{name} is a length in px"))
}

#[wasm_bindgen_test]
fn attaching_builds_the_editor_and_detaching_leaves_nothing() {
	let mut fixture = Fixture::new(PLAIN);
	let field = fixture.field();

	assert_eq!(fixture.container.child_element_count(), 4);
	assert_eq!(field.get_attribute("role").as_deref(), Some("textbox"));
	assert_eq!(
		field.get_attribute("aria-multiline").as_deref(),
		Some("true")
	);
	assert_eq!(field.get_attribute("aria-label").as_deref(), Some("Editor"));
	assert_eq!(
		field.get_attribute("aria-readonly").as_deref(),
		Some("false")
	);
	assert_eq!(field.get_attribute("aria-placeholder"), None);
	assert_eq!(
		fixture
			.part::<Element>("mirror")
			.get_attribute("role")
			.as_deref(),
		Some("document")
	);
	assert_eq!(
		fixture
			.container
			.style()
			.get_property_value("position")
			.unwrap(),
		"relative",
		"the text field is placed against the container"
	);

	fixture.type_text("kept");
	fixture.attachment.take();

	assert_eq!(fixture.container.child_element_count(), 0);
	assert_eq!(
		fixture
			.container
			.style()
			.get_property_value("position")
			.unwrap(),
		"",
		"the container is left as it was found"
	);
	assert_eq!(
		fixture.text(),
		"kept",
		"the document outlives the attachment"
	);

	// Events that reach the page after the editor left do nothing.
	let init = KeyboardEventInit::new();
	init.set_key("Enter");
	init.set_bubbles(true);
	field
		.dispatch_event(
			&KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap(),
		)
		.unwrap();
	assert_eq!(fixture.text(), "kept");
}

#[wasm_bindgen_test]
fn the_container_names_the_editor() {
	let container = container(PLAIN);
	container
		.set_attribute("aria-label", "Meeting notes")
		.unwrap();
	// Already positioned, so the editor has nothing to change or restore.
	container
		.style()
		.set_property("position", "absolute")
		.unwrap();
	let state = CanvistState::new(Extensions::paragraphs());
	let attachment = web::attach(&container, state).unwrap();
	let label = |name: &str| {
		container
			.query_selector(&format!("[data-canvist={name}]"))
			.unwrap()
			.unwrap()
			.get_attribute("aria-label")
	};

	assert_eq!(label("input").as_deref(), Some("Meeting notes"));
	assert_eq!(label("mirror").as_deref(), Some("Meeting notes"));

	drop(attachment);

	assert_eq!(
		container.style().get_property_value("position").unwrap(),
		"absolute"
	);
	state.dispose();
	container.remove();
}

#[wasm_bindgen_test]
fn a_disposed_editor_cannot_be_attached() {
	let container = container(PLAIN);
	let state = CanvistState::new(Extensions::paragraphs());
	state.dispose();

	assert_eq!(
		web::attach(&container, state).map(|_| ()),
		Err(CanvistError::Disposed)
	);
	assert_eq!(container.child_element_count(), 0);
	container.remove();
}

#[wasm_bindgen_test]
fn typing_changes_the_document_and_the_canvas() {
	let fixture = Fixture::new(PLAIN);
	let changes = fixture.changes();

	assert_eq!(
		fixture.ink(WHITE),
		0,
		"an empty editor without focus is blank"
	);

	fixture.type_text("Hello");

	assert_eq!(fixture.text(), "Hello");
	assert!(
		fixture.ink(WHITE) > 0,
		"the text is painted in the same task"
	);
	assert_eq!(
		*changes.lock().unwrap(),
		[Change {
			revision: 1,
			doc_changed: true,
			selection_changed: true
		}]
	);
	assert_eq!(fixture.field().value(), "", "the text field holds no text");
}

#[wasm_bindgen_test]
fn enter_and_backspace_split_and_join_paragraphs() {
	let fixture = Fixture::new(PLAIN);
	fixture.type_text("ab");

	assert!(fixture.key("Enter", NONE));
	fixture.type_text("cd");
	assert_eq!(fixture.text(), "ab\ncd");

	assert!(fixture.key("Backspace", NONE));
	assert!(fixture.key("Backspace", NONE));
	assert_eq!(fixture.text(), "ab\n");

	assert!(fixture.key("Backspace", NONE));
	assert_eq!(fixture.text(), "ab", "the paragraphs joined");

	assert!(fixture.key("Enter", SHIFT));
	fixture.type_text("c");
	assert_eq!(
		now(fixture.state.export(Format::Markdown)).unwrap(),
		"ab\\\nc",
		"Shift+Enter breaks the line inside one paragraph"
	);
}

#[wasm_bindgen_test]
fn soft_keyboards_edit_without_key_events() {
	let fixture = Fixture::new(PLAIN);
	fixture.type_text("ab");

	assert!(fixture.input("insertParagraph", None));
	fixture.type_text("cd");
	assert!(fixture.input("deleteContentBackward", None));
	assert_eq!(fixture.text(), "ab\nc");

	assert!(fixture.input("historyUndo", None));
	assert_ne!(fixture.text(), "ab\nc");
	assert!(fixture.input("historyRedo", None));
	assert_eq!(fixture.text(), "ab\nc");

	fixture.key("ArrowLeft", NONE);
	assert!(fixture.input("deleteContentForward", None));
	assert_eq!(fixture.text(), "ab\n");
}

#[wasm_bindgen_test]
fn undo_and_redo_walk_the_history() {
	let fixture = Fixture::new(PLAIN);
	fixture.type_text("one");
	fixture.key("Enter", NONE);
	fixture.type_text("two");

	assert!(fixture.key("z", COMMAND));
	assert_ne!(fixture.text(), "one\ntwo", "one undo takes something back");

	for _ in 0..8 {
		fixture.key("z", COMMAND);
	}

	assert_eq!(fixture.text(), "", "undo reaches the empty document");
	assert!(
		!fixture.key("z", COMMAND),
		"and then has nothing left to do"
	);

	for _ in 0..8 {
		fixture.key(
			"z",
			Modifiers {
				shift: true,
				command: true,
			},
		);
	}

	assert_eq!(fixture.text(), "one\ntwo", "redo brings all of it back");
}

#[wasm_bindgen_test]
fn the_pointer_places_the_caret_and_selects() {
	let fixture = Fixture::with_text(PLAIN, "hello world\nsecond paragraph");
	let changes = fixture.changes();

	fixture.click((20, FIRST_LINE));

	assert_eq!(fixture.copy(), None, "a caret selects nothing");
	assert_eq!(
		changes
			.lock()
			.unwrap()
			.last()
			.map(|change| change.doc_changed),
		Some(false),
		"placing the caret changes only the selection"
	);
	assert_eq!(
		document().active_element(),
		Some(fixture.field().into()),
		"a press moves keyboard focus to the editor"
	);

	// Three presses in one place: the second selects the word, the third
	// the paragraph.
	fixture.click((20, FIRST_LINE));
	assert_eq!(fixture.copy().as_deref(), Some("hello"));
	fixture.click((20, FIRST_LINE));
	assert_eq!(fixture.copy().as_deref(), Some("hello world"));
}

#[wasm_bindgen_test]
fn dragging_selects_across_paragraphs() {
	let fixture = Fixture::with_text(PLAIN, "hello world\nsecond paragraph");

	fixture.pointer("pointerdown", (0, FIRST_LINE), false);
	fixture.pointer("pointermove", (390, SECOND_PARAGRAPH), false);

	assert_eq!(
		fixture.copy().as_deref(),
		Some("hello world\nsecond paragraph")
	);

	fixture.pointer("pointerup", (390, SECOND_PARAGRAPH), false);
	fixture.pointer("pointermove", (0, FIRST_LINE), false);

	assert_eq!(
		fixture.copy().as_deref(),
		Some("hello world\nsecond paragraph"),
		"moving a released pointer selects nothing more"
	);

	// A press far from the first is a new click, and Shift extends from it.
	fixture.pointer("pointerdown", (390, FIRST_LINE), false);
	fixture.pointer("pointerup", (390, FIRST_LINE), false);
	fixture.pointer("pointerdown", (0, FIRST_LINE), true);
	fixture.pointer("pointerup", (0, FIRST_LINE), true);

	assert_eq!(fixture.copy().as_deref(), Some("hello world"));
}

#[wasm_bindgen_test]
fn the_keyboard_selects() {
	let fixture = Fixture::with_text(PLAIN, "hello world\nsecond paragraph");
	fixture.click((0, FIRST_LINE));

	for _ in 0..5 {
		assert!(fixture.key("ArrowRight", SHIFT));
	}

	assert_eq!(fixture.copy().as_deref(), Some("hello"));

	assert!(fixture.key("a", COMMAND));
	assert_eq!(
		fixture.copy().as_deref(),
		Some("hello world\nsecond paragraph")
	);

	// A plain arrow key collapses the selection to the end it points at.
	// Then down a line and to its end, answered by the editor's own layout.
	assert!(fixture.key("ArrowLeft", NONE));
	assert_eq!(fixture.copy(), None);
	assert!(fixture.key("ArrowDown", NONE));
	assert!(fixture.key("End", SHIFT));
	assert_eq!(fixture.copy().as_deref(), Some("second paragraph"));
}

#[wasm_bindgen_test]
fn cut_and_paste_move_plain_text() {
	let fixture = Fixture::with_text(PLAIN, "hello world");
	fixture.key("a", COMMAND);

	assert_eq!(
		fixture.clipboard("cut", None).as_deref(),
		Some("hello world")
	);
	assert_eq!(fixture.text(), "");
	assert_eq!(
		fixture.clipboard("cut", None),
		None,
		"with nothing selected there is nothing to cut"
	);

	assert_eq!(
		fixture.clipboard("paste", Some("one\ntwo")).as_deref(),
		Some("one\ntwo"),
		"the paste is taken over"
	);
	assert_eq!(fixture.text(), "one\ntwo", "each line is a paragraph");

	fixture.key("a", COMMAND);
	fixture.clipboard("paste", Some("replaced"));
	assert_eq!(fixture.text(), "replaced", "a paste replaces the selection");
}

#[wasm_bindgen_test]
fn read_only_refuses_edits_but_still_selects_and_copies() {
	let fixture = Fixture::with_text(PLAIN, "keep me");
	fixture.state.set_read_only(true);
	let field = fixture.field();

	assert!(field.read_only());
	assert_eq!(
		field.get_attribute("aria-readonly").as_deref(),
		Some("true")
	);

	assert!(
		fixture.input("insertText", Some("x")),
		"typing is swallowed"
	);
	assert!(
		fixture.key("Backspace", NONE),
		"so Backspace is no Back button"
	);
	assert!(!fixture.key("Enter", NONE));
	fixture.clipboard("paste", Some("pasted"));
	assert_eq!(fixture.text(), "keep me");

	assert!(fixture.key("a", COMMAND), "select all still works");
	assert_eq!(fixture.copy().as_deref(), Some("keep me"));
	assert_eq!(
		fixture.clipboard("cut", None).as_deref(),
		Some("keep me"),
		"cutting from a read-only editor copies"
	);
	assert_eq!(fixture.text(), "keep me");

	fixture.click((0, FIRST_LINE));
	assert!(
		fixture.key("ArrowRight", SHIFT),
		"the arrow keys still select"
	);
	assert_eq!(fixture.copy().as_deref(), Some("k"));

	// Read-only stops people, not the app.
	fixture.state.load(Format::Text, "set by the app").unwrap();
	assert_eq!(fixture.text(), "set by the app");

	fixture.state.set_read_only(false);
	assert!(!field.read_only());
	fixture.key("a", COMMAND);
	fixture.type_text("free");
	assert_eq!(fixture.text(), "free");
}

#[wasm_bindgen_test]
fn the_placeholder_shows_while_the_document_is_empty() {
	let fixture = Fixture::new(PLAIN);
	fixture.render();

	assert_eq!(fixture.ink(WHITE), 0);

	fixture.state.set_placeholder("Write a note…");
	fixture.render();
	let placeholder = fixture.ink(WHITE);

	assert!(placeholder > 0, "the placeholder is painted");
	assert_eq!(
		fixture.field().get_attribute("aria-placeholder").as_deref(),
		Some("Write a note…")
	);
	assert!(
		fixture.pixels().iter().all(|pixel| pixel[0] > 90),
		"and dimmed: nothing is as dark as the text colour"
	);

	fixture.type_text("W");
	assert!(
		fixture.pixels().iter().any(|pixel| pixel[0] < 60),
		"typed text is drawn in the full text colour"
	);

	fixture.key("Backspace", NONE);
	assert_eq!(
		fixture.ink(WHITE),
		placeholder,
		"it comes back when the text is gone"
	);

	fixture.state.set_placeholder("");
	fixture.render();
	assert_eq!(fixture.ink(WHITE), 0);
	assert_eq!(fixture.field().get_attribute("aria-placeholder"), None);
}

#[wasm_bindgen_test]
fn an_input_method_commits_its_text_once() {
	let fixture = Fixture::new(PLAIN);
	fixture.type_text("a");
	let changes = fixture.changes();

	fixture.compose("compositionstart", "");
	fixture.compose("compositionupdate", "に");
	assert_eq!(fixture.text(), "aに", "unfinished text shows in place");

	// Keys and text input during a composition belong to the input method.
	assert!(!fixture.key("Enter", NONE));
	assert!(!fixture.input("insertCompositionText", Some("にほ")));

	fixture.compose("compositionupdate", "にほん");
	assert_eq!(fixture.text(), "aにほん");

	fixture.compose("compositionend", "日本");
	assert_eq!(
		fixture.text(),
		"a日本",
		"the final text replaces the unfinished"
	);
	assert!(
		changes
			.lock()
			.unwrap()
			.iter()
			.all(|change| change.doc_changed)
	);

	fixture.type_text("!");
	assert_eq!(fixture.text(), "a日本!", "typing carries on after it");

	assert!(fixture.key("z", COMMAND));
	assert!(
		!fixture.text().contains('に'),
		"undo never brings unfinished text back"
	);
}

#[wasm_bindgen_test]
fn an_ancestor_that_prevents_a_key_stops_the_editor() {
	let fixture = Fixture::new(PLAIN);
	fixture.type_text("ab");
	let seen = Rc::new(RefCell::new(Vec::new()));
	let handler = Closure::<dyn FnMut(Event)>::new({
		let seen = Rc::clone(&seen);
		move |event: Event| {
			let key = event.dyn_ref::<KeyboardEvent>().unwrap().key();

			if key == "Enter" {
				event.prevent_default();
			}

			seen.borrow_mut().push(key);
		}
	});
	fixture
		.container
		.add_event_listener_with_callback("keydown", handler.as_ref().unchecked_ref())
		.unwrap();

	assert!(fixture.key("Enter", NONE), "the ancestor prevented it");
	assert_eq!(fixture.text(), "ab", "so the editor left it alone");

	assert!(fixture.key("Backspace", NONE));
	assert_eq!(fixture.text(), "a");
	assert_eq!(
		*seen.borrow(),
		["Enter", "Backspace"],
		"the ancestor heard both keys before the editor acted"
	);
}

#[wasm_bindgen_test]
fn keys_meant_for_other_fields_are_left_alone() {
	let fixture = Fixture::with_text(PLAIN, "text");
	let other: HtmlElement = document()
		.create_element("input")
		.unwrap()
		.dyn_into()
		.unwrap();
	document().body().unwrap().append_child(&other).unwrap();
	let init = KeyboardEventInit::new();
	init.set_key("Backspace");
	init.set_bubbles(true);
	init.set_cancelable(true);
	let event = KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();

	assert!(
		other.dispatch_event(&event).unwrap(),
		"nothing prevented it"
	);
	assert_eq!(fixture.text(), "text");
	other.remove();
}

#[wasm_bindgen_test]
fn focus_shows_the_caret_and_blur_hides_it() {
	let fixture = Fixture::new(PLAIN);
	fixture.state.focus();
	fixture.render();

	assert_eq!(document().active_element(), Some(fixture.field().into()));
	assert!(fixture.ink(WHITE) > 0, "the caret is drawn");

	// The text field follows the caret, so input methods open beside it.
	let style = fixture.field().style();
	assert!(
		(-2.0..=0.0).contains(&pixels_of(&style, "left")),
		"{}",
		style.css_text()
	);
	assert!(
		(0.0..=8.0).contains(&pixels_of(&style, "top")),
		"{}",
		style.css_text()
	);

	fixture.state.set_read_only(true);
	fixture.render();
	assert_eq!(fixture.ink(WHITE), 0, "a read-only editor shows no caret");

	fixture.state.set_read_only(false);
	fixture.render();
	assert!(fixture.ink(WHITE) > 0);

	fixture.field().blur().unwrap();
	fixture.render();
	assert_eq!(fixture.ink(WHITE), 0, "without focus there is no caret");
}

#[wasm_bindgen_test]
fn the_theme_comes_from_the_container() {
	let fixture = Fixture::new(
		"width:400px;font:italic 700 20px/2 serif;color:rgb(255, 0, 0);background-color:rgb(0, 0, \
		 64)",
	);
	fixture.state.load(Format::Text, "Hello\nWorld").unwrap();
	fixture.render();
	let pixels = fixture.pixels();

	assert_eq!(
		pixels.last(),
		Some(&[0, 0, 64]),
		"the background fills the canvas"
	);
	assert!(
		pixels.iter().any(|pixel| pixel[0] > 200 && pixel[2] < 40),
		"the text is in the container's colour"
	);
	// 20 px text on 40 px lines, and 15 px between the two paragraphs.
	assert_eq!(
		fixture
			.canvas()
			.style()
			.get_property_value("height")
			.unwrap(),
		"95px"
	);

	let style = fixture.container.style();
	style
		.set_property("background-color", "oklch(1 0 0)")
		.unwrap();
	style.set_property("color", "rgb(0, 0, 0)").unwrap();
	style.set_property("font-size", "12px").unwrap();
	fixture.attachment.as_ref().unwrap().refresh_theme();
	fixture.render();

	assert_eq!(
		fixture.pixels().last(),
		Some(&WHITE),
		"a colour computed styles keep as written is still read"
	);
	assert_eq!(
		fixture
			.canvas()
			.style()
			.get_property_value("height")
			.unwrap(),
		"57px"
	);
}

#[wasm_bindgen_test]
fn a_see_through_container_takes_the_colour_behind_it() {
	let outer = container("background-color:rgb(0, 128, 0);width:420px");
	let inner: HtmlElement = document()
		.create_element("div")
		.unwrap()
		.dyn_into()
		.unwrap();
	inner.style().set_css_text(
		"width:400px;font:16px/1.5 monospace;background-color:rgba(255, 255, 255, 0.5)",
	);
	outer.append_child(&inner).unwrap();
	let state = CanvistState::new(Extensions::paragraphs());
	let attachment = web::attach(&inner, state).unwrap();
	attachment.render();
	let canvas: HtmlCanvasElement = inner
		.query_selector("canvas")
		.unwrap()
		.unwrap()
		.dyn_into()
		.unwrap();
	let context: CanvasRenderingContext2d = canvas
		.get_context("2d")
		.unwrap()
		.unwrap()
		.dyn_into()
		.unwrap();
	let pixel = context.get_image_data(0.0, 0.0, 1.0, 1.0).unwrap().data();

	// Half white over the green behind it.
	assert_eq!(&pixel[..3], [128, 192, 128]);

	drop(attachment);
	state.dispose();
	outer.remove();
}

#[wasm_bindgen_test]
async fn resizing_the_container_wraps_the_text_again() {
	let fixture = Fixture::with_text(
		PLAIN,
		"a sentence long enough to need more than one line once the editor gets narrow",
	);
	let height = |fixture: &Fixture| {
		pixels_of(&fixture.canvas().style(), "height")
			+ pixels_of(&fixture.part::<HtmlElement>("spacer").style(), "height")
	};
	let wide = height(&fixture);

	assert!(wide >= 48.0, "{wide}");

	fixture
		.container
		.style()
		.set_property("width", "120px")
		.unwrap();
	frames(3).await;

	assert!(height(&fixture) > wide, "narrower text takes more lines");
	assert_eq!(fixture.canvas().get_bounding_client_rect().width(), 120.0);
}

#[wasm_bindgen_test]
async fn changes_from_the_app_paint_on_the_next_frame() {
	let fixture = Fixture::new(PLAIN);
	frames(1).await;

	assert_eq!(fixture.ink(WHITE), 0);

	fixture.state.load(Format::Text, "loaded").unwrap();
	frames(2).await;

	assert!(fixture.ink(WHITE) > 0, "no one had to ask for a repaint");

	let mirror: Element = fixture.part("mirror");
	fixture
		.state
		.load(Format::Markdown, "first\\\nline\n\nsecond")
		.unwrap();
	// The hidden copy for screen readers catches up when the browser is idle.
	for _ in 0..60 {
		if mirror.child_element_count() == 2 {
			break;
		}

		frames(1).await;
	}

	assert_eq!(mirror.inner_html(), "<p>first<br>line</p><p>second</p>");
}

#[wasm_bindgen_test]
fn a_scrolling_container_shows_a_window_onto_the_document() {
	let lines: Vec<String> = (1..=40).map(|line| format!("line {line}")).collect();
	let fixture = Fixture::with_text(
		&format!("{PLAIN};height:120px;overflow:auto;padding:10px;border:3px solid"),
		&lines.join("\n"),
	);
	let spacer: HtmlElement = fixture.part("spacer");

	assert_eq!(
		pixels_of(&fixture.canvas().style(), "height"),
		140.0,
		"the canvas is as tall as the container's padding box"
	);
	assert!(
		pixels_of(&spacer.style(), "height") > 1000.0,
		"the rest is space"
	);
	assert_eq!(fixture.container.scroll_top(), 0);

	// The canvas starts below the padding, where the first line is.
	fixture.click((0, FIRST_LINE));
	assert!(fixture.key("End", SHIFT));
	assert_eq!(fixture.copy().as_deref(), Some("line 1"));

	// The text field sits at the caret, inside the padding.
	let style = fixture.field().style();
	assert!(pixels_of(&style, "left") > 40.0, "{}", style.css_text());
	assert!(
		(10.0..=18.0).contains(&pixels_of(&style, "top")),
		"{}",
		style.css_text()
	);

	for _ in 0..39 {
		fixture.key("ArrowDown", NONE);
	}

	assert!(
		fixture.container.scroll_top() > 1000,
		"moving the caret scrolls it into view"
	);
	fixture.key("Home", NONE);
	fixture.key("End", SHIFT);
	assert_eq!(fixture.copy().as_deref(), Some("line 40"));
}

#[wasm_bindgen_test]
fn a_listener_may_dispose_the_editor_it_hears_from() {
	let container = container(PLAIN);
	let state = CanvistState::new(Extensions::paragraphs());
	let attachment = web::attach(&container, state).unwrap();
	state.on_change(move |_| state.dispose());
	let field: HtmlTextAreaElement = container
		.query_selector("textarea")
		.unwrap()
		.unwrap()
		.dyn_into()
		.unwrap();
	let init = InputEventInit::new();
	init.set_input_type("insertText");
	init.set_data(Some("x"));
	init.set_cancelable(true);
	field
		.dispatch_event(&InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap())
		.unwrap();

	assert_eq!(container.child_element_count(), 0, "disposing detaches");
	assert_eq!(now(state.export(Format::Text)), Err(CanvistError::Disposed));

	// The attachment outlived its editor; using and dropping it is harmless.
	attachment.render();
	drop(attachment);
	container.remove();
}

#[wasm_bindgen_test]
fn attaching_again_moves_the_editor() {
	let (first, second) = (container(PLAIN), container(PLAIN));
	let state = CanvistState::new(Extensions::paragraphs());
	let old = web::attach(&first, state).unwrap();
	state.load(Format::Text, "moved").unwrap();
	let new = web::attach(&second, state).unwrap();

	assert_eq!(first.child_element_count(), 0);
	assert_eq!(second.child_element_count(), 4);

	// The first attachment no longer owns the editor.
	drop(old);
	assert_eq!(second.child_element_count(), 4);

	new.render();
	let canvas: HtmlCanvasElement = second
		.query_selector("canvas")
		.unwrap()
		.unwrap()
		.dyn_into()
		.unwrap();
	assert!(canvas.height() > 0, "the document came along");

	drop(new);
	state.dispose();
	first.remove();
	second.remove();
}
