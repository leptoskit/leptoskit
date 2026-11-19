//! The window listener that hands keys to [app shortcuts](crate::shortcut).

use std::cell::Cell;

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

thread_local! {
	static LISTENING: Cell<bool> = const { Cell::new(false) };
}

/// Starts listening for shortcut keys on the window, once.
///
/// The listener sits on the window, so it hears a key after every element
/// it passed through. An element whose `on_key` handled the key stopped it
/// and prevented its default, so a handled key never reaches a shortcut.
/// Neither does a key pressed while a `Modal` is open.
pub(crate) fn listen() {
	if LISTENING.with(|listening| listening.replace(true)) {
		return;
	}

	let Some(window) = web_sys::window() else {
		return;
	};
	let listener = Closure::<dyn Fn(web_sys::KeyboardEvent)>::new(|event| on_keydown(&event));

	if window
		.add_event_listener_with_callback("keydown", listener.as_ref().unchecked_ref())
		.is_ok()
	{
		// The listener lives as long as the page.
		listener.forget();
	} else {
		LISTENING.with(|listening| listening.set(false));
	}
}

fn on_keydown(event: &web_sys::KeyboardEvent) {
	if event.default_prevented() || event.is_composing() || modal_open() {
		return;
	}

	let press = crate::KeyPress {
		key: event.key(),
		ctrl: event.ctrl_key(),
		alt: event.alt_key(),
		shift: event.shift_key(),
		meta: event.meta_key(),
	};

	if crate::shortcut::dispatch(&press, typing(event)) {
		// A shortcut replaces the browser's own meaning of the key, so ⌘K
		// does not also open the browser's search.
		event.prevent_default();
	}
}

/// Whether a modal is open. Its keys are its own: Escape asks it to close
/// rather than running the app's Escape, and the app's other shortcuts
/// wait, as everything else behind a modal does.
fn modal_open() -> bool {
	web_sys::window()
		.and_then(|window| window.document())
		.and_then(|document| document.query_selector("dialog:modal").ok().flatten())
		.is_some()
}

/// Whether the key goes into a text field: a text input, a text area, a
/// select that jumps to the letter typed, or editable content.
fn typing(event: &web_sys::KeyboardEvent) -> bool {
	let Some(target) = event.target() else {
		return false;
	};

	if let Some(input) = target.dyn_ref::<web_sys::HtmlInputElement>() {
		return !matches!(
			input.type_().as_str(),
			"button"
				| "checkbox"
				| "color"
				| "file"
				| "image"
				| "radio"
				| "range"
				| "reset"
				| "submit"
		);
	}

	target.is_instance_of::<web_sys::HtmlTextAreaElement>()
		|| target.is_instance_of::<web_sys::HtmlSelectElement>()
		|| target
			.dyn_ref::<web_sys::HtmlElement>()
			.is_some_and(web_sys::HtmlElement::is_content_editable)
}
