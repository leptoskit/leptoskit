//! Input: the keyboard, input methods, the pointer, and the clipboard,
//! turned into editor changes.
//!
//! A transparent text field at the caret takes the focus and the text. Key
//! bindings run first, and every change is painted in the same task as its
//! event.
//!
//! Ported from the text field path of canvist's
//! `packages/canvist-web/src/input.ts`.

use std::rc::Rc;

use canvist_headless::canvist_core::Modifiers;
use wasm_bindgen::JsCast;
use web_sys::ClipboardEvent;
use web_sys::CompositionEvent;
use web_sys::Event;
use web_sys::EventTarget;
use web_sys::InputEvent;
use web_sys::KeyboardEvent;
use web_sys::PointerEvent;

use super::device;
use super::editor;
use super::host::Host;
use super::host::Listen;

#[cfg(feature = "ink")]
pub(super) mod ink;

/// Keys that only move the selection, which a read-only editor still
/// handles (with any modifiers) so people can select text to copy.
const NAVIGATION_KEYS: [&str; 8] = [
	"ArrowLeft",
	"ArrowRight",
	"ArrowUp",
	"ArrowDown",
	"Home",
	"End",
	"PageUp",
	"PageDown",
];

/// Presses this close in time (ms) and space (CSS px) count as one
/// multi-click.
const MULTI_CLICK_MS: f64 = 500.0;
const MULTI_CLICK_SLOP: i32 = 4;

/// The key code of a key an input method is handling.
const KEY_CODE_COMPOSING: u32 = 229;

/// The last press, for counting double and triple clicks.
#[derive(Clone, Copy, Debug)]
pub(super) struct Press {
	count: u32,
	time: f64,
	x: i32,
	y: i32,
}

impl Default for Press {
	fn default() -> Self {
		Self {
			count: 0,
			time: f64::NEG_INFINITY,
			x: 0,
			y: 0,
		}
	}
}

/// Listen for everything people do to the editor.
pub(super) fn listen(host: &Rc<Host>) {
	let how = Listen::default();
	let (textarea, canvas) = (&host.textarea, &host.canvas);

	// leptoskit promises that an app's key handlers hear every key inside
	// them before the focused control acts on it, and those handlers run as
	// the event bubbles. So keys are handled after it has bubbled all the
	// way, and not at all when a handler prevented them.
	host.on(&host.window, "keydown", key_down, how);
	host.on(textarea, "focus", |host, _| host.set_focused(true), how);
	host.on(textarea, "blur", |host, _| host.set_focused(false), how);
	host.on(textarea, "beforeinput", before_input, how);
	host.on(
		textarea,
		"compositionstart",
		|host, _| host.composing.set(true),
		how,
	);
	host.on(textarea, "compositionupdate", composition_update, how);
	host.on(textarea, "compositionend", composition_end, how);
	// Whatever the browser put in the text field has been applied already.
	host.on(textarea, "input", |host, _| clear_field(host), how);
	host.on(canvas, "pointerdown", pointer_down, how);
	host.on(canvas, "pointermove", pointer_move, how);
	host.on(canvas, "pointerup", |host, _| release(host, true), how);
	host.on(canvas, "pointercancel", |host, _| release(host, false), how);
	#[cfg(feature = "ink")]
	{
		host.on(canvas, "touchstart", ink::touch_start, how);
		host.on(canvas, "pointerleave", |host, _| ink::leave(host), how);
	}
	// On the text field, not the container: the clipboard events of other
	// controls an app puts inside the container are theirs.
	host.on(
		textarea,
		"copy",
		|host, event| copy(host, event, false),
		how,
	);
	host.on(textarea, "cut", |host, event| copy(host, event, true), how);
	host.on(textarea, "paste", paste, how);
}

/// Run the key binding for `event`; keys without one reach the browser as
/// text.
fn key_down(host: &Rc<Host>, event: &Event) {
	let Some(event) = event.dyn_ref::<KeyboardEvent>() else {
		return;
	};
	let field: &EventTarget = &host.textarea;
	let ours = event.target().is_some_and(|target| &target == field);

	// A key that an input method is handling (`Process`, key code 229)
	// belongs to the composition.
	if !ours
		|| event.default_prevented()
		|| event.is_composing()
		|| host.composing.get()
		|| event.key_code() == KEY_CODE_COMPOSING
	{
		return;
	}

	let key = event.key();

	if host.read_only() && !read_only_key(host, event, &key) {
		// WebKit treats Backspace outside editable text as Back.
		if key == "Backspace" {
			event.prevent_default();
		}

		return;
	}

	let modifiers = Modifiers {
		shift: event.shift_key(),
		ctrl: event.ctrl_key(),
		alt: event.alt_key(),
		meta: event.meta_key(),
	};

	if let Some(change) = host.edit(|editor, session| editor.key_down(session, &key, modifiers)) {
		event.prevent_default();
		host.input_changed(change);
	}
}

/// Whether a read-only editor handles `event`: navigation, and select all.
fn read_only_key(host: &Host, event: &KeyboardEvent, key: &str) -> bool {
	let command = if device::is_apple(&host.window) {
		event.meta_key()
	} else {
		event.ctrl_key()
	};

	NAVIGATION_KEYS.contains(&key) || (command && key.eq_ignore_ascii_case("a"))
}

/// Text input through the text field: typed text, and edits without a key
/// binding (as from soft keyboards).
fn before_input(host: &Rc<Host>, event: &Event) {
	let Some(event) = event.dyn_ref::<InputEvent>() else {
		return;
	};

	// Chromium still sends text input to a read-only text field.
	if host.read_only() {
		event.prevent_default();
		return;
	}

	let kind = event.input_type();

	// Compositions, pastes and drops arrive as their own events.
	if host.composing.get()
		|| matches!(
			kind.as_str(),
			"insertCompositionText" | "insertFromPaste" | "insertFromDrop"
		) {
		return;
	}

	event.prevent_default();
	let shift = Modifiers {
		shift: true,
		..Modifiers::NONE
	};
	let change = host.edit(|editor, session| {
		match kind.as_str() {
			"insertText" | "insertReplacementText" => {
				let text = event
					.data()
					.or_else(|| event.data_transfer()?.get_data("text/plain").ok())
					.unwrap_or_default();
				editor.insert_text(session, &text)
			}
			"insertLineBreak" => editor.key_down(session, "Enter", shift),
			"insertParagraph" => editor.key_down(session, "Enter", Modifiers::NONE),
			"deleteContentBackward" => editor.key_down(session, "Backspace", Modifiers::NONE),
			"deleteContentForward" => editor.key_down(session, "Delete", Modifiers::NONE),
			"historyUndo" => editor.history(session, false),
			"historyRedo" => editor.history(session, true),
			_ => Ok(None),
		}
	});

	if let Some(change) = change {
		host.input_changed(change);
	}
}

/// An input method changed its unfinished text.
fn composition_update(host: &Rc<Host>, event: &Event) {
	let text = composed(event);

	if let Some(change) = host.edit(|editor, session| editor.set_composition(session, &text)) {
		host.input_changed(change);
	}
}

/// An input method finished: its text goes into the document once.
fn composition_end(host: &Rc<Host>, event: &Event) {
	let text = composed(event);
	host.composing.set(false);
	let change = host.edit(|editor, session| editor.commit_composition(session, &text));
	host.textarea.set_value("");

	if let Some(change) = change {
		host.input_changed(change);
	}
}

fn composed(event: &Event) -> String {
	event
		.dyn_ref::<CompositionEvent>()
		.and_then(CompositionEvent::data)
		.unwrap_or_default()
}

fn clear_field(host: &Host) {
	if !host.composing.get() {
		host.textarea.set_value("");
	}
}

fn pointer_down(host: &Rc<Host>, event: &Event) {
	let Some(event) = event.dyn_ref::<PointerEvent>() else {
		return;
	};

	if event.button() != 0 {
		return;
	}

	let point = host.document_point(f64::from(event.client_x()), f64::from(event.client_y()));

	#[cfg(feature = "ink")]
	if ink::press(host, event, point) {
		return;
	}

	let clicks = count(host, event);
	// Keep focus in the text field rather than letting the press blur it.
	event.prevent_default();
	host.focus();
	host.pressed.set(true);
	// A pointer that is already gone cannot be captured, which is fine.
	let _ = host.canvas.set_pointer_capture(event.pointer_id());
	let extend = event.shift_key();

	if let Some(change) =
		host.edit(|editor, session| editor.pointer_down(session, point, extend, clicks))
	{
		host.input_changed(change);
	}
}

/// How many clicks `event` makes in a row. Pointer events don't count
/// clicks in every browser, so presses close in time and place are counted
/// here when `detail` is zero.
fn count(host: &Host, event: &PointerEvent) -> u32 {
	let last = host.last_press.get();
	let (x, y, time) = (event.client_x(), event.client_y(), event.time_stamp());
	let near = (x - last.x).abs() <= MULTI_CLICK_SLOP
		&& (y - last.y).abs() <= MULTI_CLICK_SLOP
		&& time - last.time <= MULTI_CLICK_MS;
	let count = match u32::try_from(event.detail()) {
		Ok(detail) if detail > 0 => detail,
		_ if near => last.count + 1,
		_ => 1,
	};
	host.last_press.set(Press { count, time, x, y });

	count
}

fn pointer_move(host: &Rc<Host>, event: &Event) {
	let Some(event) = event.dyn_ref::<PointerEvent>() else {
		return;
	};

	#[cfg(feature = "ink")]
	if ink::moved(host, event) {
		return;
	}

	if !host.pressed.get() {
		return;
	}

	let point = host.document_point(f64::from(event.client_x()), f64::from(event.client_y()));

	if let Some(change) = host.edit(|editor, session| editor.pointer_move(session, point)) {
		host.input_changed(change);
	}
}

/// The pointer was released, or with `finished` false the browser took it
/// away.
#[cfg_attr(
	not(feature = "ink"),
	expect(unused_variables, reason = "only a stroke ends differently")
)]
fn release(host: &Rc<Host>, finished: bool) {
	host.pressed.set(false);

	#[cfg(feature = "ink")]
	ink::release(host, finished);

	if let Ok(mut editor) = host.editor.try_borrow_mut() {
		editor.pointer_up();
	}
}

/// Put the selection on the clipboard as plain text, and remove it from
/// the document when `cut`.
fn copy(host: &Rc<Host>, event: &Event, cut: bool) {
	let Some((event, data)) = event
		.dyn_ref::<ClipboardEvent>()
		.and_then(|event| Some((event, event.clipboard_data()?)))
	else {
		return;
	};
	// Cutting from a read-only document copies.
	let cut = cut && !host.read_only();
	let mut copied = None;
	let change = host.edit(|editor, session| {
		if cut {
			let cut = editor.cut(session)?;
			let (text, change) = cut.unzip();
			copied = text;
			Ok(change.flatten())
		} else {
			copied = editor::copy(&session.doc)?;
			Ok(None)
		}
	});

	// With nothing selected, the browser's own copy stands: of nothing.
	let Some(text) = copied else {
		return;
	};

	event.prevent_default();
	let _ = data.set_data("text/plain", &text);

	if let Some(change) = change {
		host.input_changed(change);
	}
}

/// Paste the clipboard's plain text over the selection.
fn paste(host: &Rc<Host>, event: &Event) {
	let Some((event, data)) = event
		.dyn_ref::<ClipboardEvent>()
		.and_then(|event| Some((event, event.clipboard_data()?)))
	else {
		return;
	};

	if host.read_only() {
		return;
	}

	event.prevent_default();
	let text = data.get_data("text/plain").unwrap_or_default();

	if text.is_empty() {
		return;
	}

	if let Some(change) = host.edit(|editor, session| editor.paste_text(session, &text)) {
		host.input_changed(change);
	}
}
