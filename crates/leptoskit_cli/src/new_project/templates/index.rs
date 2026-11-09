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

/// The first screen: what the toolkit is, and a counter that is the same
/// control on every platform.
#[component]
pub fn Page() -> Element {
	let state = reactive_graph::owner::use_context::<Counter>()
		.expect("the app entry provides Counter before rendering routes");
	let counter = state.clone();
	let value = state.signal();

	View()
		.class("flex-1 justify-center gap-8 bg-slate-950 p-6")
		.child(Text("leptoskit").class("text-sm font-semibold text-indigo-300"))
		.child(Text("Hello, leptoskit").class("text-4xl font-bold text-white"))
		.child(
			Text("One tree of components, drawn as real widgets on every platform you ship.")
				.class("text-base text-slate-300"),
		)
		.child(
			View()
				.class("gap-4 rounded-2xl bg-slate-900 p-6")
				.child(
					Text(move || format!("Count: {}", value.get()))
						.class("text-3xl font-bold text-white"),
				)
				.child(
					Button("Add one")
						.class("rounded-lg bg-indigo-500 px-6 py-3 font-semibold text-white")
						.on_press(move || counter.increment()),
				),
		)
}
