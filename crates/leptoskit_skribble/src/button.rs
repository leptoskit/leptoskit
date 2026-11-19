//! The hand-drawn button.

use leptoskit::native::Element;
use leptoskit::prelude::*;
use skribble_core::components;
use skribble_core::components::ButtonVariant;

use crate::Theme;
use crate::ink::layer;
use crate::ink::tone_class;
use crate::ink::with_class;

/// A button drawn by hand: a rough rectangle around its label, filled with
/// the accent when it is the primary action.
///
/// It is a real button on every platform, named by its label, and at least
/// 44 pixels tall so it is easy to hit; larger text makes it taller.
///
/// ```
/// use leptoskit::prelude::*;
/// use leptoskit_skribble::ButtonVariant;
/// use leptoskit_skribble::WiredButton;
///
/// let actions = view! {
/// 	<View class="flex-row gap-3">
/// 		<WiredButton label="Save" on_press=|| println!("saved") />
/// 		<WiredButton label="Cancel" seed=2 variant=ButtonVariant::Secondary on_press=|| {} />
/// 	</View>
/// };
/// assert_eq!(actions.children.len(), 2);
/// ```
#[component]
pub fn WiredButton(
	/// The visible label, which also names the button for assistive
	/// technology.
	#[prop(into)]
	label: String,
	/// Runs when the button is activated by pointer, touch, or keyboard.
	on_press: impl Fn() + Send + Sync + 'static,
	/// Filled with the accent, or an outline only.
	#[prop(optional)]
	variant: ButtonVariant,
	/// Shows the button but ignores activation.
	#[prop(optional)]
	disabled: bool,
	/// Chooses the wobble. The same seed always draws the same border; give
	/// neighbouring buttons different seeds.
	#[prop(default = 1)]
	seed: i64,
	/// The colours to draw with.
	#[prop(default = Theme::SKRIBBLE)]
	theme: Theme,
	/// Extra Tailwind classes, such as `w-full`.
	#[prop(optional, into)]
	class: Option<String>,
) -> Element {
	let ink = components::button(variant, seed, theme);

	let button = Pressable(label.clone())
		.class("self-start items-center justify-center min-h-11 px-5 py-3")
		.class(tone_class("text", ink.content_tone()))
		.enabled(!disabled)
		.child(layer(&ink.drawing()))
		.child(Text(label).class("text-[17px] font-semibold text-center"))
		.on_press(on_press);
	let button = if disabled {
		button.class("opacity-50")
	} else {
		button
	};
	with_class(button, class)
}
