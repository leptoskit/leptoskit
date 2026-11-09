//! Keep a launch overlay until the app is ready — Flutter's
//! `FlutterNativeSplash.preserve` / `remove`, for leptoskit.
//!
//! Configure the native / HTML splash in `[app.launch_screen]` (or
//! `[app.splash]`). When `[app.launch_screen.animated]` sets `until =
//! "manual"`, call [`hold`] before the first screen and [`dismiss`] once
//! initialization finishes. With `until = "first_frame"` (the default), the
//! OS splash ends on its own, and on the web the renderer lifts the HTML
//! splash once its first render is painted; these calls are no-ops unless
//! you held.
//!
//! ```ignore
//! use leptoskit::splash;
//!
//! fn main() {
//!     splash::hold();
//!     // load session, plugins, …
//!     splash::dismiss();
//! }
//! ```

use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

/// The id of the HTML launch screen `[app.launch_screen]` puts in a web
/// app's `index.html`. The CLI writes the element and the renderer lifts it:
/// the CLI keeps a copy of these names, which its tests hold to these.
pub const HTML_ID: &str = "leptoskit-splash";

/// The class the HTML launch screen's fade-out adds before it removes the
/// element.
pub const HTML_HIDE_CLASS: &str = "leptoskit-splash-hide";

/// The page function that fades the HTML launch screen out:
/// `window.leptoskitDismissSplash()`, which the CLI defines and the renderer
/// calls. wasm-bindgen takes the name the renderer imports as a literal, so a
/// test holds that literal to this constant.
pub const HTML_DISMISS: &str = "leptoskitDismissSplash";

static HELD: AtomicBool = AtomicBool::new(false);
static DISMISSED: AtomicBool = AtomicBool::new(false);

/// Keep the splash overlay visible after the OS / HTML splash would end.
pub fn hold() {
	HELD.store(true, Ordering::SeqCst);
	DISMISSED.store(false, Ordering::SeqCst);
}

/// Remove the splash overlay. Safe to call more than once.
///
/// On the web this also lifts the HTML splash `[app.launch_screen]` injects
/// ([`HTML_ID`]), through its `window.leptoskitDismissSplash()`
/// ([`HTML_DISMISS`]).
pub fn dismiss() {
	DISMISSED.store(true, Ordering::SeqCst);
	HELD.store(false, Ordering::SeqCst);
	#[cfg(feature = "web")]
	crate::web::lift_splash();
}

/// Whether [`hold`] is active and [`dismiss`] has not run yet.
#[must_use]
pub fn is_held() -> bool {
	HELD.load(Ordering::SeqCst) && !DISMISSED.load(Ordering::SeqCst)
}

/// Whether the overlay should still be drawn (held, or not yet dismissed after
/// hold).
#[must_use]
pub fn visible() -> bool {
	is_held()
}

/// A full-bleed splash overlay for apps that compose their own animated
/// content (for example a Rive scene) instead of relying only on the HTML
/// helper.
///
/// Prefer [`hold`] / [`dismiss`] for the common case. Use this when the
/// overlay itself is part of the component tree.
#[allow(non_snake_case)]
#[must_use]
pub fn Splash() -> crate::Element {
	crate::components::View()
		.class("absolute inset-0 items-center justify-center")
		.test_id("leptoskit-splash")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn hold_then_dismiss_clears_visibility() {
		dismiss();
		assert!(!visible());
		hold();
		assert!(visible());
		assert!(is_held());
		dismiss();
		assert!(!visible());
		assert!(!is_held());
		dismiss();
		assert!(!visible());
	}

	/// The renderer imports the page's fade-out by a literal name, which
	/// wasm-bindgen needs; it must be the one the CLI defines.
	#[test]
	fn the_renderer_imports_the_fade_out_the_cli_defines() {
		let renderer = include_str!("web/mod.rs");
		assert!(
			renderer.contains(&format!("js_name = {HTML_DISMISS},")),
			"web/mod.rs must import window.{HTML_DISMISS}"
		);
		assert!(
			renderer.contains("crate::splash::HTML_ID"),
			"web/mod.rs must look the splash up by splash::HTML_ID"
		);
	}
}
