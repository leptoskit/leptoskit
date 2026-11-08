//! The tabbed part of the app. `navigator` owns the tab bar; this layout only
//! frames the selected tab.

use leptoskit::prelude::*;
use leptoskit_router::navigators::Tab;
use leptoskit_router::navigators::Tabs;

use crate::routes::Map;
use crate::routes::Route;
use crate::routes::Today;

/// Four tabs whose roots are typed routes, not string names.
pub fn navigator() -> Tabs<Route> {
	Tabs::new([
		Tab::root(Today).title("Today"),
		Tab::root(Map { near: None }).title("Map"),
		Tab::root(Route::Notes).title("Notes"),
		Tab::root(Route::You).title("You"),
	])
}

/// Frames the selected tab's page.
#[component]
pub fn Layout(children: Children) -> Element {
	view! { <View class="flex-1">{children}</View> }
}
