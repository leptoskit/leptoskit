//! `/platforms`: where this one screen runs.

use leptoskit::prelude::Element;
use leptoskit::prelude::Text;
use leptoskit::prelude::View;
use leptoskit::prelude::component;
use leptoskit_router::Route;

/// Names the route `Platforms`.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct Platforms;

/// The destinations `leptoskit new` turns on.
#[component]
pub fn Page() -> Element {
	let names = [
		("Web", "The browser, server-rendered"),
		("iOS", "UIKit"),
		("Android", "Android views"),
		("macOS", "AppKit"),
		("Windows", "Win32"),
		("Linux", "GTK 4"),
	];

	View()
		.class("flex-1 gap-4 bg-slate-950 p-6")
		.child(Text("Platforms").class("text-3xl font-bold text-white"))
		.child(
			Text("Save a file and each of these draws the same tree.")
				.class("text-base text-slate-300"),
		)
		.children(names.into_iter().map(|(name, detail)| {
			View()
				.class("gap-1 rounded-2xl bg-slate-900 p-4")
				.child(Text(name).class("text-base font-semibold text-white"))
				.child(Text(detail).class("text-sm text-slate-300"))
		}))
}
