//! The editor in a web page.
//!
//! [`attach`] puts an editor in an element: a canvas that paints the
//! document, a transparent text field that takes the keyboard, and a
//! hidden copy of the text for screen readers. All of it is this crate's
//! Rust, compiled to Wasm; the page needs no script of its own.
//!
//! ```no_run
//! use leptoskit_canvist::CanvistState;
//! use leptoskit_canvist::Extensions;
//! use leptoskit_canvist::web;
//!
//! # fn mount(container: &web_sys::HtmlElement) -> Result<(), leptoskit_canvist::CanvistError> {
//! let note = CanvistState::new(Extensions::paragraphs());
//! note.set_placeholder("Write a note…");
//! let attachment = web::attach(container, note)?;
//! // Keep `attachment` for as long as the editor is on screen.
//! # drop(attachment);
//! # Ok(())
//! # }
//! ```
//!
//! # In a leptoskit element
//!
//! An app built with leptoskit does not call [`attach`]. It gives the
//! editor's id to a `CanvistEditor` element, and the editor appears in that
//! element when the element does and leaves with it:
//!
//! ```ignore
//! let note = CanvistState::new(Extensions::paragraphs());
//!
//! CanvistEditor(note.id())
//! 	.accessibility_label("Note")
//! 	.placeholder("Write a note…")
//! 	.class("min-h-40 w-full rounded-lg border p-3 text-base")
//! ```
//!
//! The element's own settings reach the editor and follow it as they
//! change: its placeholder, `read_only`, its accessible label, `focused`,
//! and its classes, which the editor reads its text style from. An element
//! that sets no placeholder and is not read-only leaves alone what the app
//! set on the handle. `on_focus` and `on_blur` handlers on the element hear
//! the editor gain and lose the keyboard. A disabled element cannot be
//! edited, but its text can still be selected.
//!
//! On a server-rendered page the element arrives empty and the editor
//! appears when the page hydrates. Dispose the handle when the component
//! that made it goes away, on the server too: nothing else frees an editor.
//!
//! # Styling
//!
//! The editor looks like the text of its container: it reads the
//! container's font, size, weight, line height, text colour and caret
//! colour, and paints on the background colour behind it. Give the container
//! a width; the text wraps to its content box. With a height and
//! `overflow: auto` the container scrolls; without, it grows with the text
//! and the page scrolls.
//!
//! The canvas is opaque, so a background image or gradient behind the
//! editor does not show through it.
//!
//! Keyboard focus sits on the hidden text field, so style the container
//! with `:focus-within` to show a focus ring.
//!
//! # What people can do
//!
//! Type, with an input method too; press Enter for a new paragraph and
//! Shift+Enter for a line break; undo and redo; move and select with the
//! arrow keys, Home and End; select with the pointer, by word with a double
//! click and by paragraph with a triple click; and cut, copy and paste plain
//! text.
//!
//! # Accessibility
//!
//! The text field is announced as a multi-line text box, named by the
//! container's `aria-label` (or "Editor"), with its read-only state and
//! placeholder. Screen readers read the document from the hidden copy. They
//! cannot yet follow the caret through the text the way they do in a
//! `<textarea>`, which is a limit of every canvas editor.

mod device;
mod dom;
mod editor;
mod elements;
mod fonts;
mod host;
mod input;
mod lines;
mod measure;
mod mirror;
mod paint;
mod theme;

use web_sys::HtmlElement;

use crate::CanvistError;
use crate::CanvistState;
use crate::session::Action;

/// An editor on screen. Dropping it takes the editor off the page.
///
/// The document stays with the [`CanvistState`], so the same editor can be
/// attached again, in another element, with its text and undo history.
#[derive(Debug)]
#[must_use = "dropping an attachment takes the editor off the page"]
pub struct Attachment {
	state: CanvistState,
	serial: u64,
}

/// Put `state`'s editor in `container`.
///
/// The container should be in the page and empty. The editor adds its own
/// elements to it and removes them when the [`Attachment`] is dropped. If
/// the container is `position: static`, it becomes `position: relative`
/// while the editor is attached.
///
/// An editor is in one place at a time: attaching it again moves it.
///
/// # Errors
///
/// [`CanvistError::Disposed`] when the editor was disposed, and
/// [`CanvistError::Host`] when the browser has no 2D canvas to draw it on.
pub fn attach(container: &HtmlElement, state: CanvistState) -> Result<Attachment, CanvistError> {
	host::attach(container, state).map(|serial| Attachment { state, serial })
}

impl Attachment {
	/// The editor this shows.
	#[must_use]
	pub fn state(&self) -> CanvistState {
		self.state
	}

	/// Lay out and paint now, instead of on the next animation frame.
	///
	/// The editor paints by itself; this is for code that reads the canvas
	/// right after changing the editor, such as a test or a screenshot.
	pub fn render(&self) {
		host::with(self.state.key(), Some(self.serial), host::Host::render);
	}

	/// Read the container's styles again.
	///
	/// The editor does this by itself when it gains focus, when the
	/// container is resized, and when the system switches between light and
	/// dark. Call it after changing the container's classes or styles any
	/// other time.
	pub fn refresh_theme(&self) {
		host::with(
			self.state.key(),
			Some(self.serial),
			host::Host::refresh_theme,
		);
	}
}

impl Drop for Attachment {
	fn drop(&mut self) {
		host::detach(self.state.key(), Some(self.serial));
	}
}

/// The app replaced the document of the editor with `id`.
pub(crate) fn document_replaced(id: u32) {
	host::with(id, None, host::Host::document_replaced);
}

/// The app changed read-only or the placeholder of the editor with `id`.
pub(crate) fn options_changed(id: u32) {
	host::with(id, None, host::Host::apply_options);
}

/// Make a change to the editor with `id` through the host that shows it.
/// `None` when nothing shows it, and the change is the document's alone.
pub(crate) fn act(id: u32, action: Action) -> Option<Result<bool, CanvistError>> {
	let mut shown = None;
	host::with(id, None, |host| shown = Some(host.act(action)));

	shown
}

/// Move keyboard focus to the editor with `id`.
pub(crate) fn focus(id: u32) {
	host::with(id, None, |host| host.focus());
}

/// The editor with `id` was disposed.
pub(crate) fn disposed(id: u32) {
	host::detach(id, None);
}

/// Start putting editors in the `CanvistEditor` elements that name them.
pub(crate) fn watch_elements() {
	elements::watch();
}

pub(crate) use device::platform;
