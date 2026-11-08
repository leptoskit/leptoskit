//! `/`: the notes screen, shared across every platform.

use leptoskit::prelude::Element;
use leptoskit::prelude::component;
use leptoskit_router::Route;

use crate::Notes;

/// Names the route `Home` instead of `Index`.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct Home;

/// Builds the notes screen for every destination.
#[component]
pub fn Page() -> Element {
	let notes = reactive_graph::owner::use_context::<Notes>()
		.expect("the app entry provides Notes before rendering routes");
	notes.screen()
}
