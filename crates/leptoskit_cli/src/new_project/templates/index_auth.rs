//! `/`: the signed-in home screen, shared across every platform.

use leptoskit::prelude::Element;
use leptoskit::prelude::Text;
use leptoskit::prelude::View;
use leptoskit::prelude::component;
use leptoskit_router::Route;

use crate::Shell;

/// Names the route `Home` instead of `Index`.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct Home;

/// Shows the signed-in person's notes, or a loading message while the screen
/// for a newly signed-in account is being built.
#[component]
pub fn Page() -> Element {
	let shell = reactive_graph::owner::use_context::<Shell>()
		.expect("the app entry provides Shell before rendering routes");
	match shell.notes() {
		Some(notes) => notes.screen(),
		None => {
			View()
				.class("w-full max-w-[640px] p-6 bg-slate-950 text-white")
				.child(Text("Loading your notes…").class("text-slate-300"))
		}
	}
}
