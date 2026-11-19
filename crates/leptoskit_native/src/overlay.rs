//! Content that sits above the screen: modals and popovers.
//!
//! A confirmation sheet, a "new day" dialog and an account menu have the
//! same needs. They sit above everything else, they are dismissed with
//! Escape or a press outside, and focus goes into them when they open and
//! back where it was when they close. Styling a `View` with `absolute` and a
//! high `z-index` gives none of that.
//!
//! [`Modal`](crate::components::Modal) blocks the screen behind it until it
//! is dismissed. [`Popover`](crate::components::Popover) hangs from another
//! element and leaves the rest of the screen usable. Both are their own
//! element kinds, because presenting one creates a different platform
//! object: a `<dialog>` in the browser's top layer, a sheet, a popup window.
//!
//! An overlay stays in the tree while closed and shows nothing. The app owns
//! whether it is open: a dismissal is a proposal, delivered to
//! [`on_dismiss`](crate::Element::on_dismiss), and the overlay closes when
//! the next build says so.
//!
//! ```
//! use leptoskit_native::Placement;
//! use leptoskit_native::components::Button;
//! use leptoskit_native::components::Modal;
//! use leptoskit_native::components::Popover;
//! use leptoskit_native::components::Text;
//! use leptoskit_native::components::View;
//!
//! let (confirming, menu_open) = (true, false);
//! let screen = View()
//! 	.child(Button("Account").id("me"))
//! 	.child(
//! 		Popover("me", menu_open)
//! 			.placement(Placement::BelowEnd)
//! 			.on_dismiss(|| {})
//! 			.child(Button("Sign out")),
//! 	)
//! 	.child(
//! 		Modal(confirming)
//! 			.aria_label("Delete the note?")
//! 			.initial_focus("keep")
//! 			.on_dismiss(|| {})
//! 			.child(Text("This cannot be undone."))
//! 			.child(Button("Keep it").id("keep")),
//! 	);
//!
//! assert!(screen.validate().is_ok());
//! ```

/// What an overlay element shows and where, set by the
/// [`Modal`](crate::components::Modal) and
/// [`Popover`](crate::components::Popover) constructors and the builders
/// beside them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Overlay {
	/// Whether the overlay is showing.
	pub open: bool,
	/// The `id` of the element a popover hangs from. A modal has none.
	#[cfg_attr(
		feature = "serde",
		serde(default, skip_serializing_if = "Option::is_none")
	)]
	pub anchor: Option<String>,
	/// Which side of its anchor a popover opens on.
	#[cfg_attr(
		feature = "serde",
		serde(default, skip_serializing_if = "Placement::is_default")
	)]
	pub placement: Placement,
	/// The `id` of the element inside the overlay that takes focus when it
	/// opens. Without one, the first focusable element inside does.
	#[cfg_attr(
		feature = "serde",
		serde(default, skip_serializing_if = "Option::is_none")
	)]
	pub initial_focus: Option<String>,
}

/// Which side of its anchor a popover opens on, and which edges line up.
///
/// "Start" and "end" follow the reading direction: start is the left edge
/// in a left-to-right layout. A popover that would leave the screen is moved
/// back inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(
	feature = "serde",
	derive(serde::Serialize, serde::Deserialize),
	serde(rename_all = "snake_case")
)]
pub enum Placement {
	/// Below the anchor, start edges aligned.
	#[default]
	BelowStart,
	/// Below the anchor, centred on it.
	Below,
	/// Below the anchor, end edges aligned: a menu under a button at the
	/// end of a toolbar.
	BelowEnd,
	/// Above the anchor, start edges aligned.
	AboveStart,
	/// Above the anchor, centred on it.
	Above,
	/// Above the anchor, end edges aligned.
	AboveEnd,
}

impl Placement {
	/// Every placement.
	pub const ALL: [Self; 6] = [
		Self::BelowStart,
		Self::Below,
		Self::BelowEnd,
		Self::AboveStart,
		Self::Above,
		Self::AboveEnd,
	];

	/// The wire name, as serialized and as the web's `data-lk-popover`
	/// attribute carries it.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::BelowStart => "below_start",
			Self::Below => "below",
			Self::BelowEnd => "below_end",
			Self::AboveStart => "above_start",
			Self::Above => "above",
			Self::AboveEnd => "above_end",
		}
	}

	/// The placement with this wire name, if there is one.
	#[must_use]
	pub fn from_name(name: &str) -> Option<Self> {
		Self::ALL
			.into_iter()
			.find(|placement| placement.as_str() == name)
	}

	/// The same placement in a right-to-left layout, where the start edge
	/// is the right one. [`Self::position`] works in left-to-right terms.
	#[must_use]
	pub const fn mirrored(self) -> Self {
		match self {
			Self::BelowStart => Self::BelowEnd,
			Self::BelowEnd => Self::BelowStart,
			Self::AboveStart => Self::AboveEnd,
			Self::AboveEnd => Self::AboveStart,
			centred @ (Self::Below | Self::Above) => centred,
		}
	}

	/// Whether the popover opens above its anchor.
	#[must_use]
	pub const fn is_above(self) -> bool {
		matches!(self, Self::AboveStart | Self::Above | Self::AboveEnd)
	}

	/// Where a popover of `size` goes for an anchor at `anchor`, as the
	/// top-left corner in the same coordinates, kept inside `viewport`.
	///
	/// `anchor` is `(left, top, width, height)`; `size` and `viewport` are
	/// `(width, height)`. A popover with no room on its side of the anchor
	/// flips to the other side when that side has room.
	#[must_use]
	pub fn position(
		self,
		anchor: (f32, f32, f32, f32),
		size: (f32, f32),
		viewport: (f32, f32),
	) -> (f32, f32) {
		let (left, top, width, height) = anchor;
		let x = match self {
			Self::BelowStart | Self::AboveStart => left,
			Self::Below | Self::Above => left + (width - size.0) / 2.0,
			Self::BelowEnd | Self::AboveEnd => left + width - size.0,
		};
		let (above, below) = (top - size.1, top + height);
		let fits_above = above >= 0.0;
		let fits_below = below + size.1 <= viewport.1;
		let y = match (self.is_above(), fits_above, fits_below) {
			(true, false, true) | (false, _, true) | (false, false, false) => below,
			(true, ..) | (false, true, false) => above,
		};

		(
			x.min(viewport.0 - size.0).max(0.0),
			y.min(viewport.1 - size.1).max(0.0),
		)
	}

	#[cfg(feature = "serde")]
	#[allow(
		clippy::trivially_copy_pass_by_ref,
		reason = "serde passes a reference"
	)]
	fn is_default(&self) -> bool {
		*self == Self::default()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const VIEWPORT: (f32, f32) = (400.0, 300.0);

	#[test]
	fn a_popover_lines_up_with_the_edge_its_placement_names() {
		let anchor = (100.0, 50.0, 80.0, 20.0);
		let at = |placement: Placement| placement.position(anchor, (40.0, 30.0), VIEWPORT);

		assert_eq!(at(Placement::BelowStart), (100.0, 70.0));
		assert_eq!(at(Placement::Below), (120.0, 70.0));
		assert_eq!(at(Placement::BelowEnd), (140.0, 70.0));
		assert_eq!(at(Placement::AboveStart), (100.0, 20.0));
		assert_eq!(at(Placement::Above), (120.0, 20.0));
		assert_eq!(at(Placement::AboveEnd), (140.0, 20.0));
	}

	#[test]
	fn a_popover_with_no_room_flips_and_stays_on_screen() {
		let size = (120.0, 100.0);
		// No room below: it opens above.
		let low = (10.0, 260.0, 60.0, 20.0);
		assert_eq!(
			Placement::BelowStart.position(low, size, VIEWPORT),
			(10.0, 160.0)
		);
		// No room above: it opens below.
		let high = (10.0, 20.0, 60.0, 20.0);
		assert_eq!(
			Placement::AboveStart.position(high, size, VIEWPORT),
			(10.0, 40.0)
		);
		// Past the end edge: it is pulled back inside.
		let end = (380.0, 20.0, 20.0, 20.0);
		assert_eq!(
			Placement::BelowStart.position(end, size, VIEWPORT),
			(280.0, 40.0)
		);
		// Past the start edge.
		let start = (0.0, 20.0, 20.0, 20.0);
		assert_eq!(
			Placement::BelowEnd.position(start, size, VIEWPORT),
			(0.0, 40.0)
		);
		// Room on neither side: it keeps its side and stays inside.
		let middle = (10.0, 140.0, 60.0, 20.0);
		assert_eq!(
			Placement::BelowStart.position(middle, (120.0, 200.0), VIEWPORT),
			(10.0, 100.0)
		);
	}
}
