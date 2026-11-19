//! Hand-drawn maps: paper, a faint graticule, rough areas, routes, dots, and
//! pins.
//!
//! There are no tiles, no network requests, and no extra dependencies. A
//! [`Camera`] from [`skribble_core::maps`] projects geographic coordinates
//! into a flat Web Mercator viewport, the core inks areas, routes, dots, and
//! pins, and these components lay the ink out as leptoskit `Vector` elements
//! with pressable, labelled pins.
//!
//! ```
//! use leptoskit::prelude::*;
//! use leptoskit_skribble::maps::Camera;
//! use leptoskit_skribble::maps::LatLng;
//! use leptoskit_skribble::maps::MapFeature;
//! use leptoskit_skribble::maps::MapPin;
//! use leptoskit_skribble::maps::PinCategory;
//! use leptoskit_skribble::maps::WiredMap;
//!
//! pub fn city_walk(on_pin: impl Fn() + Send + Sync + 'static) -> Element {
//! 	let start = LatLng::new(51.5074, -0.1278);
//! 	let end = LatLng::new(51.5160, -0.1100);
//! 	view! {
//! 		<WiredMap
//! 			camera=Camera::new(start, 14.0)
//! 			width=360.0
//! 			height=240.0
//! 			features=vec![MapFeature::route(vec![start, end])]
//! 			pins=vec![
//! 				MapPin::new(start, "Start").with_category(PinCategory::Coffee).with_on_press(on_pin),
//! 				MapPin::new(end, "Finish"),
//! 			]
//! 			label="A sketched walking route"
//! 		/>
//! 	}
//! }
//! # let _ = city_walk(|| {});
//! ```
//!
//! | Component                                          | Draws                                                          |
//! | -------------------------------------------------- | -------------------------------------------------------------- |
//! | [`WiredMap`](fn@WiredMap)                          | Paper, a faint graticule, and rough features and pins          |
//! | [`WiredMapZoomControls`](fn@WiredMapZoomControls)  | Two hand-drawn buttons that propose a closer or further camera |

mod map;
mod pin;
mod zoom;

pub use map::WiredMap;
pub use pin::MapPin;
pub use skribble_core::maps::Camera;
pub use skribble_core::maps::LatLng;
pub use skribble_core::maps::LatLngBounds;
pub use skribble_core::maps::MAX_LATITUDE;
pub use skribble_core::maps::MAX_ZOOM;
pub use skribble_core::maps::MIN_ZOOM;
pub use skribble_core::maps::MapFeature;
pub use skribble_core::maps::MapStyle;
pub use skribble_core::maps::PIN_HEIGHT;
pub use skribble_core::maps::PIN_WIDTH;
pub use skribble_core::maps::PinCategory;
pub use skribble_core::maps::TILE_SIZE;
pub use zoom::WiredMapZoomControls;
