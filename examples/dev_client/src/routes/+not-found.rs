//! Unmatched URLs.

use leptoskit::prelude::Element;
use leptoskit::prelude::Text;
use leptoskit::prelude::View;
use leptoskit::prelude::component;
use leptoskit::view;

/// Anything the router does not match.
#[component]
pub fn Page() -> Element {
	view! {
		<View class="flex-1 items-center justify-center p-6 bg-slate-950">
			<Text class="text-2xl font-semibold text-white">"Not found"</Text>
		</View>
	}
}
