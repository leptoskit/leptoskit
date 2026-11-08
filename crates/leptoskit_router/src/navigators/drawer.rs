//! Drawer navigator: a side panel of typed route roots.

use leptoskit_native::Element;
use leptoskit_native::components::Pressable;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;
use reactive_graph::traits::Get;

use super::route_matches_root;
use crate::Routable;
use crate::Router;

/// One drawer row: a typed root route and its label.
#[derive(Clone, Debug)]
pub struct DrawerItem<R> {
	root: R,
	title: String,
}

impl<R> DrawerItem<R>
where
	R: Routable + Clone,
{
	/// A row that selects when the current route is `root` or under it.
	pub fn root(root: impl Into<R>) -> Self {
		let root = root.into();
		let title = root.href().as_str().to_owned();
		Self { root, title }
	}

	/// Sets the row's visible title.
	#[must_use]
	pub fn title(mut self, title: impl Into<String>) -> Self {
		self.title = title.into();
		self
	}

	/// The row's root route.
	pub fn route(&self) -> &R {
		&self.root
	}

	/// The row's label.
	pub fn label(&self) -> &str {
		&self.title
	}
}

/// A side drawer for a directory of screens.
#[derive(Clone, Debug)]
pub struct Drawer<R> {
	items: Vec<DrawerItem<R>>,
}

impl<R> Drawer<R>
where
	R: Routable + Clone + PartialEq + Send + Sync + 'static,
{
	/// A drawer listing `items`.
	pub fn new(items: impl IntoIterator<Item = DrawerItem<R>>) -> Self {
		Self {
			items: items.into_iter().collect(),
		}
	}

	/// The configured rows.
	pub fn items(&self) -> &[DrawerItem<R>] {
		&self.items
	}

	/// The row that matches `route`, if any.
	pub fn selected(&self, route: &R) -> Option<&DrawerItem<R>> {
		self.items
			.iter()
			.find(|item| route_matches_root(route, &item.root))
	}

	/// Wraps `children` with a menu button and an optional side panel. Open
	/// state lives on the [`Router`] so it survives re-renders.
	pub fn shell(&self, children: Element, navigation: &Router<R>, route: &R) -> Element {
		let open = navigation.drawer_open();
		let toggle = navigation.clone();
		let open_for_press = open.clone();
		let header = View()
			.class("flex-row items-center p-3 gap-3")
			.test_id("drawer-header")
			.child(
				Pressable("Menu")
					.test_id("drawer-toggle")
					.on_press(move || {
						let next = !open_for_press.get();
						toggle.set_drawer_open(next);
					})
					.child(Text("Menu".to_owned())),
			);

		let mut shell = View()
			.class("flex-1")
			.test_id("drawer-shell")
			.child(header)
			.child(View().class("flex-1").child(children));

		if open.get() {
			let panel = drawer_panel(self, navigation, route);
			shell = View()
				.class("flex-1 flex-row")
				.test_id("drawer-shell")
				.child(panel)
				.child(shell);
		}

		shell
	}
}

fn drawer_panel<R>(drawer: &Drawer<R>, navigation: &Router<R>, route: &R) -> Element
where
	R: Routable + Clone + PartialEq + Send + Sync + 'static,
{
	let rows = drawer.items.iter().map(|item| {
		let selected = route_matches_root(route, &item.root);
		let label = item.title.clone();
		let root = item.root.clone();
		let router = navigation.clone();
		let class = if selected {
			"p-3 opacity-100"
		} else {
			"p-3 opacity-60"
		};
		Pressable(label.clone())
			.class(class)
			.test_id(format!("drawer-{}", item.title.to_ascii_lowercase()))
			.on_press(move || {
				router.replace(root.clone());
				router.set_drawer_open(false);
			})
			.child(Text(label))
	});

	View()
		.class("w-56 bg-slate-100 dark:bg-slate-900")
		.test_id("drawer-panel")
		.children(rows)
}
