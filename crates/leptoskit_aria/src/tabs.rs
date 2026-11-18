//! Tabs: a list of tabs and the panels they select.

use std::fmt;
use std::sync::Arc;
use std::sync::OnceLock;

use leptoskit_native::Element;
use leptoskit_native::EventOutcome;
use leptoskit_native::FocusRequest;
use leptoskit_native::Orientation;
use leptoskit_native::Role;
use leptoskit_yoga::Display;

use crate::roving;

/// What a tab set calls when a tab is chosen.
type OnSelect = Arc<dyn Fn(usize) + Send + Sync>;

/// One tab set: how many tabs it has and which one is selected.
///
/// Every tab and panel comes from the same `Tabs`, so the wiring cannot
/// disagree: exactly one tab is selected, every tab reports the same set
/// size, and each tab's `aria-controls` names the panel built for the same
/// index. Build a fresh `Tabs` on every render from current state, the way
/// [`Field`](crate::Field) is built.
///
/// The set is one Tab stop, as the WAI-ARIA tabs pattern asks. Only the
/// selected tab is in the Tab order; the arrow keys along the set's
/// [`orientation`](Self::orientation), Home and End move to another tab,
/// select it, and take focus with them. Enter and Space activate the focused
/// tab.
///
/// The web and the terminal deliver key presses. The other backends do not
/// yet, and they also ignore `tabindex`, so every tab stays reachable with
/// Tab there, as before.
///
/// ```
/// use leptoskit_aria::Tabs;
/// use leptoskit_native::components::Text;
/// use leptoskit_native::components::View;
///
/// let tabs = Tabs::new("settings", 2, 0);
/// let screen = View()
/// 	.child(
/// 		tabs.list(|_| ())
/// 			.child(tabs.tab(0, "General"))
/// 			.child(tabs.tab(1, "Notifications")),
/// 	)
/// 	.child(tabs.panel(0).child(Text("General settings")))
/// 	.child(tabs.panel(1).child(Text("Notification choices")));
///
/// let general = &screen.children[0].children[0];
/// assert_eq!(general.props.id.as_deref(), Some("settings-tab-0"));
/// assert_eq!(general.props.aria().selected, Some(true));
/// assert_eq!(general.props.aria().controls, ["settings-panel-0"]);
/// assert_eq!(general.props.aria().set_size, Some(2));
/// assert_eq!(general.props.aria().tab_index, Some(0));
/// assert_eq!(
/// 	screen.children[0].children[1].props.aria().tab_index,
/// 	Some(-1)
/// );
/// assert!(screen.validate().is_ok());
/// ```
#[derive(Clone)]
pub struct Tabs {
	id: String,
	count: usize,
	selected: usize,
	orientation: Orientation,
	/// Moves focus to the selected tab after an arrow key selects it.
	focus: FocusRequest,
	/// The callback [`Self::list`] was given, which each tab's press calls.
	on_select: Arc<OnceLock<OnSelect>>,
}

impl fmt::Debug for Tabs {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter
			.debug_struct("Tabs")
			.field("id", &self.id)
			.field("count", &self.count)
			.field("selected", &self.selected)
			.field("orientation", &self.orientation)
			.finish_non_exhaustive()
	}
}

impl Tabs {
	/// A horizontal tab set of `count` tabs under the id prefix `id`, with
	/// `selected` active.
	///
	/// # Panics
	///
	/// When `selected` is not below `count`: a tab set always has exactly
	/// one selected tab.
	#[track_caller]
	pub fn new(id: impl Into<String>, count: usize, selected: usize) -> Self {
		assert!(
			selected < count,
			"selected tab {selected} is outside a set of {count}"
		);
		Self {
			id: id.into(),
			count,
			selected,
			orientation: Orientation::Horizontal,
			focus: FocusRequest::new(),
			on_select: Arc::default(),
		}
	}

	/// Lays the set out along `orientation`: a horizontal set moves with the
	/// left and right arrows, a vertical one with up and down. The list
	/// reports it as `aria-orientation`.
	#[must_use]
	pub const fn orientation(mut self, orientation: Orientation) -> Self {
		self.orientation = orientation;
		self
	}

	/// The index of the selected tab.
	#[must_use]
	pub const fn selected(&self) -> usize {
		self.selected
	}

	/// The `tablist` container for this set's tabs. Lay the tabs out with
	/// Tailwind classes; the list contributes the role, the orientation and
	/// the arrow keys.
	///
	/// `on_select` receives a tab's index when it is pressed, or when an
	/// arrow key, Home or End moves to it. Store the index and build the
	/// next `Tabs` from it.
	#[must_use]
	pub fn list(&self, on_select: impl Fn(usize) + Send + Sync + 'static) -> Element {
		let on_select: OnSelect = Arc::new(on_select);
		// The first list wins; a set has one.
		let _ = self.on_select.set(Arc::clone(&on_select));
		let (count, selected, orientation, focus) =
			(self.count, self.selected, self.orientation, self.focus);

		Element::view()
			.role(Role::TabList)
			.aria_orientation(orientation)
			.on_key(move |key| {
				let Some(next) = roving::target(key, Some(orientation), selected, count) else {
					return EventOutcome::Unhandled;
				};

				if next != selected {
					on_select(next);
					focus.request();
				}

				EventOutcome::Handled
			})
	}

	/// The tab at `index`, labelled `label`. Pressing it calls the callback
	/// [`Self::list`] was given.
	///
	/// # Panics
	///
	/// When `index` is not below the set's count.
	#[must_use]
	pub fn tab(&self, index: usize, label: impl Into<String>) -> Element {
		self.check(index);
		let on_select = Arc::clone(&self.on_select);
		let current = index == self.selected;
		let tab = Element::button(label)
			.id(self.tab_id(index))
			.role(Role::Tab)
			.aria_selected(current)
			.aria_controls([self.panel_id(index)])
			.aria_pos_in_set(position(index))
			.aria_set_size(position(self.count - 1))
			.tab_index(roving::tab_index(current))
			.on_press(move || {
				if let Some(on_select) = on_select.get() {
					on_select(index);
				}
			});

		if current {
			tab.focus_on(self.focus)
		} else {
			tab
		}
	}

	/// The panel the tab at `index` selects, named after that tab.
	///
	/// Panels stay mounted with the inactive ones collapsed, so every tab's
	/// `aria-controls` resolves and switching tabs keeps panel state.
	///
	/// # Panics
	///
	/// When `index` is not below the set's count.
	#[must_use]
	pub fn panel(&self, index: usize) -> Element {
		self.check(index);
		let panel = Element::view()
			.id(self.panel_id(index))
			.role(Role::TabPanel)
			.aria_labelled_by([self.tab_id(index)]);

		if index == self.selected {
			panel
		} else {
			panel.with_style(|style| style.display = Display::None)
		}
	}

	fn check(&self, index: usize) {
		assert!(
			index < self.count,
			"tab {index} is outside a set of {}",
			self.count
		);
	}

	fn tab_id(&self, index: usize) -> String {
		format!("{}-tab-{index}", self.id)
	}

	fn panel_id(&self, index: usize) -> String {
		format!("{}-panel-{index}", self.id)
	}
}

/// The one-based `aria-posinset` of a zero-based index.
fn position(index: usize) -> u32 {
	u32::try_from(index + 1).expect("a tab set fits in u32 positions")
}
