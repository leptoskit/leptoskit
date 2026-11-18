//! Roving focus through a real keyboard pipeline: the terminal backend
//! routes keys, keeps a focus ring, and carries out focus requests.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use leptoskit_aria::Menu;
use leptoskit_aria::RadioGroup;
use leptoskit_aria::Tabs;
use leptoskit_native::Element;
use leptoskit_native::NativeSurface;
use leptoskit_native::components::Button;
use leptoskit_native::components::Text;
use leptoskit_native::components::helpers::Column;
use leptoskit_native::remember::ScreenState;
use leptoskit_tui::Exit;
use leptoskit_tui::Key;
use leptoskit_tui::KeyCode;
use leptoskit_tui::SourceEvent;
use leptoskit_tui::TuiBackend;
use leptoskit_tui::TuiHost;
use leptoskit_tui::TuiOptions;
use leptoskit_tui::testing::Recorder;
use leptoskit_tui::testing::Scripted;
use reactive_graph::owner::Owner;

fn plain(code: KeyCode) -> SourceEvent {
	SourceEvent::Key(Key::plain(code))
}

/// The row whose bracketed label draws reversed, as the focused control
/// does.
fn focused_row(frame: &leptoskit_tui::Buffer) -> Option<usize> {
	frame
		.to_plain_lines()
		.iter()
		.enumerate()
		.find_map(|(row, line)| {
			let column = u16::try_from(line[..line.find('[')?].chars().count()).ok()?;
			let reversed = frame
				.cell(column, u16::try_from(row).ok()?)?
				.attrs
				.reverse();

			reversed.then_some(row)
		})
}

/// Runs a screen through scripted keys, keeping its remembered state across
/// rebuilds as a running app does, and returns the last frame.
fn session(
	screen: impl Fn() -> Element + 'static,
	events: Vec<SourceEvent>,
) -> leptoskit_tui::Buffer {
	Owner::new().with(|| {
		let state = ScreenState::new();
		state.provide();
		let mut surface = NativeSurface::new(TuiBackend::new());
		let mut app =
			leptoskit_tui::testing::surface_app(&mut surface, move || state.build(&screen));
		let mut source = Scripted::new(events);
		let mut sink = Recorder::new(30, 8);
		let exit = TuiHost::new(TuiOptions::default())
			.run(&mut *app, &mut source, &mut sink)
			.expect("the session runs");

		assert_eq!(exit, Exit::SourceEnded);
		sink.frames.last().expect("a frame was drawn").clone()
	})
}

/// Three stacked tabs on rows 0 to 2, then a Save button on row 3.
fn tabs_screen(selected: &Arc<AtomicUsize>) -> Element {
	let tabs = Tabs::new("settings", 3, selected.load(Ordering::Relaxed))
		.orientation(leptoskit_native::Orientation::Vertical);
	let state = Arc::clone(selected);

	Column()
		.child(
			tabs.list(move |index| state.store(index, Ordering::Relaxed))
				.child(tabs.tab(0, "General"))
				.child(tabs.tab(1, "Notifications"))
				.child(tabs.tab(2, "Privacy")),
		)
		.child(Button("Save"))
		.children((0..3).map(|index| tabs.panel(index).child(Text(format!("Panel {index}")))))
}

fn tabs_session(events: Vec<SourceEvent>) -> (usize, Option<usize>) {
	let selected = Arc::new(AtomicUsize::new(0));
	let frame = session(
		{
			let selected = Arc::clone(&selected);
			move || tabs_screen(&selected)
		},
		events,
	);

	(selected.load(Ordering::Relaxed), focused_row(&frame))
}

#[test]
fn an_arrow_selects_the_next_tab_and_focus_goes_with_it() {
	let down = || plain(KeyCode::Down);

	assert_eq!(tabs_session(vec![plain(KeyCode::Tab)]), (0, Some(0)));
	assert_eq!(
		tabs_session(vec![plain(KeyCode::Tab), down()]),
		(1, Some(1))
	);
	assert_eq!(
		tabs_session(vec![plain(KeyCode::Tab), down(), down(), down()]),
		(0, Some(0)),
		"the last tab wraps to the first"
	);
	assert_eq!(
		tabs_session(vec![plain(KeyCode::Tab), plain(KeyCode::End)]),
		(2, Some(2))
	);
}

#[test]
fn a_tab_set_is_one_tab_stop() {
	let tab = || plain(KeyCode::Tab);

	// One Tab enters the set, the next leaves it for the button below.
	assert_eq!(tabs_session(vec![tab(), tab()]), (0, Some(3)));
	// Coming back lands on the selected tab, not the first.
	assert_eq!(
		tabs_session(vec![
			tab(),
			plain(KeyCode::Down),
			tab(),
			plain(KeyCode::BackTab)
		]),
		(1, Some(1))
	);
}

#[test]
fn arrows_choose_a_radio_card_and_focus_goes_with_it() {
	let chosen = Arc::new(AtomicUsize::new(usize::MAX));
	let screen = {
		let chosen = Arc::clone(&chosen);
		move || {
			let current = chosen.load(Ordering::Relaxed);
			let state = Arc::clone(&chosen);

			Column()
				.child(
					Element::from(
						RadioGroup::new(
							"persona",
							["Ada", "Grace"],
							(current != usize::MAX).then_some(current),
						)
						.option(0, Text("[Ada]"))
						.option(1, Text("[Grace]"))
						.on_change(move |index| state.store(index, Ordering::Relaxed)),
					)
					.aria_label("Persona"),
				)
				.child(Button("Save"))
		}
	};
	let frame = session(
		screen,
		vec![plain(KeyCode::Tab), plain(KeyCode::Up), plain(KeyCode::Tab)],
	);

	// Up from an unanswered group enters at the last option; Tab then leaves
	// the group in one step.
	assert_eq!(chosen.load(Ordering::Relaxed), 1);
	assert!(
		frame.to_plain_lines()[2].contains("Save"),
		"{:?}",
		frame.to_plain_lines()
	);
	assert_eq!(focused_row(&frame), Some(2));
}

/// What happened to the account menu.
#[derive(Default)]
struct Account {
	open: bool,
	chosen: Vec<&'static str>,
}

/// An account menu on row 0 with three actions, then a Save button.
fn account_screen(account: &Arc<Mutex<Account>>) -> Element {
	let open = account.lock().unwrap().open;
	let choose = |name: &'static str| {
		let account = Arc::clone(account);

		move || account.lock().unwrap().chosen.push(name)
	};
	let toggled = Arc::clone(account);

	Column()
		.child(
			Menu::new("account", open)
				.button(Button("Account"))
				.item("Profile", choose("profile"))
				.item("Billing", choose("billing"))
				.item("Sign out", choose("sign out"))
				.on_toggle(move |open| toggled.lock().unwrap().open = open)
				.into(),
		)
		.child(Text(""))
		.child(Text(""))
		.child(Text(""))
		.child(Button("Save"))
}

/// The text of the control that draws reversed or underlined, as the
/// focused button or pressable does.
fn focused_text(frame: &leptoskit_tui::Buffer) -> Option<String> {
	frame
		.to_plain_lines()
		.iter()
		.enumerate()
		.find_map(|(row, line)| {
			let row = u16::try_from(row).ok()?;
			let text = line
				.chars()
				.enumerate()
				.filter(|&(column, _)| {
					u16::try_from(column)
						.ok()
						.and_then(|column| frame.cell(column, row))
						.is_some_and(|cell| cell.attrs.reverse() || cell.attrs.underline())
				})
				.map(|(_, character)| character)
				.collect::<String>();

			(!text.trim().is_empty()).then(|| text.trim().to_owned())
		})
}

/// Runs keys against the account menu: whether it ends open, what was
/// chosen, and what holds focus.
fn account_session(events: Vec<SourceEvent>) -> (bool, Vec<&'static str>, String) {
	let account = Arc::new(Mutex::new(Account::default()));
	let frame = session(
		{
			let account = Arc::clone(&account);
			move || account_screen(&account)
		},
		events,
	);
	let account = account.lock().unwrap();

	(
		account.open,
		account.chosen.clone(),
		focused_text(&frame).unwrap_or_default(),
	)
}

#[test]
fn opening_a_menu_moves_focus_to_its_first_item() {
	let open = || vec![plain(KeyCode::Tab), plain(KeyCode::Enter)];

	assert_eq!(
		account_session(vec![plain(KeyCode::Tab)]),
		(false, vec![], "[ Account ]".to_owned())
	);
	assert_eq!(
		account_session(open()),
		(true, vec![], "Profile".to_owned())
	);
}

#[test]
fn arrows_move_through_a_menu_without_choosing() {
	let keys = |codes: &[KeyCode]| {
		[KeyCode::Tab, KeyCode::Enter]
			.iter()
			.chain(codes)
			.map(|&code| plain(code))
			.collect::<Vec<_>>()
	};

	assert_eq!(
		account_session(keys(&[KeyCode::Down])),
		(true, vec![], "Billing".to_owned())
	);
	assert_eq!(
		account_session(keys(&[KeyCode::Up])),
		(true, vec![], "Sign out".to_owned()),
		"the first item wraps to the last"
	);
	assert_eq!(
		account_session(keys(&[KeyCode::End, KeyCode::Down])),
		(true, vec![], "Profile".to_owned()),
		"the last item wraps to the first"
	);
	assert_eq!(
		account_session(keys(&[KeyCode::End, KeyCode::Home, KeyCode::Down])),
		(true, vec![], "Billing".to_owned())
	);
}

#[test]
fn choosing_an_item_runs_it_and_closes_the_menu() {
	assert_eq!(
		account_session(vec![
			plain(KeyCode::Tab),
			plain(KeyCode::Enter),
			plain(KeyCode::Down),
			plain(KeyCode::Enter),
		]),
		(false, vec!["billing"], "[ Account ]".to_owned()),
		"focus goes back to the button"
	);
}

#[test]
fn escape_and_tab_close_a_menu_without_choosing() {
	for key in [KeyCode::Escape, KeyCode::Tab, KeyCode::BackTab] {
		assert_eq!(
			account_session(vec![
				plain(KeyCode::Tab),
				plain(KeyCode::Enter),
				plain(KeyCode::Down),
				plain(key),
			]),
			(false, vec![], "[ Account ]".to_owned()),
			"{key:?} closes the menu and focus goes back to the button"
		);
	}
}

#[test]
fn a_closed_menu_keeps_its_items_out_of_the_tab_order() {
	assert_eq!(
		account_session(vec![plain(KeyCode::Tab), plain(KeyCode::Tab)]),
		(false, vec![], "[ Save ]".to_owned())
	);
}
