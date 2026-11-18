//! The canvist text editor for leptoskit on the web.
//!
//! `leptoskit_canvist` hosts a canvas-based text editor (porting canvist's
//! Rust engine) with no hand-written JavaScript. On the web it attaches to a
//! container element with [`web::attach`], building an opaque `<canvas>` and a
//! hidden `<textarea>` inside it.
//!
//! canvist's engine crates are pinned to a single git revision, and canvist's
//! types stay out of this crate's public API.

pub mod state;

#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub mod web;

#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub use web::attach;

#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub use web::Attachment;

pub use state::CanvistState;
pub use state::Change;
pub use state::Error;
pub use state::Extensions;
pub use state::Format;

pub use state::ErrorKind;