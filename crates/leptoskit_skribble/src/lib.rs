//! <!-- {=skribbleSummary|trim|linePrefix:"//! ":true} -->
//!
//! It turns the drawings of [skribble](https://openbudgetfun.github.io/skribble/), a hand-drawn
//! design system, into leptoskit components. `skribble_core` decides what to
//! draw; this crate lays the ink out as `Vector` elements, so the same rough
//! strokes render on the web and every native backend.
//!
//! <!-- {/skribbleSummary} -->
//!
//! Rough borders, sketched fills, pen marks, doodles, charts, and maps for
//! apps written once with leptoskit. Every drawing comes from
//! [`skribble_core`], the framework-free Rust port of skribble, so the same
//! seed draws the same wobble as skribble's Flutter widgets. This crate owns
//! everything that is leptoskit's: elements, Tailwind classes, layout, events,
//! and accessibility.
//!
//! ```
//! use leptoskit::prelude::*;
//! use leptoskit_skribble::*;
//!
//! pub fn welcome(on_start: impl Fn() + Send + Sync + 'static) -> Element {
//! 	view! {
//! 		<SkribblePage class="items-center gap-6 p-6">
//! 			<WiredCard seed=3 class="gap-4">
//! 				<View class="flex-row items-center gap-1">
//! 					<Text class="text-2xl font-bold">"Draw it"</Text>
//! 					<WiredHighlight kind=HighlightKind::Circle seed=5>
//! 						<Text class="text-2xl font-bold">"differently"</Text>
//! 					</WiredHighlight>
//! 				</View>
//! 				<WiredButton label="Start sketching" on_press=on_start />
//! 			</WiredCard>
//! 		</SkribblePage>
//! 	}
//! }
//! # let _ = welcome(|| {});
//! ```
//!
//! # Components
//!
//! | Component                             | Draws                                                             |
//! | ------------------------------------- | ----------------------------------------------------------------- |
//! | [`SkribblePage`](fn@SkribblePage)     | The theme's paper and ink behind a screen                         |
//! | [`WiredBox`](fn@WiredBox)             | A rough rectangle around content, optionally filled               |
//! | [`WiredCard`](fn@WiredCard)           | A roomy box for grouped content                                   |
//! | [`WiredCircle`](fn@WiredCircle)       | A rough ellipse around content, for stickers and step numbers     |
//! | [`WiredBadge`](fn@WiredBadge)         | A small filled capsule for a status or count                      |
//! | [`WiredButton`](fn@WiredButton)       | A button with a rough border, filled when it is the primary one   |
//! | [`WiredInput`](fn@WiredInput)         | A platform text field inside a rough frame                        |
//! | [`WiredCheckbox`](fn@WiredCheckbox)   | A rough square that gets a tick                                   |
//! | [`WiredRadio`](fn@WiredRadio)         | A rough circle that gets a dot                                    |
//! | [`WiredSwitch`](fn@WiredSwitch)       | A rough capsule with a sliding knob                               |
//! | [`WiredProgress`](fn@WiredProgress)   | A rough track that fills with hachure                             |
//! | [`WiredHighlight`](fn@WiredHighlight) | An underline, circle, box, strike, or marker around text          |
//! | [`WiredDivider`](fn@WiredDivider)     | A rough horizontal rule                                           |
//! | [`Sketch`](fn@Sketch)                 | Any [`Shape`] at an explicit size: arrows, scribbles, annotations |
//!
//! [`charts`] has candlestick, line, and bar charts, and [`maps`] has a
//! hand-drawn map with pins and zoom controls.
//!
//! Every component takes a `seed` (give neighbours different ones so their
//! wobbles differ), an optional [`Theme`], and extra Tailwind `class`es.
//! Controls follow leptoskit's controlled pattern: they show a value and
//! propose changes through a callback, and you rebuild with the accepted
//! value.
//!
//! # Colour
//!
//! The default [`Theme`] is skribble's sunny colourway. Colour-code cards
//! with [`Theme::accent_at`], keep [`ButtonVariant::Primary`] for the one loud
//! action, and pick another colourway with
//! `Theme::colorway(Colorway::Berry)`.
//!
//! # Your own drawings
//!
//! Anything [`skribble_core`] draws is an [`InkDrawing`]. [`ink::layer`] and
//! [`ink::vectors`] turn one into leptoskit elements, so a drawing the core
//! gains later works here without waiting for a component.
//!
//! # Platforms
//!
//! The ink is drawn as leptoskit `Vector` elements, which the web renderer
//! shows as inline SVG and every native backend (macOS, iOS, Android,
//! Windows, Linux) draws through its own path API. Ink paints with the
//! element's text colour, so dark mode follows leptoskit's `dark:` classes
//! everywhere, and no feature needs turning on.

#![allow(non_snake_case)]

mod button;
pub mod charts;
mod container;
mod divider;
mod highlight;
pub mod ink;
mod input;
pub mod maps;
mod progress;
mod sketch;
mod toggle;

pub use button::WiredButton;
pub use container::SkribblePage;
pub use container::WiredBadge;
pub use container::WiredBox;
pub use container::WiredCard;
pub use container::WiredCircle;
pub use divider::WiredDivider;
pub use highlight::WiredHighlight;
pub use input::WiredInput;
pub use progress::WiredProgress;
pub use sketch::Sketch;
pub use skribble_core;
pub use skribble_core::FillStyle;
pub use skribble_core::InkDrawing;
pub use skribble_core::Palette;
pub use skribble_core::Point;
pub use skribble_core::RoughPen;
pub use skribble_core::Shape;
pub use skribble_core::Theme;
pub use skribble_core::Tone;
pub use skribble_core::colorway;
pub use skribble_core::colorway::Colorway;
pub use skribble_core::components::ButtonVariant;
pub use skribble_core::components::HighlightKind;
pub use skribble_core::rough;
pub use toggle::WiredCheckbox;
pub use toggle::WiredRadio;
pub use toggle::WiredSwitch;
