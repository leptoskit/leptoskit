//! Pen marks around text: underlines, circles, boxes, strikes, and markers.

use leptoskit::native::Element;
use leptoskit::prelude::*;
use skribble_core::components;
use skribble_core::components::HighlightKind;

use crate::Theme;
use crate::Tone;
use crate::ink::layer;
use crate::ink::tone_class;
use crate::ink::with_class;

/// Marks text the way a pen would: underlined, circled, boxed, struck
/// through, or highlighted.
///
/// The mark is decorative, so it adds nothing for screen readers. When it
/// carries meaning, such as a struck-out price, say so in the text.
///
/// ```
/// use leptoskit::prelude::*;
/// use leptoskit_skribble::HighlightKind;
/// use leptoskit_skribble::WiredHighlight;
///
/// let title = view! {
/// 	<View class="flex-row items-center gap-1">
/// 		<Text>"Draw it"</Text>
/// 		<WiredHighlight kind=HighlightKind::Circle seed=5>
/// 			<Text>"differently"</Text>
/// 		</WiredHighlight>
/// 	</View>
/// };
/// assert_eq!(title.text_content(), "Draw it differently");
/// ```
#[component]
pub fn WiredHighlight(
	/// The mark to draw.
	#[prop(optional)]
	kind: HighlightKind,
	/// The mark's colour. Defaults to the ink for pen marks and the theme's
	/// marker for [`HighlightKind::Marker`]; pass `theme.accent_at(i)` for a
	/// run of colourful highlights.
	#[prop(optional, into)]
	color: Option<Tone>,
	/// Chooses the wobble. The same seed always draws the same mark.
	#[prop(default = 1)]
	seed: i64,
	/// The colours to draw with.
	#[prop(default = Theme::SKRIBBLE)]
	theme: Theme,
	/// Extra Tailwind classes.
	#[prop(optional, into)]
	class: Option<String>,
	/// The marked text.
	children: Children,
) -> Element {
	let mark = components::highlight(kind, color, seed, theme);
	let padding = match kind {
		HighlightKind::Underline => "pb-1.5",
		HighlightKind::Strikethrough => "px-0.5",
		// Enough room that the corners of the text stay inside the ellipse.
		HighlightKind::Circle => "px-4 py-2.5",
		HighlightKind::Box => "px-1.5 py-0.5",
		HighlightKind::Marker => "px-1 py-0.5",
	};
	let ink = layer(&mark.ink.drawing());

	let highlight = View()
		.class("self-start")
		.class(padding)
		.class(tone_class("text", mark.text));
	// A strike crosses the text, so it paints over it; every other mark sits
	// behind.
	let highlight = if mark.over_text {
		highlight.children(children).child(ink)
	} else {
		highlight.child(ink).children(children)
	};
	with_class(highlight, class)
}
