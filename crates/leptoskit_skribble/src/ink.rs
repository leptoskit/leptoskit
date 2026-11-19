//! skribble ink as leptoskit vector drawings, which every backend draws
//! natively.
//!
//! [`skribble_core`] describes everything it draws as an [`InkDrawing`]: path
//! data with a light and a dark [`Tone`] per path. This module is the only
//! place that knows how that becomes leptoskit elements: each run of paths in
//! one tone is one `Vector` painting with `currentColor`, and the element's
//! `text-[…] dark:text-[…]` class picks the colour, so dark mode recolours the
//! ink on every platform without redrawing it.

use leptoskit::native::Color;
use leptoskit::native::Element;
use leptoskit::native::Paint;
use leptoskit::native::PathData;
use leptoskit::native::Stroke;
use leptoskit::native::VectorPath;
use leptoskit::native::VectorScaling;
use leptoskit::native::ViewBox;
use leptoskit::prelude::Vector;
use leptoskit::prelude::View;
use skribble_core::InkDrawing;
use skribble_core::InkPaint;
use skribble_core::InkPath;
use skribble_core::InkScaling;
use skribble_core::Tone;
use skribble_core::colorway::Rgba;

/// Covers the parent, so the drawing stretches with it.
const COVER: &str = "absolute top-0 left-0 w-full h-full";

/// `rgba`, a skribble colour, as a leptoskit colour.
///
/// ```
/// use leptoskit::native::Color;
/// use leptoskit_skribble::colorway::swatch;
/// use leptoskit_skribble::ink::color;
///
/// assert_eq!(color(swatch::TOMATO), Color::rgba(0xff, 0x6b, 0x4a, 0xff));
/// ```
pub const fn color(rgba: Rgba) -> Color {
	Color::rgba(rgba.red, rgba.green, rgba.blue, rgba.alpha)
}

/// A Tailwind utility for `tone`, such as `text-[#34283f]
/// dark:text-[#fffaf0]`, with the dark half only when the tone changes in
/// dark mode.
///
/// ```
/// use leptoskit_skribble::Theme;
/// use leptoskit_skribble::ink::tone_class;
///
/// assert_eq!(
/// 	tone_class("bg", Theme::SKRIBBLE.paper()),
/// 	"bg-[#fffaf0] dark:bg-[#292331]"
/// );
/// ```
pub fn tone_class(utility: &str, tone: Tone) -> String {
	let light = format!("{utility}-[{}]", tone.light.css_hex());
	if tone.is_fixed() {
		light
	} else {
		format!("{light} dark:{utility}-[{}]", tone.dark.css_hex())
	}
}

/// The vector elements that draw `drawing`, covering their parent.
///
/// Each run of paths in one tone becomes one `Vector` painting with
/// `currentColor`, so a `dark:` text colour recolours it on every platform.
/// The runs stack in order, so fills stay under outlines.
///
/// # Panics
///
/// Panics when the drawing's canvas is not positive and finite, which
/// skribble's recipes never produce.
pub fn vectors(drawing: &InkDrawing) -> Vec<Element> {
	let view_box = ViewBox::new(0.0, 0.0, drawing.width as f32, drawing.height as f32)
		.expect("ink is drawn on a positive, finite canvas");
	let scaling = match drawing.scaling {
		InkScaling::Fit => VectorScaling::Fit,
		InkScaling::Stretch => VectorScaling::Stretch,
	};
	drawing
		.runs()
		.map(|run| {
			run.paths.iter().fold(
				Vector(view_box)
					.scaling(scaling)
					.class(COVER)
					.class(tone_class("text", run.tone))
					.aria_hidden(true),
				|vector, path| vector.path(vector_path(path)),
			)
		})
		.collect()
}

/// One decorative element that draws `drawing` over its whole parent. Put it
/// first among the parent's children so content paints on top.
pub fn layer(drawing: &InkDrawing) -> Element {
	let mut vectors = vectors(drawing);
	if vectors.len() == 1 {
		return vectors.remove(0);
	}
	View().class(COVER).aria_hidden(true).children(vectors)
}

/// `path` as a vector path painted with the element's text colour.
fn vector_path(path: &InkPath) -> VectorPath {
	let data = PathData::parse(&path.data).expect("the rough engine writes valid path data");
	let vector = VectorPath::new(data);
	match path.paint {
		InkPaint::Fill(_) => vector.fill(Paint::CurrentColor),
		InkPaint::Stroke { width, .. } => {
			vector.stroke(
				Stroke::new(Paint::CurrentColor, width)
					.expect("skribble stroke widths are positive")
					.round(),
			)
		}
	}
}

/// `element` with the caller's extra classes, when there are any.
pub(crate) fn with_class(element: Element, class: Option<String>) -> Element {
	match class {
		Some(class) => element.class(class),
		None => element,
	}
}

#[cfg(test)]
mod tests {
	use leptoskit::native::NativeKind;
	use skribble_core::Canvas;
	use skribble_core::FillStyle;
	use skribble_core::Ink;
	use skribble_core::InkShape;
	use skribble_core::Theme;
	use skribble_core::colorway::swatch;

	use super::*;

	fn ink(shape: InkShape) -> Ink {
		Ink::outline(shape, Canvas::new(120.0, 40.0), 3, Theme::SKRIBBLE)
	}

	#[test]
	fn outlines_are_one_stretched_vector_in_the_ink_colour() {
		let layer = layer(&ink(InkShape::Rectangle).drawing());
		assert_eq!(layer.kind, NativeKind::Vector);
		let drawing = layer.props.drawing.as_ref().expect("a drawing");
		assert_eq!(drawing.vector_scaling(), VectorScaling::Stretch);
		assert_eq!(drawing.paths().len(), 1);
		let stroke = drawing.paths()[0].stroke_style().expect("an outline");
		assert_eq!(stroke.width(), Theme::SKRIBBLE.stroke_width);
		assert!(drawing.uses_current_color());
	}

	#[test]
	fn dark_mode_recolours_ink_through_the_text_colour() {
		let classes = tone_class("text", Theme::SKRIBBLE.ink());
		assert!(classes.contains("dark:text-"));
		let layer = layer(&ink(InkShape::Ellipse).drawing());
		for class in classes.split(' ') {
			assert!(
				layer.classes().classes().split(' ').any(|own| own == class),
				"{class}"
			);
		}
	}

	#[test]
	fn fills_paint_under_the_outline() {
		let theme = Theme::SKRIBBLE;
		let filled = ink(InkShape::Rectangle).filled(FillStyle::Solid, theme.accent());
		let vectors = vectors(&filled.drawing());
		assert_eq!(vectors.len(), 2);
		let fill = vectors[0].props.drawing.as_ref().expect("a fill");
		assert!(fill.paths()[0].fill_paint().is_some());
		let outline = vectors[1].props.drawing.as_ref().expect("an outline");
		assert!(outline.paths()[0].stroke_style().is_some());
		let layer = layer(&filled.drawing());
		assert_eq!(layer.children.len(), 2);
		assert_eq!(layer.props.aria().hidden, Some(true));
	}

	#[test]
	fn fitted_drawings_keep_their_proportions() {
		let mut drawing = InkDrawing::new(10.0, 10.0, InkScaling::Fit);
		drawing.push_fill("M0 0 L10 0 L10 10 Z".to_owned(), Theme::SKRIBBLE.marker());
		let vectors = vectors(&drawing);
		let vector = vectors[0].props.drawing.as_ref().expect("a drawing");
		assert_eq!(vector.vector_scaling(), VectorScaling::Fit);
	}

	#[test]
	fn fixed_tones_need_no_dark_class() {
		let fixed = ink(InkShape::Rule).outlined(Tone::fixed(swatch::INK));
		let layer = layer(&fixed.drawing());
		assert!(!layer.classes().classes().contains("dark:"));
		assert_eq!(
			tone_class("text", Tone::fixed(swatch::INK)),
			"text-[#34283f]"
		);
	}

	#[test]
	fn translucent_colours_keep_their_alpha() {
		assert_eq!(
			tone_class("bg", Tone::fixed(Rgba::new(1, 2, 3, 0x80))),
			"bg-[#01020380]"
		);
		assert_eq!(color(Rgba::new(1, 2, 3, 4)), Color::rgba(1, 2, 3, 4));
	}
}
