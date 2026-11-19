//! Hand-drawn map pins with pastel category fills and a cream sticker edge.

use std::fmt;
use std::sync::Arc;

use leptoskit::native::Element;
use leptoskit::native::Role;
use leptoskit::prelude::Pressable;
use leptoskit::prelude::Text;
use leptoskit::prelude::View;
use leptoskit::yoga::Dimension;
use leptoskit::yoga::PositionType;
use skribble_core::maps::Camera;
use skribble_core::maps::LatLng;
use skribble_core::maps::PIN_HEIGHT;
use skribble_core::maps::PIN_LABEL_GAP;
use skribble_core::maps::PIN_LABEL_WIDTH;
use skribble_core::maps::PIN_WIDTH;
use skribble_core::maps::PinCategory;
use skribble_core::maps::pin_drawing;
use skribble_core::maps::pin_origin;

use crate::Theme;
use crate::ink::tone_class;
use crate::ink::vectors;

/// A hand-drawn pin anchored to one geographic point.
///
/// The point is the tip of the pin: the rounded head sits above it, so a pin
/// never covers the place it marks. A short label can sit under the tip. With
/// an `on_press` the pin is a real button named by its label; without one it
/// is a decorative image.
///
/// ```
/// use leptoskit_skribble::maps::LatLng;
/// use leptoskit_skribble::maps::MapPin;
/// use leptoskit_skribble::maps::PinCategory;
///
/// let cafe = MapPin::new(LatLng::new(51.5074, -0.1278), "Favourite cafe")
/// 	.with_category(PinCategory::Coffee);
/// assert_eq!(cafe.category, PinCategory::Coffee);
/// ```
#[derive(Clone)]
pub struct MapPin {
	/// The geographic anchor, at the tip of the pin.
	pub position: LatLng,
	/// A short label shown under the pin, and its accessible name.
	pub label: String,
	/// What the pin marks, which chooses its pastel fill.
	pub category: PinCategory,
	/// Runs when the pin is activated. Without one the pin is decorative.
	pub on_press: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl MapPin {
	/// A pin at `position` carrying `label`.
	#[must_use]
	pub fn new(position: LatLng, label: impl Into<String>) -> Self {
		Self {
			position,
			label: label.into(),
			category: PinCategory::Place,
			on_press: None,
		}
	}

	/// The same pin in another category.
	#[must_use]
	pub fn with_category(mut self, category: PinCategory) -> Self {
		self.category = category;
		self
	}

	/// The same pin, pressable.
	#[must_use]
	pub fn with_on_press(self, on_press: impl Fn() + Send + Sync + 'static) -> Self {
		Self {
			on_press: Some(Arc::new(on_press)),
			..self
		}
	}

	/// A pin with no position and no label, useful as a placeholder.
	#[must_use]
	pub fn empty() -> Self {
		Self::new(LatLng::default(), String::new())
	}
}

impl Default for MapPin {
	fn default() -> Self {
		Self::empty()
	}
}

impl fmt::Debug for MapPin {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter
			.debug_struct("MapPin")
			.field("position", &self.position)
			.field("label", &self.label)
			.field("category", &self.category)
			.field("on_press", &self.on_press.is_some())
			.finish()
	}
}

/// The pin elements inside the map, positioned and culled.
pub(crate) fn elements(
	camera: Camera,
	width: f32,
	height: f32,
	pins: &[MapPin],
	theme: Theme,
	seed: i64,
) -> Vec<Element> {
	pins.iter()
		.enumerate()
		.filter_map(|(index, pin)| {
			let (left, top) = pin_origin(camera, pin.position, width, height)?;
			let drawing = pin_drawing(pin.category, theme, seed.wrapping_add(index as i64));
			let mut children = vectors(&drawing);
			if !pin.label.is_empty() {
				children.push(label(pin, theme));
			}
			let container = if let Some(on_press) = &pin.on_press {
				let on_press = Arc::clone(on_press);
				let name = if pin.label.is_empty() {
					"Map pin"
				} else {
					pin.label.as_str()
				};
				Pressable(name.to_owned())
					.class("items-center")
					.on_press(move || on_press())
			} else {
				let view = View();
				if pin.label.is_empty() {
					view.aria_hidden(true)
				} else {
					view.role(Role::Img).aria_label(pin.label.clone())
				}
			};
			Some(container.children(children).with_style(|style| {
				style.position = PositionType::Absolute;
				style.width = Dimension::Points(PIN_WIDTH);
				style.height = Dimension::Points(PIN_HEIGHT);
				style.inset.left = Dimension::Points(left);
				style.inset.top = Dimension::Points(top);
			}))
		})
		.collect()
}

/// The pin's short label, centred under the tip.
fn label(pin: &MapPin, theme: Theme) -> Element {
	View()
		.class("items-center")
		.child(
			Text(pin.label.clone())
				.class("text-xs text-center")
				.class(tone_class("text", theme.ink())),
		)
		.with_style(|style| {
			style.position = PositionType::Absolute;
			style.width = Dimension::Points(PIN_LABEL_WIDTH);
			style.inset.left = Dimension::Points((PIN_WIDTH - PIN_LABEL_WIDTH) / 2.0);
			style.inset.top = Dimension::Points(PIN_HEIGHT + PIN_LABEL_GAP);
		})
}

#[cfg(test)]
mod tests {
	use skribble_core::maps::MAX_LATITUDE;

	use super::*;

	const SIZE: (f32, f32) = (320.0, 200.0);

	fn camera() -> Camera {
		Camera::new(LatLng::new(0.0, 0.0), 4.0)
	}

	#[test]
	fn pins_are_positioned_with_their_tip_at_the_point() {
		let pin = MapPin::new(LatLng::default(), "Centre");
		let (x, y) = camera().project(LatLng::default(), SIZE.0, SIZE.1);
		let elements = elements(camera(), SIZE.0, SIZE.1, &[pin], Theme::SKRIBBLE, 1);
		assert_eq!(elements.len(), 1);
		let style = elements[0].style.clone();
		assert_eq!(style.position, PositionType::Absolute);
		assert_eq!(style.width, Dimension::Points(PIN_WIDTH));
		assert_eq!(style.height, Dimension::Points(PIN_HEIGHT));
		assert_eq!(style.inset.left, Dimension::Points(x - PIN_WIDTH / 2.0));
		assert_eq!(style.inset.top, Dimension::Points(y - PIN_HEIGHT));
	}

	#[test]
	fn pins_outside_the_viewport_are_culled() {
		let far = MapPin::new(LatLng::new(MAX_LATITUDE, 170.0), "Far");
		let elements = elements(camera(), SIZE.0, SIZE.1, &[far], Theme::SKRIBBLE, 1);
		assert!(elements.is_empty());
	}

	#[test]
	fn a_pressable_pin_is_a_button_named_by_its_label() {
		let pressed = Arc::new(std::sync::atomic::AtomicBool::new(false));
		let shared = Arc::clone(&pressed);
		let pin = MapPin::new(LatLng::default(), "Favourite cafe")
			.with_on_press(move || shared.store(true, std::sync::atomic::Ordering::SeqCst));
		let elements = elements(camera(), SIZE.0, SIZE.1, &[pin], Theme::SKRIBBLE, 1);
		assert_eq!(
			elements[0].props.accessibility_label.as_deref(),
			Some("Favourite cafe")
		);
	}

	#[test]
	fn a_pin_without_a_label_is_hidden_from_assistive_technology() {
		let pin = MapPin::new(LatLng::default(), "");
		let elements = elements(camera(), SIZE.0, SIZE.1, &[pin], Theme::SKRIBBLE, 1);
		assert_eq!(elements[0].props.aria().hidden, Some(true));
	}

	#[test]
	fn empty_pins_draw_their_shape_without_a_label() {
		let pin = MapPin::empty();
		let elements = elements(camera(), SIZE.0, SIZE.1, &[pin], Theme::SKRIBBLE, 1);
		assert_eq!(elements[0].children.len(), 3);
	}
}
