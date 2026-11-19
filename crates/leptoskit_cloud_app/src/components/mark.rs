//! The mark and the line icons, drawn as views and vectors (the admin's
//! `mark` and `icon`, with `--admin-` renamed to `--cloud-`).

use leptoskit::native::Paint;
use leptoskit::native::PathData;
use leptoskit::native::Stroke;
use leptoskit::native::VectorPath;
use leptoskit::native::ViewBox;
use leptoskit::prelude::*;

/// An icon, drawn as strokes in the current text colour. `path` is SVG path
/// data in a 24-unit box; `|` separates strokes. A part that doesn't parse is
/// left out, which the icon tests rule out for every constant here.
pub fn icon(path: &str) -> Element {
	let mut vector = match ViewBox::new(0.0, 0.0, 24.0, 24.0) {
		Ok(view_box) => Vector(view_box),
		Err(_) => return View().class("h-[18px] w-[18px] shrink-0"),
	}
	.class("h-[18px] w-[18px] shrink-0")
	.aria_hidden(true);

	for part in path.split('|') {
		if let (Ok(data), Ok(stroke)) = (
			PathData::parse(part),
			Stroke::new(Paint::CurrentColor, 1.75),
		) {
			vector = vector.path(VectorPath::new(data).stroke(stroke.round()));
		}
	}

	vector
}

/// Projects: four boxes.
pub const ICON_APPS: &str = "M4 4h6v6H4z|M14 4h6v6h-6z|M4 14h6v6H4z|M14 14h6v6h-6z";
/// Releases: a sheet with lines.
pub const ICON_RELEASES: &str = "M6 3h12v18H6z|M9 8h6|M9 12h6|M9 16h3";
/// To do: a tray.
pub const ICON_INBOX: &str = "M4 13l3-8h10l3 8|M4 13v6h16v-6|M4 13h5l1 2h4l1-2h5";
/// You: a person.
pub const ICON_ACCOUNT: &str = "M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8z|M4 21c0-4 4-6 8-6s8 2 8 6";
/// Filter: a lens.
pub const ICON_SEARCH: &str = "M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14z|M20 20l-4-4";
/// New: a plus.
pub const ICON_PLUS: &str = "M12 5v14|M5 12h14";
/// Switch: two arrows.
pub const ICON_SWITCH: &str = "M7 7h12l-3-3|M17 17H5l3 3";
/// Billing: a card.
pub const ICON_BILLING: &str = "M3 6h18v12H3z|M3 10h18|M7 15h4";

/// Every icon, for the test that checks each one parses.
#[cfg(test)]
const ICONS: &[&str] = &[
	ICON_APPS,
	ICON_RELEASES,
	ICON_INBOX,
	ICON_ACCOUNT,
	ICON_SEARCH,
	ICON_PLUS,
	ICON_SWITCH,
];

/// The leptoskit mark: two ember boxes and the mint piece that arrives over
/// the air, tilted −12°.
pub fn mark(size: f32) -> Element {
	let unit = size / 28.0;
	let px = |value: f32| format!("{:.1}px", value * unit);
	View()
		.class("relative shrink-0")
		.class(format!("h-[{}] w-[{}]", px(28.0), px(28.0)))
		.aria_hidden(true)
		.child(
			View()
				.class("absolute rounded-[3px] bg-[var(--cloud-ember)]")
				.class(format!(
					"left-0 top-[{}] h-[{}] w-[{}]",
					px(4.0),
					px(24.0),
					px(11.0)
				)),
		)
		.child(
			View()
				.class("absolute rounded-[3px] bg-[var(--cloud-ember-warm)]")
				.class(format!(
					"left-[{}] top-[{}] h-[{}] w-[{}]",
					px(14.0),
					px(17.0),
					px(11.0),
					px(11.0)
				)),
		)
		.child(
			View()
				.class("absolute rounded-[3px] bg-[var(--cloud-mint)] -rotate-12")
				.class(format!(
					"left-[{}] top-0 h-[{}] w-[{}]",
					px(15.0),
					px(11.0),
					px(11.0)
				)),
		)
}

#[cfg(test)]
mod tests {
	use leptoskit::native::PathData;

	use super::ICONS;

	#[test]
	fn every_icon_parses() {
		for icon in ICONS {
			for part in icon.split('|') {
				assert!(PathData::parse(part).is_ok(), "{part}");
			}
		}
	}
}
