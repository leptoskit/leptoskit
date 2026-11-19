//! Freehand scribbles at an explicit size.

use leptoskit::native::Element;
use leptoskit::native::Role;
use leptoskit::prelude::*;
use leptoskit::yoga::Dimension;
use skribble_core::FillStyle;
use skribble_core::RoughPen;
use skribble_core::Scribble;
use skribble_core::Shape;

use crate::Theme;
use crate::Tone;
use crate::ink::vectors;
use crate::ink::with_class;

/// A hand-drawn doodle: arrows, scribbles, and annotations the other
/// components do not cover.
///
/// The shapes are drawn in the sketch's own coordinates, `width` by `height`
/// logical pixels, with the pen's tapered ink. Colours default to the theme,
/// including in dark mode.
///
/// A sketch is decorative and hidden from assistive technology unless it has
/// a `label`, which makes it an image with that name.
///
/// # Panics
///
/// Panics when `width` or `height` is not a positive, finite number, or when a
/// shape fails [`Shape::validate`]: a NaN or infinite coordinate, an arc that
/// sweeps nothing, or a curve through fewer than two points. These are
/// mistakes in the drawing rather than something to recover from.
///
/// ```
/// use leptoskit::prelude::*;
/// use leptoskit_skribble::Shape;
/// use leptoskit_skribble::Sketch;
///
/// let arrow = view! {
/// 	<Sketch
/// 		width=120.0
/// 		height=60.0
/// 		shapes=vec![Shape::curve([(4.0, 50.0), (40.0, 10.0), (80.0, 40.0), (116.0, 12.0)]).with_arrowhead()]
/// 		label="Points to the sign-up form"
/// 	/>
/// };
/// assert_eq!(arrow.props.accessibility_label.as_deref(), Some("Points to the sign-up form"));
/// ```
#[component]
pub fn Sketch(
	/// Width, in logical pixels and in the shapes' coordinates.
	width: f32,
	/// Height, in logical pixels and in the shapes' coordinates.
	height: f32,
	/// What to draw, in order, from one random stream.
	shapes: Vec<Shape>,
	/// Names the drawing for assistive technology. Without one the sketch is
	/// decorative.
	#[prop(optional, into)]
	label: Option<String>,
	/// Chooses the wobble. The same seed always draws the same doodle.
	#[prop(default = 1)]
	seed: i64,
	/// How ink leaves the pen.
	#[prop(default = RoughPen::INK)]
	pen: RoughPen,
	/// The fill pattern for closed shapes. Defaults to hachure when `fill` is
	/// set, and to no fill otherwise.
	#[prop(optional)]
	fill_style: Option<FillStyle>,
	/// The fill colour. Defaults to the theme's marker.
	#[prop(optional, into)]
	fill: Option<Tone>,
	/// The ink colour. Defaults to the theme's ink.
	#[prop(optional, into)]
	stroke: Option<Tone>,
	/// The colours, line weight, and wobble to draw with.
	#[prop(default = Theme::SKRIBBLE)]
	theme: Theme,
	/// Extra Tailwind classes.
	#[prop(optional, into)]
	class: Option<String>,
) -> Element {
	let scribble = Scribble {
		width,
		height,
		shapes,
		seed,
		pen,
		fill_style: fill_style.unwrap_or(if fill.is_some() {
			FillStyle::Hachure
		} else {
			FillStyle::None
		}),
		fill: fill.unwrap_or(theme.marker()),
		stroke: stroke.unwrap_or(theme.ink()),
		theme,
	};
	let drawing = scribble.drawing().unwrap_or_else(|error| panic!("{error}"));

	let element = View().children(vectors(&drawing));
	let element = match label {
		Some(label) => element.role(Role::Img).aria_label(label),
		None => element.aria_hidden(true),
	};
	let element = element.with_style(|style| {
		style.width = Dimension::Points(width);
		style.height = Dimension::Points(height);
	});
	with_class(element, class)
}
