//! File-route navigators: Stack, Tabs, and Drawer.
//!
//! A `_layout.rs` exports `navigator()` returning one of these. The generated
//! router wraps that layout's output with [`Navigator::shell`], so the folder
//! name `(tabs)` stays a URL-less group — the return value is what selects the
//! chrome, not the directory spelling.

mod drawer;
mod stack;
mod tabs;

pub use drawer::Drawer;
pub use drawer::DrawerItem;
use leptoskit_native::Element;
pub use stack::Presentation;
pub use stack::Stack;
pub use stack::StackGesture;
pub use tabs::Tab;
pub use tabs::TabPlacement;
pub use tabs::Tabs;

use crate::Render;
use crate::Routable;
use crate::Router;

/// A directory's navigator, produced by [`IntoNavigator`].
#[derive(Clone, Debug)]
pub enum Navigator<R> {
	/// Push/pop history with optional modal and sheet presentations.
	Stack(Stack<R>),
	/// A bar of roots; the selected tab matches the current route by href.
	Tabs(Tabs<R>),
	/// A side panel of roots beside the current screen.
	Drawer(Drawer<R>),
}

impl<R> Navigator<R>
where
	R: Routable + Clone + PartialEq + Send + Sync + Render + 'static,
{
	/// Wraps `children` (the layout's output) with this navigator's chrome.
	pub fn shell(&self, children: Element, navigation: &Router<R>, route: &R) -> Element {
		match self {
			Self::Stack(stack) => stack.shell(children, navigation, route),
			Self::Tabs(tabs) => tabs.shell(children, navigation, route),
			Self::Drawer(drawer) => drawer.shell(children, navigation, route),
		}
	}
}

/// Converts a concrete navigator builder into a [`Navigator`].
pub trait IntoNavigator<R> {
	/// The type-erased navigator the router shells with.
	fn into_navigator(self) -> Navigator<R>;
}

impl<R> IntoNavigator<R> for Navigator<R> {
	fn into_navigator(self) -> Navigator<R> {
		self
	}
}

impl<R> IntoNavigator<R> for Stack<R> {
	fn into_navigator(self) -> Navigator<R> {
		Navigator::Stack(self)
	}
}

impl<R> IntoNavigator<R> for Tabs<R> {
	fn into_navigator(self) -> Navigator<R> {
		Navigator::Tabs(self)
	}
}

impl<R> IntoNavigator<R> for Drawer<R> {
	fn into_navigator(self) -> Navigator<R> {
		Navigator::Drawer(self)
	}
}

/// Whether `current` sits on `root` or under it, by href.
///
/// The app root `/` matches only itself (and a query on `/`), so a Today tab
/// does not steal `/notes`.
pub fn route_matches_root<R: Routable>(current: &R, root: &R) -> bool {
	href_matches_root(&current.href(), &root.href())
}

/// Href form of [`route_matches_root`].
pub fn href_matches_root(current: &crate::Href, root: &crate::Href) -> bool {
	let current = current.as_str();
	let root = root.as_str();

	if root == "/" {
		return current == "/" || current.starts_with("/?");
	}

	current == root
		|| current.starts_with(&format!("{root}/"))
		|| current.starts_with(&format!("{root}?"))
}
