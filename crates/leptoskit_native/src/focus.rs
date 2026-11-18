//! Moving keyboard focus once.
//!
//! [`Element::focused`](crate::Element::focused) holds focus on a widget for
//! as long as it is true: after every commit, focus goes back there. That
//! suits a chat prompt. It does not suit focus that should move once, such
//! as landing in the code field after a code is sent, or going back to the
//! button that opened a menu: the next unrelated commit would pull focus
//! back after the person has moved on.
//!
//! A [`FocusRequest`] moves focus once per call to
//! [`request`](FocusRequest::request):
//!
//! ```
//! use leptoskit_native::Element;
//! use leptoskit_native::FocusRequest;
//! use leptoskit_native::components::Button;
//! use leptoskit_native::components::TextInput;
//! use leptoskit_native::components::View;
//!
//! fn sign_in() -> Element {
//! 	let to_code = FocusRequest::new();
//!
//! 	View()
//! 		.child(Button("Send code").on_press(move || to_code.request()))
//! 		.child(TextInput().aria_label("Code").focus_on(&to_code))
//! }
//! # reactive_graph::owner::Owner::new().with(sign_in);
//! ```

use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use reactive_graph::signal::RwSignal;
use reactive_graph::traits::Get;
use reactive_graph::traits::Set;

use crate::remember::remember;

/// A handle that moves keyboard focus to one widget, once per request.
///
/// Create it in the screen or component that owns the widget, give it to the
/// widget with [`Element::focus_on`](crate::Element::focus_on), and call
/// [`request`](Self::request) from any handler. It is `Copy`, so handlers can
/// capture it freely.
///
/// The handle is [remembered](crate::remember), so the same one comes back
/// each time the screen builds, and the screen builds again when focus is
/// requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FocusRequest {
	/// The serial of the latest request, or zero before the first.
	latest: RwSignal<u64>,
}

/// The serial the next request takes. Serials are unique across every
/// handle, so a renderer can tell a request it has already carried out from
/// a new one without knowing which handle made it.
static NEXT: AtomicU64 = AtomicU64::new(1);

impl FocusRequest {
	/// A request nobody has made yet.
	#[must_use]
	#[track_caller]
	pub fn new() -> Self {
		Self {
			latest: remember(|| 0),
		}
	}

	/// Moves focus to the widget this request was given to, once, after the
	/// next commit.
	pub fn request(&self) {
		self.latest.set(NEXT.fetch_add(1, Ordering::Relaxed));
	}

	/// The serial of the latest request, or `None` before the first. Reading
	/// it subscribes the build, so a request rebuilds the screen.
	pub(crate) fn serial(&self) -> Option<u64> {
		Some(self.latest.get()).filter(|&serial| serial > 0)
	}
}

impl Default for FocusRequest {
	#[track_caller]
	fn default() -> Self {
		Self::new()
	}
}

#[cfg(test)]
mod tests {
	use reactive_graph::owner::Owner;

	use super::*;
	use crate::remember::ScreenState;

	#[test]
	fn a_request_survives_a_rebuild_and_each_one_is_new() {
		Owner::new().with(|| {
			let state = ScreenState::new();
			state.provide();
			let first = state.build(FocusRequest::new);
			assert_eq!(first.serial(), None);

			first.request();
			let again = state.build(FocusRequest::new);
			assert_eq!(again, first);
			let requested = again.serial().expect("a request was made");

			again.request();
			assert!(first.serial().expect("a second request") > requested);
		});
	}

	#[test]
	fn two_requests_in_one_build_are_separate() {
		Owner::new().with(|| {
			let state = ScreenState::new();
			state.provide();
			let (menu, code) = state.build(|| (FocusRequest::new(), FocusRequest::new()));

			menu.request();
			assert!(menu.serial().is_some());
			assert_eq!(code.serial(), None);
		});
	}
}
