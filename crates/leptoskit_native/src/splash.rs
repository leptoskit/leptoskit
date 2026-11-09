//! Keep a launch overlay until the app is ready — Flutter's
//! `FlutterNativeSplash.preserve` / `remove`, for leptoskit.
//!
//! Configure the native / HTML splash in `[app.launch_screen]` (or
//! `[app.splash]`). When `[app.launch_screen.animated]` sets `until =
//! "manual"`, call [`hold`] before the first screen and [`dismiss`] once
//! initialization finishes. With `until = "first_frame"` (the default), the
//! OS splash ends on its own and these calls are no-ops unless you held.
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

static HELD: AtomicBool = AtomicBool::new(false);
static DISMISSED: AtomicBool = AtomicBool::new(false);

/// Keep the splash overlay visible after the OS / HTML splash would end.
pub fn hold() {
	HELD.store(true, Ordering::SeqCst);
	DISMISSED.store(false, Ordering::SeqCst);
}

/// Remove the splash overlay. Safe to call more than once.
///
/// On the web, also call the injected `window.leptoskitDismissSplash()` from
/// `[app.launch_screen]` so the HTML splash node is removed.
pub fn dismiss() {
	DISMISSED.store(true, Ordering::SeqCst);
	HELD.store(false, Ordering::SeqCst);
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
}
