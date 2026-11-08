//! The root layout: chrome around every screen.

use leptoskit::prelude::Children;
use leptoskit::prelude::Element;
use leptoskit::prelude::View;
use leptoskit::prelude::component;

/// Wraps the matched route in the app's frame.
#[component]
pub fn Layout(children: Children) -> Element {
	View().class("flex-1").children(children)
}
