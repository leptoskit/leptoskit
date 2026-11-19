//! Scrolling an element into view once.
//!
//! [`Element::scroll_offset`](crate::Element::scroll_offset) scrolls a
//! `ScrollView` to a number. That suits a list that knows its own row
//! heights. It does not suit "show the notice that just appeared" or "bring
//! the Sign in button into view": the screen knows which element it means,
//! not where layout put it.
//!
//! A [`Reveal`] scrolls one element into view once per call to
//! [`request`](Reveal::request):
//!
//! ```
//! use leptoskit_native::Element;
//! use leptoskit_native::Reveal;
//! use leptoskit_native::ScrollAlign;
//! use leptoskit_native::components::Button;
//! use leptoskit_native::components::ScrollView;
//! use leptoskit_native::components::Text;
//!
//! fn inbox() -> Element {
//! 	let to_notice = Reveal::new();
//!
//! 	ScrollView()
//! 		.child(Button("Save").on_press(move || to_notice.request()))
//! 		.child(Text("Saved.").reveal_on(&to_notice, ScrollAlign::Nearest))
//! }
//! # reactive_graph::owner::Owner::new().with(inbox);
//! ```

use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use reactive_graph::owner::Owner;
use reactive_graph::signal::RwSignal;
use reactive_graph::traits::Get;
use reactive_graph::traits::Set;

use crate::remember::remember;

/// Where a revealed element ends up in the scroll view that holds it, along
/// the direction the view scrolls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum ScrollAlign {
	/// The element's start edge at the view's start edge.
	Start,
	/// The element in the middle of the view.
	Center,
	/// The element's end edge at the view's end edge.
	End,
	/// The least scrolling that shows the element, and none when it is
	/// already in view.
	#[default]
	Nearest,
}

impl ScrollAlign {
	/// The name CSS gives this alignment, as `scrollIntoView` takes it.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Start => "start",
			Self::Center => "center",
			Self::End => "end",
			Self::Nearest => "nearest",
		}
	}

	/// The scroll offset along one axis that puts an element `size` long
	/// at `position` in the content where this alignment asks, in a view
	/// `view` long that is scrolled to `current`. Never negative.
	#[must_use]
	pub fn offset(self, position: f32, size: f32, view: f32, current: f32) -> f32 {
		let end = position + size - view;
		let offset = match self {
			Self::Start => position,
			Self::Center => position - (view - size) / 2.0,
			Self::End => end,
			// An element longer than the view shows its start.
			Self::Nearest if position < current || size > view => position,
			Self::Nearest if end > current => end,
			Self::Nearest => current,
		};

		offset.max(0.0)
	}
}

/// One request to scroll an element into view, as the element carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RevealRequest {
	/// The request's serial. Serials grow with every request in the
	/// process, so a renderer that remembers the newest one it carried out
	/// can tell a new request from one it has already answered.
	pub serial: u64,
	/// Where the element ends up in its scroll view.
	pub align: ScrollAlign,
}

/// A handle that scrolls one element into view, once per request.
///
/// Create it in the screen or component that owns the element, give it to
/// the element with [`Element::reveal_on`](crate::Element::reveal_on), and
/// call [`request`](Self::request) from any handler. It is `Copy`, so
/// handlers can capture it freely.
///
/// The handle is [remembered](crate::remember), so the same one comes back
/// each time the screen builds, and the screen builds again when a reveal
/// is requested.
///
/// A screen built outside a running app, as a test that builds one tree and
/// inspects it does, has nowhere to keep the request. The handle is inert
/// there: [`request`](Self::request) does nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reveal {
	/// The serial of the latest request, or zero before the first. `None`
	/// outside a reactive owner, where no signal can live.
	latest: Option<RwSignal<u64>>,
}

/// The serial the next request takes, unique across every handle.
static NEXT: AtomicU64 = AtomicU64::new(1);

impl Reveal {
	/// A request nobody has made yet.
	#[must_use]
	#[track_caller]
	pub fn new() -> Self {
		Self {
			latest: Owner::current().map(|_| remember(|| 0)),
		}
	}

	/// Scrolls the element this request was given to into view, once, after
	/// the next commit.
	pub fn request(&self) {
		if let Some(latest) = self.latest {
			latest.set(NEXT.fetch_add(1, Ordering::Relaxed));
		}
	}

	/// The serial of the latest request, or `None` before the first. Reading
	/// it subscribes the build, so a request rebuilds the screen.
	pub(crate) fn serial(&self) -> Option<u64> {
		let serial = self.latest?.get();

		(serial > 0).then_some(serial)
	}
}

impl Default for Reveal {
	#[track_caller]
	fn default() -> Self {
		Self::new()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::remember::ScreenState;

	#[test]
	fn a_request_survives_a_rebuild_and_each_one_is_new() {
		Owner::new().with(|| {
			let state = ScreenState::new();
			state.provide();
			let first = state.build(Reveal::new);
			assert_eq!(first.serial(), None);

			first.request();
			let again = state.build(Reveal::new);
			assert_eq!(again, first);
			let requested = again.serial().expect("a request was made");

			again.request();
			assert!(first.serial().expect("a second request") > requested);
		});
	}

	#[test]
	fn a_request_outside_a_running_app_does_nothing() {
		let reveal = Reveal::new();

		reveal.request();
		assert_eq!(reveal.serial(), None);
	}

	#[test]
	fn each_alignment_places_the_element() {
		// A 20-long element at 300 in a 100-long view.
		assert_eq!(ScrollAlign::Start.offset(300.0, 20.0, 100.0, 0.0), 300.0);
		assert_eq!(ScrollAlign::Center.offset(300.0, 20.0, 100.0, 0.0), 260.0);
		assert_eq!(ScrollAlign::End.offset(300.0, 20.0, 100.0, 0.0), 220.0);
		// Near the top there is nothing above to scroll to.
		assert_eq!(ScrollAlign::Center.offset(10.0, 20.0, 100.0, 50.0), 0.0);
	}

	#[test]
	fn nearest_scrolls_only_as_far_as_it_must() {
		// Below the view: its end edge comes to the view's end.
		assert_eq!(ScrollAlign::Nearest.offset(300.0, 20.0, 100.0, 0.0), 220.0);
		// Above the view: its start edge comes to the view's start.
		assert_eq!(
			ScrollAlign::Nearest.offset(300.0, 20.0, 100.0, 400.0),
			300.0
		);
		// In view: nothing moves.
		assert_eq!(
			ScrollAlign::Nearest.offset(300.0, 20.0, 100.0, 250.0),
			250.0
		);
		// Longer than the view: its start shows.
		assert_eq!(ScrollAlign::Nearest.offset(300.0, 200.0, 100.0, 0.0), 300.0);
	}
}
