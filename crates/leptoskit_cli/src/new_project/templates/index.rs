//! `/`: the home screen, shared across every platform.

use leptoskit::prelude::Button;
use leptoskit::prelude::Element;
use leptoskit::prelude::Get;
use leptoskit::prelude::Text;
use leptoskit::prelude::View;
use leptoskit::prelude::component;
use leptoskit_router::Route;

use crate::Counter;

/// Names the route `Home` instead of `Index`.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct Home;

/// Builds the shared counter screen with a platform-neutral event handler.
#[component]
pub fn Page() -> Element {
	let state = reactive_graph::owner::use_context::<Counter>()
		.expect("the app entry provides Counter before rendering routes");
	let counter = state.clone();
	let value = state.signal();

	View()
		.class("min-h-screen items-center justify-center gap-6 p-6 bg-slate-950 text-white")
		.child(Text("Hello, leptoskit").class("text-3xl font-bold"))
		.child(Text(move || format!("Count: {}", value.get())).class("text-xl"))
		.child(
			Button("Add one")
				.class("rounded-lg bg-indigo-500 px-6 py-3 font-semibold")
				.on_press(move || counter.increment()),
		)
}
