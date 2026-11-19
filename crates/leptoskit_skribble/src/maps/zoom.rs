//! Hand-drawn zoom controls for a controlled map camera.

use std::sync::Arc;

use leptoskit::native::Element;
use leptoskit::prelude::*;
use leptoskit::yoga::Dimension;
use skribble_core::maps::ZOOM_BUTTON_SIZE;
use skribble_core::maps::zoom_button_drawing;
use skribble_core::maps::zoom_steps;

use crate::Theme;
use crate::ink::layer;
use crate::ink::tone_class;
use crate::ink::with_class;

/// Two hand-drawn buttons that propose a closer or further camera.
///
/// Like every leptoskit control it holds no state: it shows `zoom` and
/// proposes `zoom + step` or `zoom - step` through `on_zoom`. Keep the zoom in
/// your own state, build the [`Camera`](super::Camera) from it, and rebuild
/// with the value you accept.
///
/// ```
/// use leptoskit::prelude::*;
/// use leptoskit_skribble::maps::WiredMapZoomControls;
///
/// let controls = view! {
/// 	<WiredMapZoomControls zoom=12.0 on_zoom=|next: f64| println!("zoom to {next}") />
/// };
/// assert_eq!(controls.children.len(), 2);
/// ```
///
/// # Panics
///
/// Panics when `step` or `zoom` is not finite, or when `step` is not positive.
#[component]
pub fn WiredMapZoomControls(
	/// The zoom the map is showing now.
	zoom: f64,
	/// Receives the proposed zoom. Clamp it to your own limits and rebuild.
	on_zoom: impl Fn(f64) + Send + Sync + 'static,
	/// How far one press moves the zoom.
	#[prop(default = 1.0)]
	step: f64,
	/// Chooses the wobble. The same seed always draws the same buttons.
	#[prop(default = 1)]
	seed: i64,
	/// The colours and line weight to draw with.
	#[prop(default = Theme::SKRIBBLE)]
	theme: Theme,
	/// Extra Tailwind classes for the column.
	#[prop(optional, into)]
	class: Option<String>,
) -> Element {
	let Some((next_closer, next_further)) = zoom_steps(zoom, step) else {
		panic!(
			"zoom controls need a finite zoom and a positive step, found zoom {zoom} and step \
			 {step}"
		);
	};

	let on_zoom = Arc::new(on_zoom);
	let closer = Arc::clone(&on_zoom);
	let further = Arc::clone(&on_zoom);

	let controls = View()
		.class("flex-col gap-2")
		.child(button("Zoom in", "+", seed, theme, move || {
			closer(next_closer);
		}))
		.child(button(
			"Zoom out",
			"−",
			seed.wrapping_add(1),
			theme,
			move || further(next_further),
		));

	with_class(controls, class)
}

/// One square, pressable zoom button with its rough border.
fn button(
	label: &str,
	glyph: &str,
	seed: i64,
	theme: Theme,
	on_press: impl Fn() + Send + Sync + 'static,
) -> Element {
	let drawing = zoom_button_drawing(seed, theme);
	Pressable(label.to_owned())
		.class("items-center justify-center")
		.class(tone_class("text", theme.ink()))
		.child(layer(&drawing))
		.child(Text(glyph.to_owned()).class("text-xl font-bold"))
		.with_style(|style| {
			style.width = Dimension::Points(ZOOM_BUTTON_SIZE);
			style.height = Dimension::Points(ZOOM_BUTTON_SIZE);
		})
		.on_press(on_press)
}
