//! Tabs navigator: a bar of typed route roots.

use leptoskit_native::Element;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;

use super::route_matches_root;
use crate::Routable;
use crate::Router;

/// Where the tab bar sits relative to the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TabPlacement {
	/// Under the screen content.
	#[default]
	Bottom,
	/// Above the screen content.
	Top,
}

/// One tab: a typed root route and its label.
#[derive(Clone, Debug)]
pub struct Tab<R> {
	root: R,
	title: String,
}

impl<R> Tab<R>
where
	R: Routable + Clone,
{
	/// A tab that selects when the current route is `root` or under it.
	pub fn root(root: impl Into<R>) -> Self {
		let root = root.into();
		let title = root.href().as_str().to_owned();
		Self { root, title }
	}

	/// Sets the tab's visible title.
	#[must_use]
	pub fn title(mut self, title: impl Into<String>) -> Self {
		self.title = title.into();
		self
	}

	/// The tab's root route.
	pub fn route(&self) -> &R {
		&self.root
	}

	/// The tab's label.
	pub fn label(&self) -> &str {
		&self.title
	}
}

/// A tab bar for a directory of screens.
#[derive(Clone, Debug)]
pub struct Tabs<R> {
	tabs: Vec<Tab<R>>,
	placement: TabPlacement,
}

impl<R> Tabs<R>
where
	R: Routable + Clone + PartialEq + Send + Sync + 'static,
{
	/// Tabs for `items`, bottom placement by default.
	pub fn new(items: impl IntoIterator<Item = Tab<R>>) -> Self {
		Self {
			tabs: items.into_iter().collect(),
			placement: TabPlacement::Bottom,
		}
	}

	/// Places the bar above or below the content.
	#[must_use]
	pub fn placement(mut self, placement: TabPlacement) -> Self {
		self.placement = placement;
		self
	}

	/// The configured tabs.
	pub fn items(&self) -> &[Tab<R>] {
		&self.tabs
	}

	/// The tab that matches `route`, if any.
	pub fn selected(&self, route: &R) -> Option<&Tab<R>> {
		self.tabs
			.iter()
			.find(|tab| route_matches_root(route, &tab.root))
	}

	/// Wraps `children` with a tab bar of links. Selecting another tab
	/// [`replace`](Router::replace)s to that tab's root so nested history
	/// under the previous tab is not left on the stack.
	pub fn shell(&self, children: Element, navigation: &Router<R>, route: &R) -> Element {
		let bar = tab_bar(self, navigation, route);
		let column = View().class("flex-1").test_id("tabs-shell");

		match self.placement {
			TabPlacement::Bottom => {
				column
					.child(View().class("flex-1").child(children))
					.child(bar)
			}
			TabPlacement::Top => {
				column
					.child(bar)
					.child(View().class("flex-1").child(children))
			}
		}
	}
}

fn tab_bar<R>(tabs: &Tabs<R>, navigation: &Router<R>, route: &R) -> Element
where
	R: Routable + Clone + PartialEq + Send + Sync + 'static,
{
	let links = tabs.tabs.iter().map(|tab| {
		let selected = route_matches_root(route, &tab.root);
		let label = tab.title.clone();
		let root = tab.root.clone();
		let router = navigation.clone();
		let class = if selected {
			"flex-1 items-center p-3 opacity-100"
		} else {
			"flex-1 items-center p-3 opacity-60"
		};
		router
			.link_replace(root, label.clone())
			.class(class)
			.test_id(format!("tab-{}", tab.title.to_ascii_lowercase()))
			.child(Text(label))
	});

	View().class("flex-row").test_id("tabs-bar").children(links)
}
