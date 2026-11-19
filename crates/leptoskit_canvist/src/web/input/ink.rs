//! The pointer on a drawing: a press there draws a stroke instead of
//! selecting text.

use std::rc::Rc;

use wasm_bindgen::JsCast;
use web_sys::Event;
use web_sys::PointerEvent;

use crate::web::editor::Editor;
use crate::web::host::Host;

/// What the pointer last hovered over, which the canvas's styles follow.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::web) enum Hover {
	/// Text, or a drawing nobody may draw on.
	#[default]
	Text,
	/// A drawing a press would draw on.
	Drawing,
	/// The same, under a pen.
	DrawingUnderPen,
}

/// Start a stroke if the press at document coordinates `point` is on a
/// drawing. Returns whether it did.
pub(super) fn press(host: &Rc<Host>, event: &PointerEvent, point: (f32, f32)) -> bool {
	// Shift extends the text selection wherever the press lands.
	if event.shift_key() {
		return false;
	}

	let started = host
		.with_editor(|editor, session| editor.start_scribble(session, point))
		.unwrap_or(false);

	if !started {
		return false;
	}

	event.prevent_default();

	// A mouse comes with a keyboard, and undo needs the focus. A finger or a
	// pen does not, and focusing the text field would raise the on-screen
	// keyboard over the drawing.
	if event.pointer_type() == "mouse" {
		host.focus();
	}

	host.pressed.set(true);
	// A pointer that is already gone cannot be captured, which is fine.
	let _ = host.canvas.set_pointer_capture(event.pointer_id());
	// A press that never moves leaves a dot.
	host.invalidate();

	true
}

/// The pointer moved. Returns whether the move belonged to a stroke.
pub(super) fn moved(host: &Rc<Host>, event: &PointerEvent) -> bool {
	if !host.pressed.get() {
		hover(host, event);
		return false;
	}

	let scribbling = host
		.with_editor(|editor, _| editor.is_scribbling())
		.unwrap_or(false);

	if !scribbling {
		return false;
	}

	let (x, y) = client(event);
	let (left, top) = host.document_point(x, y);
	let positions = positions(event);
	host.with_editor(|editor, _| {
		for (client_x, client_y) in positions {
			editor.extend_scribble((left + (client_x - x) as f32, top + (client_y - y) as f32));
		}
	});
	// Moves arrive faster than frames on some screens: paint once a frame.
	host.invalidate();

	true
}

/// The pointer was released, or with `finished` false the browser took it
/// away: the stroke joins the document, or is dropped.
pub(super) fn release(host: &Rc<Host>, finished: bool) {
	let change = if finished {
		host.edit(Editor::finish_scribble)
	} else {
		None
	};

	if let Some(change) = change {
		host.drawn(change);
	} else if host.with_editor(|editor, _| editor.cancel_scribble()) == Some(true) {
		host.invalidate();
	}
}

/// A touch began. When it began a stroke, it must not also scroll the page:
/// the browser sends `pointerdown` first, so by now the editor knows.
pub(super) fn touch_start(host: &Rc<Host>, event: &Event) {
	let scribbling = host
		.with_editor(|editor, _| editor.is_scribbling())
		.unwrap_or(false);

	if scribbling && event.cancelable() {
		event.prevent_default();
	}
}

/// The pointer left the canvas: forget what it hovered over.
pub(super) fn leave(host: &Rc<Host>) {
	show(host, Hover::Text);
}

/// Show where a press would draw before it lands.
fn hover(host: &Rc<Host>, event: &PointerEvent) {
	if !host.inked {
		return;
	}

	let (x, y) = client(event);
	let point = host.document_point(x, y);
	let draws = host
		.with_editor(|editor, session| editor.draws_at(session, point))
		.unwrap_or(false);

	show(
		host,
		match (draws, event.pointer_type().as_str()) {
			(false, _) => Hover::Text,
			(true, "pen") => Hover::DrawingUnderPen,
			(true, _) => Hover::Drawing,
		},
	);
}

/// Style the canvas for what the pointer is over.
fn show(host: &Rc<Host>, hover: Hover) {
	if host.hover.replace(hover) == hover {
		return;
	}

	let style = host.canvas.style();
	let _ = style.set_property(
		"cursor",
		if hover == Hover::Text {
			"text"
		} else {
			"crosshair"
		},
	);
	// A pen dragged across a page scrolls it, and the browser decides that
	// as the pen comes down, so it has to be told while the pen hovers. A
	// finger never hovers; `touch_start` speaks for it.
	let _ = style.set_property(
		"touch-action",
		if hover == Hover::DrawingUnderPen {
			"none"
		} else {
			"auto"
		},
	);
}

/// Where `event` happened in the page, to the fraction of a pixel the
/// browser knows it to: `web-sys` reads these as whole numbers.
fn client(event: &PointerEvent) -> (f64, f64) {
	let read = |name: &str, whole: i32| {
		js_sys::Reflect::get(event, &name.into())
			.ok()
			.and_then(|value| value.as_f64())
			.unwrap_or_else(|| f64::from(whole))
	};

	(
		read("clientX", event.client_x()),
		read("clientY", event.client_y()),
	)
}

/// Every position `event` stands for, oldest first. A browser delivers one
/// move a frame and keeps the positions the pointer passed through on the
/// way, which make a fast stroke round instead of angular. Browsers without
/// them give the one position.
fn positions(event: &PointerEvent) -> Vec<(f64, f64)> {
	let coalesced = js_sys::Reflect::get(event, &"getCoalescedEvents".into())
		.ok()
		.and_then(|method| method.dyn_into::<js_sys::Function>().ok())
		.and_then(|method| method.call0(event).ok())
		.and_then(|events| events.dyn_into::<js_sys::Array>().ok());
	let mut positions: Vec<_> = coalesced
		.iter()
		.flat_map(js_sys::Array::iter)
		.filter_map(|event| event.dyn_into::<PointerEvent>().ok())
		.map(|event| client(&event))
		.collect();

	if positions.is_empty() {
		positions.push(client(event));
	}

	positions
}
