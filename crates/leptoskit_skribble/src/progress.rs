//! The hand-drawn progress bar.

use leptoskit::native::Element;
use leptoskit::native::Role;
use leptoskit::prelude::*;
use leptoskit::yoga::Dimension;
use skribble_core::components;

use crate::Theme;
use crate::ink::layer;
use crate::ink::with_class;

/// A progress bar drawn by hand: a rough track that fills with hachure.
///
/// It is a progress bar for assistive technology, which reads the label and
/// the percentage.
///
/// ```
/// use leptoskit::prelude::*;
/// use leptoskit_skribble::WiredProgress;
///
/// let upload = view! { <WiredProgress label="Uploading" value=0.4 /> };
/// assert_eq!(upload.children.len(), 2);
/// ```
#[component]
pub fn WiredProgress(
	/// Names the task for assistive technology.
	#[prop(into)]
	label: String,
	/// How much is done, from 0 to 1. Values outside are clamped.
	value: f64,
	/// Chooses the wobble. The same seed always draws the same bar.
	#[prop(default = 1)]
	seed: i64,
	/// The colours to draw with.
	#[prop(default = Theme::SKRIBBLE)]
	theme: Theme,
	/// Extra Tailwind classes, such as a width.
	#[prop(optional, into)]
	class: Option<String>,
) -> Element {
	let ink = components::progress(value, seed, theme);
	let done = ink.done;
	// The filled part draws on a canvas as wide as its share of the track, so
	// the hachure keeps one density at every value instead of squeezing up.
	let filling = ink.filling.map(|filling| {
		View()
			.class("absolute top-0 left-0 h-full")
			.with_style(|style| style.width = Dimension::Percent(done as f32))
			.child(layer(&filling.drawing()))
	});

	// Screen readers announce the value as a percentage of the range.
	let bar = View()
		.role(Role::ProgressBar)
		.aria_label(label)
		.aria_value_min(0.0)
		.aria_value_max(100.0)
		.aria_value_now(ink.percent)
		.class("w-full h-6")
		.children(filling)
		.child(layer(&ink.track.drawing()));
	with_class(bar, class)
}
