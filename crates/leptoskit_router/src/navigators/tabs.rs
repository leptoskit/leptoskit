//! Tabs navigator: a bar of typed route roots.

use std::sync::Arc;

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

type BarRenderer<R> = Arc<dyn Fn(&TabBar<'_, R>) -> Element + Send + Sync>;

/// A tab bar for a directory of screens.
#[derive(Clone)]
pub struct Tabs<R: 'static> {
	entries: Vec<Tab<R>>,
	placement: TabPlacement,
	bar: Option<BarRenderer<R>>,
}

impl<R: std::fmt::Debug + 'static> std::fmt::Debug for Tabs<R> {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("Tabs")
			.field("tabs", &self.entries)
			.field("placement", &self.placement)
			.field("bar", &self.bar.is_some())
			.finish()
	}
}

/// What a custom tab bar draws from: every tab, which one is selected, and
/// links that switch to a tab the way the built-in bar does.
pub struct TabBar<'a, R: 'static> {
	tabs: &'a Tabs<R>,
	navigation: &'a Router<R>,
	route: &'a R,
}

impl<R: 'static> TabBar<'_, R>
where
	R: Routable + Clone + PartialEq + Send + Sync + 'static,
{
	/// Each tab, in order, with whether the current route is under it.
	pub fn items(&self) -> impl Iterator<Item = (&Tab<R>, bool)> {
		self.tabs
			.entries
			.iter()
			.map(|tab| (tab, route_matches_root(self.route, &tab.root)))
	}

	/// The current route.
	pub fn route(&self) -> &R {
		self.route
	}

	/// The router the bar navigates.
	pub fn navigation(&self) -> &Router<R> {
		self.navigation
	}

	/// A link to `tab` around `child`: it `replace`s to the tab's root, is
	/// named by the tab's title, and has the built-in bar's `tab-<title>`
	/// test id.
	pub fn link(&self, tab: &Tab<R>, child: Element) -> Element {
		self.navigation
			.link_replace(tab.root.clone(), tab.title.clone())
			.test_id(format!("tab-{}", tab.title.to_ascii_lowercase()))
			.child(child)
	}
}

impl<R> Tabs<R>
where
	R: Routable + Clone + PartialEq + Send + Sync + 'static,
{
	/// Tabs for `items`, bottom placement by default.
	pub fn new(items: impl IntoIterator<Item = Tab<R>>) -> Self {
		Self {
			entries: items.into_iter().collect(),
			placement: TabPlacement::Bottom,
			bar: None,
		}
	}

	/// Places the bar above or below the content.
	#[must_use]
	pub fn placement(mut self, placement: TabPlacement) -> Self {
		self.placement = placement;
		self
	}

	/// Draws the bar with `render` instead of the built-in row of titles,
	/// so an app can give its tabs its own look. [`TabBar::link`] keeps
	/// switching tabs the same.
	#[must_use]
	pub fn bar(
		mut self,
		render: impl Fn(&TabBar<'_, R>) -> Element + Send + Sync + 'static,
	) -> Self {
		self.bar = Some(Arc::new(render));
		self
	}

	/// The configured tabs.
	pub fn items(&self) -> &[Tab<R>] {
		&self.entries
	}

	/// The tab that matches `route`, if any.
	pub fn selected(&self, route: &R) -> Option<&Tab<R>> {
		self.entries
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
	if let Some(render) = &tabs.bar {
		return render(&TabBar {
			tabs,
			navigation,
			route,
		});
	}
	let links = tabs.entries.iter().map(|tab| {
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
