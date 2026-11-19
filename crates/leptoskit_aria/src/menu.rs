//! Menu: a button that opens a list of actions.

use std::fmt;
use std::sync::Arc;

use leptoskit_native::Element;
use leptoskit_native::EventOutcome;
use leptoskit_native::FocusRequest;
use leptoskit_native::HasPopup;
use leptoskit_native::Orientation;
use leptoskit_native::Placement;
use leptoskit_native::Role;
use leptoskit_native::components::Popover;
use leptoskit_native::components::Pressable;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;

use crate::roving;

/// What a menu calls when it should open or close.
type OnToggle = Arc<dyn Fn(bool) + Send + Sync>;

/// One action in a menu.
struct Item {
	label: String,
	content: Option<Element>,
	on_select: Arc<dyn Fn() + Send + Sync>,
	/// Moves focus to this item when an arrow key reaches it.
	focus: FocusRequest,
}

/// A button that opens a list of actions, such as an account menu.
///
/// The menu is a [`Popover`] hanging from the button. Opening it moves
/// focus to the first item. Up and Down move between items and wrap, Home
/// and End go to the first and last, and Enter or Space runs the focused
/// item. Choosing an item, Escape, Tab, or a press outside closes the menu,
/// and focus goes back to the button. Moving through a menu never runs an
/// item.
///
/// The button is any control you pass: the menu adds its `id`, says that it
/// opens a menu and whether the menu is open, and toggles on a press. The
/// app owns the open state. [`Menu::on_toggle`] is told when it should
/// change.
///
/// Where each part works follows [`Popover`] and key delivery: all of it on
/// the web and in the terminal. On the other backends the menu shows and
/// its items run, but the arrow keys, Escape and a press outside do nothing
/// yet, so the button and the items are how it closes.
///
/// ```
/// use leptoskit_aria::Menu;
/// use leptoskit_native::Element;
/// use leptoskit_native::components::Button;
/// use leptoskit_native::components::View;
///
/// let open = true;
/// let menu = Element::from(
/// 	Menu::new("account", open)
/// 		.button(Button("Account"))
/// 		.item("Open the admin", || ())
/// 		.item("Sign out", || ())
/// 		.on_toggle(|_open| ()),
/// );
///
/// let (button, list) = (&menu.children[0], &menu.children[1]);
/// assert_eq!(button.props.id.as_deref(), Some("account-button"));
/// assert_eq!(button.props.aria().expanded, Some(true));
/// assert_eq!(button.props.aria().controls, ["account-menu"]);
/// assert_eq!(list.props.aria().labelled_by, ["account-button"]);
/// assert_eq!(list.children[1].props.id.as_deref(), Some("account-item-1"));
/// assert!(View().child(menu).validate().is_ok());
/// ```
pub struct Menu {
	id: String,
	open: bool,
	button: Option<Element>,
	items: Vec<Item>,
	placement: Placement,
	on_toggle: Option<OnToggle>,
}

impl fmt::Debug for Menu {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter
			.debug_struct("Menu")
			.field("id", &self.id)
			.field("open", &self.open)
			.field(
				"items",
				&self
					.items
					.iter()
					.map(|item| item.label.as_str())
					.collect::<Vec<_>>(),
			)
			.finish_non_exhaustive()
	}
}

impl Menu {
	/// A menu under the id prefix `id`, showing its items while `open`.
	pub fn new(id: impl Into<String>, open: bool) -> Self {
		Self {
			id: id.into(),
			open,
			button: None,
			items: Vec::new(),
			placement: Placement::default(),
			on_toggle: None,
		}
	}

	/// The control that opens the menu: a `Button`, or a `Pressable` around
	/// an avatar. The menu gives it the id `{id}-button`.
	#[must_use]
	pub fn button(mut self, button: Element) -> Self {
		self.button = Some(button);
		self
	}

	/// Adds an action named `label`. `on_select` runs when the item is
	/// pressed, or when Enter or Space is pressed on it; the menu then asks
	/// to close.
	#[must_use]
	#[track_caller]
	pub fn item(
		mut self,
		label: impl Into<String>,
		on_select: impl Fn() + Send + Sync + 'static,
	) -> Self {
		self.items.push(Item {
			label: label.into(),
			content: None,
			on_select: Arc::new(on_select),
			focus: FocusRequest::new(),
		});
		self
	}

	/// Shows `content` in the last item added, in place of its label as
	/// text: an icon beside the words, a second line. The label still names
	/// the item for assistive technology.
	///
	/// # Panics
	///
	/// When no item has been added.
	#[must_use]
	pub fn content(mut self, content: Element) -> Self {
		self.items
			.last_mut()
			.expect("add an item before giving it content")
			.content = Some(content);
		self
	}

	/// Which side of the button the menu opens on. It opens below the
	/// button, start edges aligned, unless told otherwise.
	#[must_use]
	pub fn placement(mut self, placement: Placement) -> Self {
		self.placement = placement;
		self
	}

	/// Calls `on_toggle` with whether the menu should be open: `true` when
	/// the button is pressed while closed, `false` when the button is
	/// pressed while open, an item is chosen, or the person dismisses the
	/// menu. Store the value and build the next menu from it.
	#[must_use]
	pub fn on_toggle(mut self, on_toggle: impl Fn(bool) + Send + Sync + 'static) -> Self {
		self.on_toggle = Some(Arc::new(on_toggle));
		self
	}
}

impl From<Menu> for Element {
	fn from(menu: Menu) -> Self {
		let Menu {
			id,
			open,
			button,
			items,
			placement,
			on_toggle,
		} = menu;
		let button_id = format!("{id}-button");
		let menu_id = format!("{id}-menu");
		let item_id = |index: usize| format!("{id}-item-{index}");
		let toggle = move |open: bool| {
			let on_toggle = on_toggle.clone();

			move || {
				if let Some(on_toggle) = &on_toggle {
					on_toggle(open);
				}
			}
		};
		let button = button
			.unwrap_or_else(|| Pressable("Menu").child(Text("Menu")))
			.id(button_id.clone())
			.aria_has_popup(HasPopup::Menu)
			.aria_expanded(open)
			.aria_controls([menu_id.clone()])
			.on_press(toggle(!open));
		let count = items.len();
		let requests: Arc<[FocusRequest]> = items.iter().map(|item| item.focus).collect();
		let items = items.into_iter().enumerate().map(|(index, item)| {
			let Item {
				label,
				content,
				on_select,
				focus,
			} = item;
			let content = content.unwrap_or_else(|| Text(label.clone()));
			let close = toggle(false);
			let leave = toggle(false);
			let requests = Arc::clone(&requests);

			Pressable(label)
				.id(item_id(index))
				.role(Role::MenuItem)
				// Items are reached with the arrow keys, not with Tab.
				.tab_index(-1)
				.focus_on(focus)
				.on_press(move || {
					on_select();
					close();
				})
				.on_key(move |key| {
					if key.key == "Tab" && !key.ctrl && !key.alt && !key.meta {
						leave();

						return EventOutcome::Handled;
					}

					let Some(next) = roving::target(key, Some(Orientation::Vertical), index, count)
					else {
						return EventOutcome::Unhandled;
					};

					requests[next].request();
					EventOutcome::Handled
				})
				.child(content)
		});
		let list = Popover(button_id.clone(), open)
			.id(menu_id)
			.role(Role::Menu)
			.aria_labelled_by([button_id])
			.aria_orientation(Orientation::Vertical)
			.placement(placement)
			.on_dismiss(toggle(false))
			.children(items);
		let list = if count > 0 {
			list.initial_focus(item_id(0))
		} else {
			list
		};

		View().child(button).child(list)
	}
}
