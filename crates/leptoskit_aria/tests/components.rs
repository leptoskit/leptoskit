//! Behavior tests for the headless components, over the real headless
//! renderer: press dispatch, reconciliation, validation, and semantics.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use leptoskit_aria::Dialog;
use leptoskit_aria::DisclosureButton;
use leptoskit_aria::DisclosurePanel;
use leptoskit_aria::Field;
use leptoskit_aria::Menu;
use leptoskit_aria::RadioGroup;
use leptoskit_aria::Tabs;
use leptoskit_aria::ToggleButton;
use leptoskit_native::Element;
use leptoskit_native::HasPopup;
use leptoskit_native::KeyPress;
use leptoskit_native::NativeKind;
use leptoskit_native::Orientation;
use leptoskit_native::Placement;
use leptoskit_native::Role;
use leptoskit_native::Tristate;
use leptoskit_native::components::Button;
use leptoskit_native::components::Text;
use leptoskit_native::components::TextInput;
use leptoskit_native::components::View;
use leptoskit_native::remember::ScreenState;
use leptoskit_testing::Finder;
use leptoskit_testing::TestError;
use leptoskit_testing::WidgetTester;
use reactive_graph::owner::Owner;

fn find_button() -> Finder {
	Finder::aria_role(Role::Button, "Frequently asked questions")
}

#[test]
fn disclosure_toggles_expanded_and_panel_visibility() -> Result<(), TestError> {
	let open = Arc::new(AtomicBool::new(false));
	let mut tester = WidgetTester::new(
		{
			let open = Arc::clone(&open);
			move || {
				let state = Arc::clone(&open);
				View()
					.child(DisclosureButton(
						"faq",
						"Frequently asked questions",
						open.load(Ordering::Relaxed),
						move |next| state.store(next, Ordering::Relaxed),
					))
					.child(
						DisclosurePanel("faq", open.load(Ordering::Relaxed))
							.child(Text("Only visible once expanded")),
					)
			}
		},
		390.0,
		844.0,
	)?;

	let button = tester.find(&find_button())?;
	assert_eq!(button.props.aria().expanded, Some(false));
	assert!(
		tester
			.find(&Finder::text("Only visible once expanded"))
			.is_err()
	);

	tester.tap(&find_button())?;

	let button = tester.find(&find_button())?;
	assert_eq!(button.props.aria().expanded, Some(true));
	tester.find(&Finder::text("Only visible once expanded"))?;

	tester.tap(&find_button())?;
	let button = tester.find(&find_button())?;
	assert_eq!(button.props.aria().expanded, Some(false));
	Ok(())
}

#[test]
fn disclosure_tree_without_panel_fails_validation() {
	let tree = View().child(DisclosureButton("faq", "FAQ", false, |_| ()));
	assert!(tree.validate().is_err(), "aria-controls needs its panel");
}

fn tristate_of(value: usize) -> Tristate {
	match value {
		0 => Tristate::False,
		1 => Tristate::True,
		_ => Tristate::Mixed,
	}
}

#[test]
fn toggle_button_walks_pressed_states() -> Result<(), TestError> {
	// 0 off, 1 on, 2 mixed: the walk starts from mixed.
	let pressed = Arc::new(AtomicUsize::new(2));
	let mut tester = WidgetTester::new(
		{
			let pressed = Arc::clone(&pressed);
			move || {
				let state = Arc::clone(&pressed);
				ToggleButton(
					"Mute",
					tristate_of(pressed.load(Ordering::Relaxed)),
					move |next| state.store(usize::from(next), Ordering::Relaxed),
				)
			}
		},
		390.0,
		844.0,
	)?;

	// Mixed proposes true, pressed proposes false, unpressed proposes true.
	for expected in [Tristate::True, Tristate::False, Tristate::True] {
		tester.tap(&Finder::aria_role(Role::Button, "Mute"))?;
		let button = tester.find(&Finder::aria_role(Role::Button, "Mute"))?;
		assert_eq!(button.props.aria().pressed, Some(expected));
	}

	Ok(())
}

/// A screen with three settings tabs, selecting whichever `selected` holds.
fn settings_tabs(selected: &Arc<AtomicUsize>, orientation: Orientation) -> Element {
	const LABELS: [&str; 3] = ["General", "Notifications", "Privacy"];
	let tabs = Tabs::new("settings", LABELS.len(), selected.load(Ordering::Relaxed))
		.orientation(orientation);
	let state = Arc::clone(selected);
	let list = tabs
		.list(move |index| state.store(index, Ordering::Relaxed))
		.children(
			LABELS
				.iter()
				.enumerate()
				.map(|(index, label)| tabs.tab(index, *label)),
		);

	View().child(list).child(
		View().children(
			LABELS
				.iter()
				.enumerate()
				.map(|(index, label)| tabs.panel(index).child(Text(format!("{label} panel")))),
		),
	)
}

/// Runs `test` over a screen whose remembered state lives across pumps, as
/// it does in a running app.
fn with_screen(
	screen: impl Fn() -> Element + 'static,
	test: impl FnOnce(&mut WidgetTester) -> Result<(), TestError>,
) -> Result<(), TestError> {
	Owner::new().with(|| {
		let state = ScreenState::new();
		state.provide();
		let mut tester = WidgetTester::new(move || state.build(&screen), 390.0, 844.0)?;

		test(&mut tester)
	})
}

#[test]
fn tabs_switch_panels_and_keep_one_tab_stop() -> Result<(), TestError> {
	let selected = Arc::new(AtomicUsize::new(0));
	let screen = {
		let selected = Arc::clone(&selected);
		move || settings_tabs(&selected, Orientation::Horizontal)
	};

	with_screen(screen, |tester| {
		let first = tester.find(&Finder::aria_role(Role::Tab, "General"))?;
		assert_eq!(first.props.aria().selected, Some(true));
		assert_eq!(first.props.aria().tab_index, Some(0));
		let second = tester.find(&Finder::aria_role(Role::Tab, "Notifications"))?;
		assert_eq!(second.props.aria().selected, Some(false));
		assert_eq!(second.props.aria().pos_in_set, Some(2));
		assert_eq!(second.props.aria().set_size, Some(3));
		// Only the selected tab is a Tab stop; the arrows reach the rest.
		assert_eq!(second.props.aria().tab_index, Some(-1));
		tester.find(&Finder::text("General panel"))?;
		assert!(tester.find(&Finder::text("Notifications panel")).is_err());

		tester.tap(&Finder::aria_role(Role::Tab, "Notifications"))?;

		let first = tester.find(&Finder::aria_role(Role::Tab, "General"))?;
		assert_eq!(first.props.aria().selected, Some(false));
		assert_eq!(first.props.aria().tab_index, Some(-1));
		let second = tester.find(&Finder::aria_role(Role::Tab, "Notifications"))?;
		assert_eq!(second.props.aria().selected, Some(true));
		assert_eq!(second.props.aria().tab_index, Some(0));
		// A press focuses the tab on its own; no request is needed.
		assert_eq!(second.props.focus_request, None);
		assert!(tester.find(&Finder::text("General panel")).is_err());
		tester.find(&Finder::text("Notifications panel"))?;
		Ok(())
	})
}

#[test]
fn arrow_keys_move_between_tabs_and_take_focus_along() -> Result<(), TestError> {
	let selected = Arc::new(AtomicUsize::new(0));
	let screen = {
		let selected = Arc::clone(&selected);
		move || settings_tabs(&selected, Orientation::Horizontal)
	};
	let tab = |name: &str| Finder::aria_role(Role::Tab, name);

	with_screen(screen, |tester| {
		tester.press_key(&tab("General"), KeyPress::new("ArrowRight"))?;
		let notifications = tester.find(&tab("Notifications"))?;
		assert_eq!(notifications.props.aria().selected, Some(true));
		let first_move = notifications.props.focus_request;
		assert!(first_move.is_some(), "focus follows the selection");
		assert_eq!(tester.find(&tab("General"))?.props.focus_request, None);

		// Left from the first tab wraps to the last, and End stays there.
		tester.press_key(&tab("Notifications"), KeyPress::new("ArrowLeft"))?;
		tester.press_key(&tab("General"), KeyPress::new("ArrowLeft"))?;
		let privacy = tester.find(&tab("Privacy"))?;
		assert_eq!(privacy.props.aria().selected, Some(true));
		assert!(privacy.props.focus_request > first_move);
		let at_the_end = privacy.props.focus_request;
		tester.press_key(&tab("Privacy"), KeyPress::new("End"))?;
		assert_eq!(
			tester.find(&tab("Privacy"))?.props.focus_request,
			at_the_end
		);

		tester.press_key(&tab("Privacy"), KeyPress::new("Home"))?;
		assert_eq!(
			tester.find(&tab("General"))?.props.aria().selected,
			Some(true)
		);

		// A horizontal set leaves the vertical arrows to the page.
		assert!(
			tester
				.press_key(&tab("General"), KeyPress::new("ArrowDown"))
				.is_err()
		);
		assert!(
			tester
				.press_key(&tab("General"), KeyPress::new("ArrowRight").ctrl())
				.is_err()
		);
		Ok(())
	})
}

#[test]
fn a_vertical_tab_set_moves_with_up_and_down() -> Result<(), TestError> {
	let selected = Arc::new(AtomicUsize::new(0));
	let screen = {
		let selected = Arc::clone(&selected);
		move || settings_tabs(&selected, Orientation::Vertical)
	};
	let tab = |name: &str| Finder::aria_role(Role::Tab, name);

	with_screen(screen, |tester| {
		tester.press_key(&tab("General"), KeyPress::new("ArrowDown"))?;
		assert_eq!(
			tester.find(&tab("Notifications"))?.props.aria().selected,
			Some(true)
		);
		tester.press_key(&tab("Notifications"), KeyPress::new("ArrowUp"))?;
		assert_eq!(
			tester.find(&tab("General"))?.props.aria().selected,
			Some(true)
		);
		assert!(
			tester
				.press_key(&tab("General"), KeyPress::new("ArrowRight"))
				.is_err()
		);
		Ok(())
	})
}

/// A persona chooser whose options are cards.
fn personas(chosen: &Arc<Mutex<Option<usize>>>, locked: bool) -> Element {
	let state = Arc::clone(chosen);
	let card = |name: &str, role: &str| View().child(Text(name)).child(Text(role));

	View().child(
		Element::from(
			RadioGroup::new(
				"persona",
				["Ada", "Grace", "Linus"],
				*chosen.lock().unwrap(),
			)
			.option(0, card("Ada", "Writes the engine"))
			.option(1, card("Grace", "Writes the compiler"))
			.on_change(move |index| {
				if !locked {
					*state.lock().unwrap() = Some(index);
				}
			}),
		)
		.aria_label("Persona"),
	)
}

#[test]
fn a_radio_group_of_cards_checks_one_option_and_keeps_one_tab_stop() -> Result<(), TestError> {
	let chosen = Arc::new(Mutex::new(None));
	let screen = {
		let chosen = Arc::clone(&chosen);
		move || personas(&chosen, false)
	};
	let radio = |name: &str| Finder::aria_role(Role::Radio, name);

	with_screen(screen, |tester| {
		tester.find(&Finder::aria_role(Role::RadioGroup, "Persona"))?;
		let ada = tester.find(&radio("Ada"))?;
		assert_eq!(ada.props.aria().checked, Some(Tristate::False));
		assert_eq!(ada.props.aria().pos_in_set, Some(1));
		assert_eq!(ada.props.aria().set_size, Some(3));
		// Nothing is checked, so Tab lands on the first option.
		assert_eq!(ada.props.aria().tab_index, Some(0));
		assert_eq!(
			tester.find(&radio("Grace"))?.props.aria().tab_index,
			Some(-1)
		);
		// An option given no content shows its label.
		tester.find(&Finder::text("Linus"))?;

		tester.tap(&radio("Grace"))?;
		let grace = tester.find(&radio("Grace"))?;
		assert_eq!(grace.props.aria().checked, Some(Tristate::True));
		assert_eq!(grace.props.aria().tab_index, Some(0));
		assert_eq!(tester.find(&radio("Ada"))?.props.aria().tab_index, Some(-1));

		// Any arrow moves, chooses, and takes focus along.
		tester.press_key(&radio("Grace"), KeyPress::new("ArrowDown"))?;
		let linus = tester.find(&radio("Linus"))?;
		assert_eq!(linus.props.aria().checked, Some(Tristate::True));
		assert!(linus.props.focus_request.is_some());
		tester.press_key(&radio("Linus"), KeyPress::new("ArrowRight"))?;
		assert_eq!(
			tester.find(&radio("Ada"))?.props.aria().checked,
			Some(Tristate::True)
		);
		tester.press_key(&radio("Ada"), KeyPress::new("ArrowUp"))?;
		assert_eq!(*chosen.lock().unwrap(), Some(2));
		Ok(())
	})
}

#[test]
fn an_unanswered_radio_group_is_entered_from_either_end() -> Result<(), TestError> {
	for (key, expected) in [("ArrowRight", 0), ("ArrowUp", 2), ("End", 2), ("Home", 0)] {
		let chosen = Arc::new(Mutex::new(None));
		let screen = {
			let chosen = Arc::clone(&chosen);
			move || personas(&chosen, false)
		};

		with_screen(screen, |tester| {
			tester.press_key(&Finder::aria_role(Role::Radio, "Ada"), KeyPress::new(key))
		})?;
		assert_eq!(*chosen.lock().unwrap(), Some(expected), "{key}");
	}

	Ok(())
}

#[test]
fn a_radio_group_that_keeps_its_state_refuses_the_choice() -> Result<(), TestError> {
	let chosen = Arc::new(Mutex::new(Some(0)));
	let screen = {
		let chosen = Arc::clone(&chosen);
		move || personas(&chosen, true)
	};
	let radio = |name: &str| Finder::aria_role(Role::Radio, name);

	with_screen(screen, |tester| {
		tester.press_key(&radio("Ada"), KeyPress::new("ArrowRight"))?;
		assert_eq!(
			tester.find(&radio("Ada"))?.props.aria().checked,
			Some(Tristate::True)
		);
		assert_eq!(
			tester.find(&radio("Grace"))?.props.aria().checked,
			Some(Tristate::False)
		);
		Ok(())
	})
}

#[test]
#[should_panic(expected = "selected option Some(2) is outside a group of 2")]
fn radio_groups_reject_a_selection_outside_the_group() {
	let _ = RadioGroup::new("persona", ["Ada", "Grace"], Some(2));
}

#[test]
#[should_panic(expected = "selected tab 3 is outside a set of 3")]
fn tabs_reject_a_selection_outside_the_set() {
	let _ = Tabs::new("settings", 3, 3);
}

#[test]
#[should_panic(expected = "tab 2 is outside a set of 2")]
fn tabs_reject_a_tab_outside_the_set() {
	let _ = Tabs::new("settings", 2, 0).tab(2, "Extra");
}

#[test]
fn field_wires_label_help_and_error_into_the_input() {
	let build = |error: Option<&'static str>| {
		let field = Field::new("email", "Email address").help("We never share your address.");

		let field = match error {
			Some(text) => field.error(text),
			None => field,
		};

		View()
			.child(field.label_element())
			.child(field.wire(TextInput()))
			.children(field.help_element())
			.children(field.error_element())
	};

	// Without an error: labelled by the label, described by the help alone,
	// valid, and no error element.
	let screen = build(None);
	assert!(screen.validate().is_ok());
	let input = &screen.children[1];
	assert_eq!(input.props.aria().labelled_by, ["email-label"]);
	assert_eq!(input.props.aria().described_by, ["email-help"]);
	assert_eq!(input.props.aria().invalid, None);
	assert_eq!(screen.children.len(), 3, "no error element when unset");

	// With one: the error joins the description and the value is invalid.
	let screen = build(Some("That address has no @."));
	assert!(screen.validate().is_ok());
	let input = &screen.children[1];
	assert_eq!(
		input.props.aria().described_by,
		["email-help", "email-error"]
	);
	assert_eq!(
		input.props.aria().invalid,
		Some(leptoskit_native::Invalid::True)
	);
	assert_eq!(
		screen.children[3].props.text.as_deref(),
		Some("That address has no @.")
	);
}

#[test]
fn field_without_label_element_fails_validation() {
	let field = Field::new("email", "Email address");
	// The wired input references a label id nothing carries.
	let tree = View().child(field.wire(TextInput()));
	assert!(tree.validate().is_err());
}

#[test]
fn field_keeps_descriptions_the_input_already_carries() {
	let field = Field::new("bio", "Biography").help("Shown on your profile.");
	let screen = View()
		.child(field.label_element())
		.child(Text("0 of 160 characters").id("bio-count"))
		.child(field.wire(TextInput().aria_described_by(["bio-count"])))
		.children(field.help_element());

	assert!(screen.validate().is_ok());
	let input = &screen.children[2];
	assert_eq!(input.props.id.as_deref(), Some("bio"));
	assert_eq!(input.props.aria().described_by, ["bio-count", "bio-help"]);
}

#[test]
fn dialog_opens_named_and_described() -> Result<(), TestError> {
	let open = Arc::new(AtomicBool::new(false));
	let dialog = Dialog::new("discard", "Discard draft?").description("This cannot be undone.");
	let mut tester = WidgetTester::new(
		{
			let open = Arc::clone(&open);
			let dialog = dialog.clone();
			move || {
				dialog
					.container(open.load(Ordering::Relaxed))
					.child(dialog.title_element(2))
					.children(dialog.description_element())
			}
		},
		390.0,
		844.0,
	)?;

	assert!(tester.find(&Finder::text("Discard draft?")).is_err());

	open.store(true, Ordering::Relaxed);
	tester.pump();
	tester.find(&Finder::text("Discard draft?"))?;
	tester.find(&Finder::text("This cannot be undone."))?;

	// The container carries its role and both references, and no modal
	// promise the renderer cannot keep without focus containment.
	let tree = dialog
		.container(true)
		.child(dialog.title_element(2))
		.children(dialog.description_element());
	assert_eq!(tree.props.id.as_deref(), Some("discard"));
	assert_eq!(tree.props.aria().role, Some(Role::Dialog));
	assert_eq!(tree.props.aria().modal, None);
	assert_eq!(tree.props.aria().labelled_by, ["discard-title"]);
	assert_eq!(tree.props.aria().described_by, ["discard-description"]);
	assert!(tree.validate().is_ok());
	Ok(())
}

#[test]
fn dialog_without_description_references_none() {
	let dialog = Dialog::new("discard", "Discard draft?");
	let tree = dialog.container(true).child(dialog.title_element(2));
	assert_eq!(tree.props.aria().described_by, [] as [String; 0]);
	assert!(dialog.description_element().is_none());
	assert!(tree.validate().is_ok());
}

#[test]
fn dialog_without_title_fails_validation() {
	let dialog = Dialog::new("discard", "Discard draft?");
	let tree = dialog.container(true).child(Text("This cannot be undone."));
	assert!(
		tree.validate().is_err(),
		"a dialog must be named by its title"
	);
}

#[test]
fn wiring_an_input_twice_keeps_one_of_each_description() {
	let field = Field::new("email", "Email address").help("We never share it.");
	let failing = field.clone().error("That address has no @.");

	let twice = field.wire(field.wire(TextInput().aria_described_by(["email-note"])));
	assert_eq!(
		twice.props.aria().described_by,
		["email-note", "email-help"]
	);

	// An error the field no longer shows leaves the description too.
	let fixed = field.wire(failing.wire(TextInput()));
	assert_eq!(fixed.props.aria().described_by, ["email-help"]);
}

#[test]
fn a_modal_dialog_is_a_named_modal() {
	let dialog = Dialog::new("discard", "Discard draft?").description("This cannot be undone.");
	let tree = dialog
		.modal(true)
		.child(dialog.title_element(2))
		.children(dialog.description_element());

	assert_eq!(tree.kind, NativeKind::Modal);
	assert_eq!(tree.props.id.as_deref(), Some("discard"));
	assert_eq!(
		tree.props.overlay.as_ref().map(|overlay| overlay.open),
		Some(true)
	);
	assert_eq!(tree.props.aria().labelled_by, ["discard-title"]);
	assert_eq!(tree.props.aria().described_by, ["discard-description"]);
	assert!(tree.validate().is_ok());
	assert_eq!(
		dialog
			.modal(false)
			.props
			.overlay
			.map(|overlay| overlay.open),
		Some(false)
	);
	assert!(
		dialog.modal(true).validate().is_err(),
		"a modal dialog must be named by its title"
	);
}

/// What the account menu was asked to do.
#[derive(Default)]
struct Account {
	open: bool,
	chosen: Vec<&'static str>,
}

fn account_menu(account: &Arc<Mutex<Account>>) -> Element {
	let open = account.lock().unwrap().open;
	let choose = |name: &'static str| {
		let account = Arc::clone(account);

		move || account.lock().unwrap().chosen.push(name)
	};
	let toggled = Arc::clone(account);

	View().child(
		Menu::new("account", open)
			.button(Button("Account"))
			.item("Profile", choose("profile"))
			.item("Sign out", choose("sign out"))
			.placement(Placement::BelowEnd)
			.on_toggle(move |open| toggled.lock().unwrap().open = open)
			.into(),
	)
}

#[test]
fn a_menu_wires_its_button_to_its_list() -> Result<(), TestError> {
	let account = Arc::new(Mutex::new(Account::default()));
	let screen = {
		let account = Arc::clone(&account);
		move || account_menu(&account)
	};

	with_screen(screen, |tester| {
		let button = tester.find(&Finder::aria_role(Role::Button, "Account"))?;
		assert_eq!(button.props.id.as_deref(), Some("account-button"));
		assert_eq!(button.props.aria().has_popup, Some(HasPopup::Menu));
		assert_eq!(button.props.aria().expanded, Some(false));
		assert_eq!(button.props.aria().controls, ["account-menu"]);
		assert!(
			tester.find(&Finder::text("Profile")).is_err(),
			"a closed menu shows nothing"
		);

		tester.tap(&Finder::aria_role(Role::Button, "Account"))?;

		let button = tester.find(&Finder::aria_role(Role::Button, "Account"))?;
		assert_eq!(button.props.aria().expanded, Some(true));
		let first = tester.find(&Finder::aria_role(Role::MenuItem, "Profile"))?;
		assert_eq!(first.props.id.as_deref(), Some("account-item-0"));
		// Items are reached with the arrow keys, not with Tab.
		assert_eq!(first.props.aria().tab_index, Some(-1));

		tester.tap(&Finder::aria_role(Role::MenuItem, "Sign out"))?;

		assert!(tester.find(&Finder::text("Sign out")).is_err());
		Ok(())
	})?;

	let account = account.lock().unwrap();
	assert!(!account.open, "choosing an item closes the menu");
	assert_eq!(account.chosen, ["sign out"]);
	Ok(())
}

#[test]
fn a_menu_list_is_a_popover_hanging_from_its_button() {
	Owner::new().with(|| {
		let state = ScreenState::new();
		state.provide();
		let tree = state.build(|| {
			Element::from(
				Menu::new("account", true)
					.item("Profile", || ())
					.item("Sign out", || ())
					.placement(Placement::BelowEnd),
			)
		});
		let list = &tree.children[1];
		let overlay = list.props.overlay.as_ref().expect("the list is an overlay");

		assert_eq!(list.kind, NativeKind::Popover);
		assert_eq!(list.props.aria().role, Some(Role::Menu));
		assert_eq!(list.props.aria().labelled_by, ["account-button"]);
		assert_eq!(list.props.aria().orientation, Some(Orientation::Vertical));
		assert_eq!(overlay.anchor.as_deref(), Some("account-button"));
		assert_eq!(overlay.placement, Placement::BelowEnd);
		assert_eq!(overlay.initial_focus.as_deref(), Some("account-item-0"));
		assert!(View().child(tree).validate().is_ok());
	});
}

#[test]
fn arrows_in_a_menu_ask_for_focus_on_the_next_item() -> Result<(), TestError> {
	let account = Arc::new(Mutex::new(Account {
		open: true,
		chosen: Vec::new(),
	}));
	let screen = {
		let account = Arc::clone(&account);
		move || account_menu(&account)
	};

	with_screen(screen, |tester| {
		let second = Finder::aria_role(Role::MenuItem, "Sign out");
		assert_eq!(tester.find(&second)?.props.focus_request, None);

		tester.press_key(
			&Finder::aria_role(Role::MenuItem, "Profile"),
			KeyPress::new("ArrowDown"),
		)?;

		assert!(tester.find(&second)?.props.focus_request.is_some());

		tester.press_key(&second, KeyPress::new("Tab"))?;

		assert!(tester.find(&Finder::text("Sign out")).is_err());
		Ok(())
	})?;

	let account = account.lock().unwrap();
	assert!(!account.open, "Tab closes the menu");
	assert!(
		account.chosen.is_empty(),
		"moving through a menu runs nothing"
	);
	Ok(())
}
