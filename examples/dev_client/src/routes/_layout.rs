//! The frame around the join screen.

use leptoskit::prelude::Children;
use leptoskit::prelude::Element;
use leptoskit::prelude::View;
use leptoskit::prelude::component;
use leptoskit::view;

/// Wraps the matched route.
#[component]
pub fn Layout(children: Children) -> Element {
	view! { <View class="flex-1 bg-slate-950">{children}</View> }
}
