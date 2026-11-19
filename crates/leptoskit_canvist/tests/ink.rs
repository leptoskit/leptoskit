//! Drawings in a real browser: what the pointer draws, what the document
//! keeps, and what the canvas shows.
//!
//! Like the other browser tests, these send the DOM events a browser sends
//! and read back what an app and a person can: the exported document, the
//! page's elements, and the canvas's pixels.
#![cfg(all(feature = "web", feature = "ink", target_arch = "wasm32"))]

use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;

use leptoskit_canvist::CanvistState;
use leptoskit_canvist::Change;
use leptoskit_canvist::Extensions;
use leptoskit_canvist::Format;
use leptoskit_canvist::Pen;
use leptoskit_canvist::web;
use leptoskit_canvist::web::Attachment;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;
use web_sys::CanvasRenderingContext2d;
use web_sys::Element;
use web_sys::Event;
use web_sys::EventInit;
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

/// A container 400 px wide with black 16 px text on white. A drawing is
/// 1000 units across, so here a unit is 0.4 px, and a new drawing, 400
/// units tall, is 160 px.
const PLAIN: &str =
	"width:400px;font:16px/1.5 monospace;color:rgb(0, 0, 0);background-color:rgb(255, 255, 255)";

/// The paper behind a drawing: the text colour at a twentieth over white.
const PAPER: [u8; 3] = [242, 242, 242];

/// A pen no text or paper is the colour of.
const ROSE: [u8; 3] = [0xe1, 0x1d, 0x48];

fn document() -> web_sys::Document {
	web_sys::window().unwrap().document().unwrap()
}

/// Where a drawing is on the canvas, in CSS pixels from its top left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Sheet {
	left: i32,
	top: i32,
	width: i32,
	height: i32,
}

impl Sheet {
	/// A point `(x, y)` pixels in from the drawing's top left.
	fn at(self, x: i32, y: i32) -> (i32, i32) {
		(self.left + x, self.top + y)
	}
}

/// An editor that holds drawings, taken off the page when the test ends.
struct Fixture {
	container: HtmlElement,
	state: CanvistState,
	attachment: Option<Attachment>,
}

impl Fixture {
	/// An editor holding "above", a drawing, and "below".
	fn new(style: &str) -> Self {
		let container: HtmlElement = document()
			.create_element("div")
			.unwrap()
			.dyn_into()
			.unwrap();
		container.style().set_css_text(style);
		document().body().unwrap().append_child(&container).unwrap();
		let state = CanvistState::new(Extensions::paragraphs().ink());
		let attachment = web::attach(&container, state).expect("a browser with a canvas");
		let fixture = Self {
			container,
			state,
			attachment: Some(attachment),
		};
		fixture.type_text("above");
		assert_eq!(fixture.state.insert_drawing(), Ok(true));
		fixture.type_text("below");
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

	fn json(&self) -> String {
		now(self.state.export(Format::Json)).expect("a live editor")
	}

	/// How many strokes the document holds.
	fn strokes(&self) -> usize {
		self.json().matches(r#""points""#).count()
	}

	fn type_text(&self, text: &str) {
		let init = InputEventInit::new();
		init.set_input_type("insertText");
		init.set_data(Some(text));
		init.set_bubbles(true);
		init.set_cancelable(true);
		let event = InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();
		self.field().dispatch_event(&event).unwrap();
	}

	/// Press Ctrl+Z, or Cmd+Z on Apple platforms. Returns whether the
	/// editor handled it.
	fn undo_key(&self) -> bool {
		let platform = web_sys::window().unwrap().navigator().platform().unwrap();
		let apple = ["Mac", "iPhone", "iPad"]
			.iter()
			.any(|name| platform.contains(name));
		let init = KeyboardEventInit::new();
		init.set_key("z");
		init.set_bubbles(true);
		init.set_cancelable(true);
		init.set_ctrl_key(!apple);
		init.set_meta_key(apple);
		let event = KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();

		!self.field().dispatch_event(&event).unwrap()
	}

	/// Send a pointer event from a `device` ("mouse", "pen" or "touch") at
	/// `(x, y)` from the canvas's top left corner.
	fn pointer(&self, kind: &str, (x, y): (i32, i32), device: &str) {
		let canvas = self.canvas();
		let bounds = canvas.get_bounding_client_rect();
		let init = PointerEventInit::new();
		init.set_client_x(bounds.left() as i32 + x);
		init.set_client_y(bounds.top() as i32 + y);
		init.set_button(0);
		init.set_pointer_id(1);
		init.set_pointer_type(device);
		init.set_bubbles(true);
		init.set_cancelable(true);
		let event = PointerEvent::new_with_event_init_dict(kind, &init).unwrap();
		canvas.dispatch_event(&event).unwrap();
	}

	/// Press at the first of `points`, move through the rest, and release.
	fn draw(&self, points: &[(i32, i32)]) {
		let (first, rest) = points.split_first().expect("somewhere to press");
		self.pointer("pointerdown", *first, "mouse");

		for point in rest {
			self.pointer("pointermove", *point, "mouse");
		}

		self.pointer("pointerup", *points.last().unwrap(), "mouse");
	}

	/// The canvas's pixels as `[r, g, b]` each, in rows of the returned
	/// width.
	fn pixels(&self) -> (Vec<[u8; 3]>, usize) {
		let canvas = self.canvas();
		let context: CanvasRenderingContext2d = canvas
			.get_context("2d")
			.unwrap()
			.unwrap()
			.dyn_into()
			.unwrap();
		let (width, height) = (f64::from(canvas.width()), f64::from(canvas.height()));
		let pixels = context
			.get_image_data(0.0, 0.0, width, height)
			.unwrap()
			.data()
			.as_chunks::<4>()
			.0
			.iter()
			.map(|pixel| [pixel[0], pixel[1], pixel[2]])
			.collect();

		(pixels, canvas.width() as usize)
	}

	/// How many of the canvas's pixels are exactly `color`.
	fn count(&self, color: [u8; 3]) -> usize {
		self.pixels()
			.0
			.iter()
			.filter(|pixel| **pixel == color)
			.count()
	}

	/// Where the drawing's paper is on the canvas.
	fn sheet(&self) -> Sheet {
		self.find_sheet()
			.expect("the canvas shows a drawing's paper")
	}

	fn find_sheet(&self) -> Option<Sheet> {
		let (pixels, width) = self.pixels();
		let scale = width as f64 / self.canvas().get_bounding_client_rect().width();
		// Rows that are mostly paper: the grey edges of letters can be the
		// paper's colour too, a pixel here and there.
		let rows: Vec<usize> = pixels
			.chunks(width)
			.enumerate()
			.filter(|(_, row)| row.iter().filter(|pixel| **pixel == PAPER).count() > width / 2)
			.map(|(index, _)| index)
			.collect();
		let (top, bottom) = (*rows.first()?, rows.last()? + 1);
		// The corners are round, so the middle row is the widest.
		let middle = &pixels[usize::midpoint(top, bottom) * width..][..width];
		let left = middle.iter().position(|pixel| *pixel == PAPER).unwrap();
		let right = middle.iter().rposition(|pixel| *pixel == PAPER).unwrap() + 1;

		let css = |pixels: usize| (pixels as f64 / scale).round() as i32;

		Some(Sheet {
			left: css(left),
			top: css(top),
			width: css(right - left),
			height: css(bottom - top),
		})
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
/// editor paints a stroke in progress.
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

#[wasm_bindgen_test]
fn a_drawing_goes_in_at_the_caret_between_paragraphs() {
	let fixture = Fixture::new(PLAIN);
	let json = fixture.json();
	let order: Vec<_> = ["above", r#""type":"ink""#, "below"]
		.iter()
		.map(|part| {
			json.find(part)
				.unwrap_or_else(|| panic!("{part} in {json}"))
		})
		.collect();

	assert!(order.is_sorted(), "{json}");
	assert_eq!(fixture.strokes(), 0);

	let sheet = fixture.sheet();
	assert_eq!(
		(sheet.left, sheet.width, sheet.height),
		(0, 400, 160),
		"a drawing is as wide as the editor and two fifths as tall"
	);
	assert!(sheet.top > 24, "the first paragraph is above it: {sheet:?}");

	// Undo takes "below" away first, then the drawing.
	while fixture.json().contains(r#""type":"ink""#) {
		assert!(fixture.state.undo());
	}

	assert_eq!(fixture.find_sheet(), None, "and the canvas follows");
	assert!(fixture.state.redo());
	assert_eq!(fixture.find_sheet(), Some(sheet));
}

#[wasm_bindgen_test]
async fn dragging_on_a_drawing_draws_a_stroke() {
	let fixture = Fixture::new(PLAIN);
	let sheet = fixture.sheet();
	let changes = fixture.changes();
	fixture
		.state
		.set_pen(Pen::new().color(0xe1, 0x1d, 0x48).width(12.0));

	fixture.pointer("pointerdown", sheet.at(40, 40), "mouse");
	fixture.pointer("pointermove", sheet.at(120, 80), "mouse");
	fixture.pointer("pointermove", sheet.at(200, 40), "mouse");
	frames(2).await;

	assert!(
		fixture.count(ROSE) > 100,
		"the stroke shows while the pointer is down"
	);
	assert_eq!(fixture.strokes(), 0, "and is not in the document yet");
	assert!(changes.lock().unwrap().is_empty());

	fixture.pointer("pointerup", sheet.at(200, 40), "mouse");
	let json = fixture.json();

	assert!(
		json.contains(
			r##"{"color":"#e11d48","points":[100.0,100.0,300.0,200.0,500.0,100.0],"width":12.0}"##
		),
		"the stroke is stored in the drawing's own units: {json}"
	);
	assert!(fixture.count(ROSE) > 100, "and stays on the canvas");

	let changes = changes.lock().unwrap();
	assert_eq!(changes.len(), 1, "one stroke is one change");
	assert!(changes[0].doc_changed);
	assert!(!changes[0].selection_changed, "the caret stays in the text");
}

#[wasm_bindgen_test]
fn the_pen_is_the_text_colour_until_it_is_set() {
	let fixture = Fixture::new(PLAIN);
	let sheet = fixture.sheet();
	let before = fixture.count(PAPER);
	fixture.draw(&[sheet.at(40, 40), sheet.at(120, 80)]);
	let json = fixture.json();

	assert!(
		json.contains(r#""strokes":[{"points":[100.0,100.0,300.0,200.0]}]"#),
		"a stroke in the default pen stores neither colour nor width: {json}"
	);
	assert!(fixture.count(PAPER) < before, "ink covers some paper");
	assert_eq!(fixture.count(ROSE), 0);

	fixture
		.state
		.set_pen(Pen::new().color(0xe1, 0x1d, 0x48).width(12.0));
	fixture.draw(&[sheet.at(40, 120), sheet.at(200, 120)]);

	assert_eq!(fixture.strokes(), 2);
	assert!(fixture.count(ROSE) > 100);
	assert!(
		fixture
			.json()
			.contains(r#"{"points":[100.0,100.0,300.0,200.0]}"#),
		"the first stroke keeps the pen it was drawn with"
	);
}

#[wasm_bindgen_test]
fn each_stroke_is_one_step_of_undo() {
	let fixture = Fixture::new(PLAIN);
	let sheet = fixture.sheet();
	fixture
		.state
		.set_pen(Pen::new().color(0xe1, 0x1d, 0x48).width(12.0));
	fixture.draw(&[sheet.at(40, 40), sheet.at(200, 40)]);
	fixture.draw(&[sheet.at(40, 120), sheet.at(200, 120)]);
	let both = fixture.count(ROSE);

	assert_eq!(fixture.strokes(), 2);

	// A button for people without a keyboard.
	assert!(fixture.state.undo());
	assert_eq!(fixture.strokes(), 1);
	let one = fixture.count(ROSE);
	assert!(one > 0 && one < both, "{one} of {both}");

	// The key for people with one.
	assert!(fixture.undo_key());
	assert_eq!(fixture.strokes(), 0);
	assert_eq!(fixture.count(ROSE), 0);
	assert!(fixture.json().contains(r#""type":"ink""#));

	assert!(fixture.state.redo());
	assert!(fixture.state.redo());
	assert_eq!(fixture.strokes(), 2);
	assert_eq!(fixture.count(ROSE), both);
}

#[wasm_bindgen_test]
fn a_tap_leaves_a_dot_and_a_stroke_stays_on_its_paper() {
	let fixture = Fixture::new(PLAIN);
	let sheet = fixture.sheet();
	fixture
		.state
		.set_pen(Pen::new().color(0xe1, 0x1d, 0x48).width(20.0));
	fixture.draw(&[sheet.at(200, 80)]);

	assert!(
		fixture.json().contains(r#""points":[500.0,200.0]"#),
		"{}",
		fixture.json()
	);
	assert!(fixture.count(ROSE) > 10, "a dot");

	// Dragged past the bottom right corner, onto the text below.
	fixture.draw(&[sheet.at(40, 40), sheet.at(600, 400)]);

	assert!(
		fixture
			.json()
			.contains(r#""points":[100.0,100.0,1000.0,400.0]"#),
		"{}",
		fixture.json()
	);
}

#[wasm_bindgen_test]
async fn a_pointer_the_browser_takes_away_draws_nothing() {
	let fixture = Fixture::new(PLAIN);
	let sheet = fixture.sheet();
	fixture
		.state
		.set_pen(Pen::new().color(0xe1, 0x1d, 0x48).width(12.0));
	fixture.pointer("pointerdown", sheet.at(40, 40), "touch");
	fixture.pointer("pointermove", sheet.at(200, 40), "touch");
	frames(2).await;
	assert!(fixture.count(ROSE) > 100);

	fixture.pointer("pointercancel", sheet.at(200, 40), "touch");
	frames(2).await;

	assert_eq!(fixture.strokes(), 0);
	assert_eq!(fixture.count(ROSE), 0, "the unfinished stroke is wiped");

	// The next move is not part of any stroke.
	fixture.pointer("pointermove", sheet.at(300, 100), "touch");
	fixture.pointer("pointerup", sheet.at(300, 100), "touch");
	assert_eq!(fixture.strokes(), 0);
}

#[wasm_bindgen_test]
fn a_read_only_editor_does_not_draw() {
	let fixture = Fixture::new(PLAIN);
	let sheet = fixture.sheet();
	fixture.state.set_read_only(true);
	fixture.draw(&[sheet.at(40, 40), sheet.at(200, 40)]);

	assert_eq!(fixture.strokes(), 0);

	fixture.pointer("pointermove", sheet.at(40, 40), "mouse");
	assert_eq!(
		fixture
			.canvas()
			.style()
			.get_property_value("cursor")
			.unwrap(),
		"text",
		"and does not offer to"
	);

	// The app still can.
	fixture.state.set_read_only(false);
	fixture.draw(&[sheet.at(40, 40), sheet.at(200, 40)]);
	assert_eq!(fixture.strokes(), 1);
}

#[wasm_bindgen_test]
async fn an_editor_made_read_only_mid_stroke_drops_the_stroke() {
	let fixture = Fixture::new(PLAIN);
	let sheet = fixture.sheet();
	fixture
		.state
		.set_pen(Pen::new().color(0xe1, 0x1d, 0x48).width(12.0));
	fixture.pointer("pointerdown", sheet.at(40, 40), "mouse");
	fixture.pointer("pointermove", sheet.at(200, 40), "mouse");
	frames(2).await;
	assert!(fixture.count(ROSE) > 100);

	fixture.state.set_read_only(true);
	fixture.pointer("pointerup", sheet.at(200, 40), "mouse");
	frames(2).await;

	assert_eq!(fixture.strokes(), 0);
	assert_eq!(fixture.count(ROSE), 0, "the unfinished stroke is wiped");
}

#[wasm_bindgen_test]
fn a_press_on_text_still_places_the_caret() {
	let fixture = Fixture::new(PLAIN);
	let sheet = fixture.sheet();
	let changes = fixture.changes();
	// The first line, above the drawing.
	fixture.draw(&[(0, 12), (30, 12)]);

	assert_eq!(fixture.strokes(), 0);
	assert!(
		changes
			.lock()
			.unwrap()
			.iter()
			.all(|change| change.selection_changed && !change.doc_changed),
		"dragging over text selects it"
	);
	assert!(!changes.lock().unwrap().is_empty());

	// Shift extends that selection even when the press lands on the drawing.
	let init = PointerEventInit::new();
	let bounds = fixture.canvas().get_bounding_client_rect();
	let (x, y) = sheet.at(40, 40);
	init.set_client_x(bounds.left() as i32 + x);
	init.set_client_y(bounds.top() as i32 + y);
	init.set_button(0);
	init.set_pointer_id(1);
	init.set_shift_key(true);
	init.set_bubbles(true);
	init.set_cancelable(true);
	let press = PointerEvent::new_with_event_init_dict("pointerdown", &init).unwrap();
	fixture.canvas().dispatch_event(&press).unwrap();
	fixture.pointer("pointerup", sheet.at(40, 40), "mouse");

	assert_eq!(fixture.strokes(), 0);
}

#[wasm_bindgen_test]
fn a_finger_that_draws_neither_scrolls_nor_raises_the_keyboard() {
	let fixture = Fixture::new(PLAIN);
	let sheet = fixture.sheet();
	let field: Element = fixture.field().into();
	let touch_start = || {
		let init = EventInit::new();
		init.set_bubbles(true);
		init.set_cancelable(true);
		let event = Event::new_with_event_init_dict("touchstart", &init).unwrap();
		// Whether the editor stopped the browser's own handling: scrolling.
		!fixture.canvas().dispatch_event(&event).unwrap()
	};
	document()
		.active_element()
		.and_then(|element| element.dyn_into::<HtmlElement>().ok())
		.expect("something has the focus")
		.blur()
		.unwrap();

	// On text, a finger scrolls the page as it does anywhere else.
	fixture.pointer("pointerdown", (0, 12), "touch");
	assert!(!touch_start());
	fixture.pointer("pointerup", (0, 12), "touch");
	document()
		.active_element()
		.and_then(|element| element.dyn_into::<HtmlElement>().ok())
		.expect("something has the focus")
		.blur()
		.unwrap();

	// The browser sends `pointerdown` before `touchstart`.
	fixture.pointer("pointerdown", sheet.at(40, 40), "touch");
	assert!(touch_start(), "a touch that draws must not scroll");
	assert_ne!(
		document().active_element(),
		Some(field.clone()),
		"focusing the text field would raise the on-screen keyboard"
	);
	fixture.pointer("pointermove", sheet.at(200, 40), "touch");
	fixture.pointer("pointerup", sheet.at(200, 40), "touch");
	assert_eq!(fixture.strokes(), 1);

	// A mouse has a keyboard beside it, and undo needs the focus.
	fixture.draw(&[sheet.at(40, 120), sheet.at(200, 120)]);
	assert_eq!(document().active_element(), Some(field));
}

#[wasm_bindgen_test]
fn the_canvas_says_where_a_press_would_draw() {
	let fixture = Fixture::new(PLAIN);
	let sheet = fixture.sheet();
	let style = |name: &str| fixture.canvas().style().get_property_value(name).unwrap();

	fixture.pointer("pointermove", sheet.at(40, 40), "mouse");
	assert_eq!(style("cursor"), "crosshair");
	assert_eq!(style("touch-action"), "auto");

	fixture.pointer("pointermove", (0, 12), "mouse");
	assert_eq!(style("cursor"), "text");

	// A pen scrolls the page unless the browser is told otherwise before it
	// comes down.
	fixture.pointer("pointermove", sheet.at(40, 40), "pen");
	assert_eq!(style("touch-action"), "none");
	assert_eq!(style("cursor"), "crosshair");

	fixture.pointer("pointermove", (0, 12), "pen");
	assert_eq!(style("touch-action"), "auto");

	fixture.pointer("pointermove", sheet.at(40, 40), "pen");
	fixture.pointer("pointerleave", sheet.at(40, 40), "pen");
	assert_eq!(style("touch-action"), "auto", "a pen that left may scroll");
	assert_eq!(style("cursor"), "text");
}

#[wasm_bindgen_test]
async fn drawing_keeps_the_page_where_it_is() {
	let fixture = Fixture::new(&format!("{PLAIN};height:120px;overflow:auto"));
	// The caret is in "below", under the drawing, and the editor scrolled
	// to it. Put the caret on the first line, then scroll that line out of
	// view.
	fixture.container.set_scroll_top(0);
	frames(2).await;
	fixture.draw(&[(0, 12)]);
	assert_eq!(fixture.strokes(), 0);
	fixture.container.set_scroll_top(60);
	frames(2).await;

	assert_eq!(fixture.container.scroll_top(), 60);
	let sheet = fixture.sheet();
	fixture.draw(&[(40, sheet.top.max(0) + 20), (200, sheet.top.max(0) + 20)]);

	assert_eq!(fixture.strokes(), 1);
	assert_eq!(
		fixture.container.scroll_top(),
		60,
		"a stroke does not scroll to the caret"
	);
}

#[wasm_bindgen_test]
async fn the_hidden_copy_names_the_drawing() {
	let fixture = Fixture::new(PLAIN);
	let mirror: Element = fixture.part("mirror");

	// The copy is refreshed when the browser is idle.
	for _ in 0..120 {
		if mirror.child_element_count() == 3 {
			break;
		}

		frames(1).await;
	}

	assert_eq!(
		mirror.inner_html(),
		r#"<p>above</p><div role="img" aria-label="Drawing"></div><p>below</p>"#
	);
}

#[wasm_bindgen_test]
fn a_saved_drawing_loads_and_shows_its_strokes() {
	let fixture = Fixture::new(PLAIN);
	let sheet = fixture.sheet();
	fixture
		.state
		.set_pen(Pen::new().color(0xe1, 0x1d, 0x48).width(12.0));
	fixture.draw(&[sheet.at(40, 40), sheet.at(200, 40)]);
	let markdown = now(fixture.state.export(Format::Markdown)).expect("a live editor");
	let drawn = fixture.count(ROSE);

	fixture.state.load(Format::Text, "").expect("plain text");
	fixture.render();
	assert_eq!(fixture.count(ROSE), 0);

	fixture
		.state
		.load(Format::Markdown, &markdown)
		.expect("its own Markdown");
	fixture.render();

	assert_eq!(fixture.strokes(), 1);
	assert_eq!(fixture.count(ROSE), drawn, "the same picture");
}

#[wasm_bindgen_test]
fn backspace_selects_a_drawing_and_then_removes_it() {
	let fixture = Fixture::new(PLAIN);
	let key = |key: &str| {
		let init = KeyboardEventInit::new();
		init.set_key(key);
		init.set_bubbles(true);
		init.set_cancelable(true);
		let event = KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
		!fixture.field().dispatch_event(&event).unwrap()
	};

	// From the start of "below", the paragraph after the drawing.
	key("Home");
	assert!(key("Backspace"));
	assert!(
		fixture.json().contains(r#""type":"ink""#),
		"the first press only selects the drawing"
	);
	assert_eq!(
		fixture.find_sheet(),
		None,
		"and the selection shows over its paper"
	);
	assert_eq!(
		fixture.state.insert_drawing(),
		Ok(false),
		"a drawing does not go inside a drawing"
	);

	assert!(key("Backspace"));
	assert!(!fixture.json().contains(r#""type":"ink""#));
	assert_eq!(fixture.find_sheet(), None);
	assert_eq!(
		now(fixture.state.export(Format::Text)).as_deref(),
		Ok("above\nbelow")
	);

	assert!(fixture.state.undo(), "undo brings the drawing back");
	assert!(fixture.json().contains(r#""type":"ink""#));
}
