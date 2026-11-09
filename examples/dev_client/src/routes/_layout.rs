//! The frame around the join screen.

use leptoskit::prelude::Children;
use leptoskit::prelude::Element;
use leptoskit::prelude::View;
use leptoskit::prelude::component;

/// Wraps the matched route.
#[component]
pub fn Layout(children: Children) -> Element {
	View().class("flex-1 bg-slate-950").children(children)
}
