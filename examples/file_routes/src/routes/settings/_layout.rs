//! Settings: a drawer of sections beside the current screen.

use leptoskit::prelude::*;
use leptoskit_router::navigators::Drawer;
use leptoskit_router::navigators::DrawerItem;

use crate::routes::Route;
use crate::routes::Section;

/// Overview plus two typed section roots.
pub fn navigator() -> Drawer<Route> {
	Drawer::new([
		DrawerItem::root(Route::Settings).title("Overview"),
		DrawerItem::root(Section {
			section: "account".to_owned(),
		})
		.title("Account"),
		DrawerItem::root(Section {
			section: "privacy".to_owned(),
		})
		.title("Privacy"),
	])
}

/// A heading that follows the route: `Settings`, or `Settings › privacy`.
#[component]
pub fn Layout(children: Children, route: &Route) -> Element {
	let heading = match route {
		Route::Section(section) => format!("Settings › {}", section.section),
		_ => "Settings".to_owned(),
	};

	view! {
		<View class="flex-1">
			<Text class="p-6 text-2xl font-semibold">{heading}</Text>
			<View class="flex-1">{children}</View>
		</View>
	}
}
