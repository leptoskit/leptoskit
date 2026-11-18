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
//! - **The web**, behind the `web` feature: [`web::attach`] puts the editor in
//!   an element of the page. The whole browser host is Rust compiled to Wasm,
//!   with no JavaScript of its own.
//! - **Everywhere else** the handle still loads, converts and exports
//!   documents, so a server can read what an editor saved, but nothing draws an
//!   editor yet.
//!
//! This first slice edits plain paragraphs. Formatting, lists, ink and the
//! native hosts come later, and so does the `leptoskit` element that mounts
//! an editor for you.
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
pub use state::Subscription;
