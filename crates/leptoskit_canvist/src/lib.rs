//! The canvist text editor for leptoskit.
//!
//! [canvist](https://github.com/canvist/canvist) is a text editor that draws
//! its own text on a canvas instead of handing it to `contenteditable`. This
//! crate hosts it: an app owns an editor through a [`CanvistState`] handle,
//! loads a document into it, hears about changes, and exports the document
//! when it is time to save.
//!
//! ```
//! use leptoskit_canvist::CanvistState;
//! use leptoskit_canvist::Extensions;
//! use leptoskit_canvist::Format;
//!
//! let note = CanvistState::new(Extensions::paragraphs());
//! note.load(Format::Markdown, "Hello\n\nWorld")?;
//! note.on_change(|change| {
//! 	if change.doc_changed {
//! 		// Mark the note as unsaved.
//! 	}
//! });
//! # note.dispose();
//! # Ok::<(), leptoskit_canvist::CanvistError>(())
//! ```
//!
//! # Where it runs
//!
//! - **The web**, behind the `web` feature. With leptoskit, give the editor's
//!   [`CanvistState::id`] to a `CanvistEditor` element and the editor appears
//!   in it. A page built some other way puts the editor in an element with
//!   [`web::attach`]. Either way the whole browser host is Rust compiled to
//!   Wasm, with no JavaScript of its own.
//! - **Everywhere else** the handle still loads, converts and exports
//!   documents, so a server can read what an editor saved, but nothing draws an
//!   editor yet: a `CanvistEditor` element is an empty view.
//!
//! The editor holds plain paragraphs, and with the `ink` feature drawings
//! between them. Formatting, lists and the native hosts come later.
//!
//! # Drawings
//!
//! Some things are quicker to draw than to describe. With the `ink` feature,
//! an editor made with `Extensions::paragraphs().ink()` holds drawings in the
//! flow of its text: `insert_drawing()` puts a sheet where the caret is, and
//! on the web a mouse, a pen or a finger draws on it in the `Pen` the app
//! chose. Each stroke is one change and one step of undo, and the drawings
//! are saved with the text in JSON and Markdown.
//!
//! ```
//! # #[cfg(feature = "ink")]
//! # {
//! use leptoskit_canvist::CanvistState;
//! use leptoskit_canvist::Extensions;
//! use leptoskit_canvist::Pen;
//!
//! let page = CanvistState::new(Extensions::paragraphs().ink());
//! page.set_pen(Pen::new().color(0xe1, 0x1d, 0x48).width(12.0));
//! assert_eq!(page.insert_drawing(), Ok(true));
//! assert!(page.undo());
//! # page.dispose();
//! # }
//! ```
//!
//!
//! canvist's own types stay out of this crate's API: canvist is still moving
//! fast, and its releases should not break apps that only hold a handle.

mod session;
mod state;
#[cfg(any(test, all(feature = "web", target_arch = "wasm32")))]
mod text;

#[cfg(all(feature = "web", target_arch = "wasm32"))]
pub mod web;

pub use state::CanvistError;
pub use state::CanvistState;
pub use state::Change;
pub use state::Extensions;
pub use state::Format;
#[cfg(feature = "ink")]
pub use state::Pen;
pub use state::Subscription;
