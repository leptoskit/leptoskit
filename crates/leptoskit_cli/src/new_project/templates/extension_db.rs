//! The browser extension: the popup opens the home screen, and the
//! background worker answers messages from it.
//!
//! `[extension]` in `leptoskit.toml` lists the surfaces; each opens one of the
//! app's routes. `leptoskit build --platform extension` packages them for
//! Chromium and Firefox.

use leptoskit::extension::Background;
use leptoskit::extension::Surface;
use leptoskit::prelude::Element;

/// Renders a surface. This app has one screen, so every surface shows it.
pub fn main(_surface: Surface) -> impl Fn() -> Element + Send + Sync + 'static {
	let state = crate::provide();

	move || crate::screen(&state)
}

/// Registers the worker's handlers. Any surface can ask it something with
/// `leptoskit::extension::runtime::send`.
pub fn background(worker: &Background) {
	worker.on_message(
		|message: String, _sender| async move { format!("The worker heard: {message}") },
	);
}
