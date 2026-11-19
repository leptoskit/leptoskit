//! Hand-drawn checkboxes, radio buttons, and switches.
//!
//! Each is one accessible control containing its drawing and its label, so
//! the whole row responds to a press and assistive technology announces the
//! label with the state.

use leptoskit::native::Element;
use leptoskit::native::Role;
use leptoskit::prelude::*;
use skribble_core::components;

use crate::Theme;
use crate::ink::layer;
use crate::ink::tone_class;
use crate::ink::with_class;

/// The shared row: an accessible control with `role` and `checked`, its
/// drawing, then its label.
fn toggle_row(
	label: String,
	role: Role,
	checked: bool,
	disabled: bool,
	drawing: Element,
	theme: Theme,
	on_press: impl Fn() + Send + Sync + 'static,
) -> Element {
	let row = Pressable(label.clone())
		.role(role)
		.aria_checked(checked)
		.enabled(!disabled)
		.class("self-start flex-row items-center gap-3 min-h-11")
		.class(tone_class("text", theme.ink()))
		.child(drawing)
		.child(Text(label).class("text-base"))
		.on_press(on_press);
	if disabled {
		row.class("opacity-50")
	} else {
		row
	}
}

/// A checkbox drawn by hand: a rough square that gets a tick.
///
/// Like every leptoskit control it shows `checked` and proposes the opposite
/// through `on_toggle`; keep the value in your state and rebuild with it.
///
/// ```
/// use leptoskit::prelude::*;
/// use leptoskit_skribble::WiredCheckbox;
///
/// let row = view! {
/// 	<WiredCheckbox label="Email me updates" checked=true on_toggle=|on: bool| println!("{on}") />
/// };
/// assert_eq!(row.text_content(), "Email me updates");
/// ```
#[component]
pub fn WiredCheckbox(
	/// The visible label, which also names the checkbox.
	#[prop(into)]
	label: String,
	/// Whether the box is ticked.
	checked: bool,
	/// Receives the proposed new state.
	on_toggle: impl Fn(bool) + Send + Sync + 'static,
	/// Shows the checkbox but ignores presses.
	#[prop(optional)]
	disabled: bool,
	/// Chooses the wobble. The same seed always draws the same box and tick.
	#[prop(default = 1)]
	seed: i64,
	/// The colours to draw with.
	#[prop(default = Theme::SKRIBBLE)]
	theme: Theme,
	/// Extra Tailwind classes for the row.
	#[prop(optional, into)]
	class: Option<String>,
) -> Element {
	let ink = components::checkbox(checked, seed, theme);
	let drawing = View()
		.class("w-6 h-6")
		.child(layer(&ink.square.drawing()))
		.children(ink.tick.map(|tick| layer(&tick.drawing())));
	let row = toggle_row(
		label,
		Role::Checkbox,
		checked,
		disabled,
		drawing,
		theme,
		move || on_toggle(!checked),
	);
	with_class(row, class)
}

/// A radio button drawn by hand: a rough circle that gets a dot.
///
/// Render one per option, select at most one, and group the options in a
/// view with the radio group role and a name, so assistive technology
/// announces them as one set. `on_select` runs when this option is chosen.
///
/// ```
/// use leptoskit::native::Role;
/// use leptoskit::prelude::*;
/// use leptoskit_skribble::WiredRadio;
///
/// let options = view! {
/// 	<View role=Role::RadioGroup aria_label="Tool" class="gap-2">
/// 		<WiredRadio label="Pencil" selected=true on_select=|| {} />
/// 		<WiredRadio label="Brush" seed=2 selected=false on_select=|| {} />
/// 	</View>
/// };
/// assert_eq!(options.children.len(), 2);
/// ```
#[component]
pub fn WiredRadio(
	/// The visible label, which also names the option.
	#[prop(into)]
	label: String,
	/// Whether this option is the selected one.
	selected: bool,
	/// Runs when this option is chosen.
	on_select: impl Fn() + Send + Sync + 'static,
	/// Shows the option but ignores presses.
	#[prop(optional)]
	disabled: bool,
	/// Chooses the wobble. The same seed always draws the same circle.
	#[prop(default = 1)]
	seed: i64,
	/// The colours to draw with.
	#[prop(default = Theme::SKRIBBLE)]
	theme: Theme,
	/// Extra Tailwind classes for the row.
	#[prop(optional, into)]
	class: Option<String>,
) -> Element {
	let ink = components::radio(selected, seed, theme);
	let drawing = View()
		.class("w-6 h-6")
		.child(layer(&ink.ring.drawing()))
		.children(ink.dot.map(|dot| layer(&dot.drawing())));
	let row = toggle_row(
		label,
		Role::Radio,
		selected,
		disabled,
		drawing,
		theme,
		on_select,
	);
	with_class(row, class)
}

/// A switch drawn by hand: a rough capsule that fills with the marker, and a
/// knob that slides to the end.
///
/// ```
/// use leptoskit::prelude::*;
/// use leptoskit_skribble::WiredSwitch;
///
/// let row = view! {
/// 	<WiredSwitch label="Dark paper" on=false on_toggle=|on: bool| println!("{on}") />
/// };
/// assert_eq!(row.text_content(), "Dark paper");
/// ```
#[component]
pub fn WiredSwitch(
	/// The visible label, which also names the switch.
	#[prop(into)]
	label: String,
	/// Whether the switch is on.
	on: bool,
	/// Receives the proposed new state.
	on_toggle: impl Fn(bool) + Send + Sync + 'static,
	/// Shows the switch but ignores presses.
	#[prop(optional)]
	disabled: bool,
	/// Chooses the wobble. The same seed always draws the same track and knob.
	#[prop(default = 1)]
	seed: i64,
	/// The colours to draw with.
	#[prop(default = Theme::SKRIBBLE)]
	theme: Theme,
	/// Extra Tailwind classes for the row.
	#[prop(optional, into)]
	class: Option<String>,
) -> Element {
	let ink = components::switch(on, seed, theme);
	let drawing = View()
		.class("w-[52px] h-7")
		.child(layer(&ink.track.drawing()))
		.child(
			View()
				.class("absolute top-[4px] w-5 h-5")
				.class(format!("left-[{}px]", ink.knob_left))
				.child(layer(&ink.knob.drawing())),
		);
	let row = toggle_row(
		label,
		Role::Switch,
		on,
		disabled,
		drawing,
		theme,
		move || on_toggle(!on),
	);
	with_class(row, class)
}
