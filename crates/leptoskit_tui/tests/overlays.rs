//! Modals and popovers in the terminal: drawn over the screen, with focus
//! kept inside a modal and given back when an overlay closes.

use std::sync::Arc;
use std::sync::Mutex;

use leptoskit_native::Element;
use leptoskit_native::NativeSurface;
use leptoskit_native::components::Button;
use leptoskit_native::components::Modal;
use leptoskit_native::components::Popover;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;
use leptoskit_native::components::helpers::Column;
use leptoskit_tui::Buffer;
use leptoskit_tui::Exit;
use leptoskit_tui::Key;
use leptoskit_tui::KeyCode;
use leptoskit_tui::Mouse;
use leptoskit_tui::SourceEvent;
use leptoskit_tui::TuiBackend;
use leptoskit_tui::TuiHost;
use leptoskit_tui::TuiOptions;
use leptoskit_tui::testing::Recorder;
use leptoskit_tui::testing::Scripted;

fn plain(code: KeyCode) -> SourceEvent {
	SourceEvent::Key(Key::plain(code))
}

fn click(col: u16, row: u16) -> SourceEvent {
	SourceEvent::Mouse(Mouse::Click { col, row })
}

fn session(screen: impl FnMut() -> Element + 'static, events: Vec<SourceEvent>) -> Recorder {
	let mut surface = NativeSurface::new(TuiBackend::new());
	let mut app = leptoskit_tui::testing::surface_app(&mut surface, screen);
	let mut source = Scripted::new(events);
	let mut sink = Recorder::new(32, 8);

	let exit = TuiHost::new(TuiOptions::default())
		.run(&mut *app, &mut source, &mut sink)
		.expect("session runs");

	assert_eq!(exit, Exit::SourceEnded);
	sink
}

/// Where a label is drawn, as `(column, row)`.
fn find(frame: &Buffer, label: &str) -> Option<(u16, u16)> {
	frame
		.to_plain_lines()
		.iter()
		.enumerate()
		.find_map(|(row, line)| {
			let at = line.find(label)?;
			let column = line[..at].chars().count();

			Some((u16::try_from(column).ok()?, u16::try_from(row).ok()?))
		})
}

/// Whether the control showing `label` draws reversed, as the focused one
/// does.
fn is_focused(frame: &Buffer, label: &str) -> bool {
	let (column, row) = find(frame, label).unwrap_or_else(|| panic!("`{label}` is on screen"));

	frame.cell(column, row).expect("a cell").attrs.reverse()
}

/// What the person did, in order.
#[derive(Default)]
struct Page {
	open: bool,
	log: Vec<&'static str>,
}

/// A page with a button that opens a confirmation.
fn confirm(page: &Arc<Mutex<Page>>, initial_focus: bool) -> impl FnMut() -> Element + 'static {
	let page = Arc::clone(page);

	move || {
		let open = page.lock().unwrap().open;
		let set = |open: bool, note: &'static str| {
			let page = Arc::clone(&page);

			move || {
				let mut page = page.lock().unwrap();
				page.open = open;
				page.log.push(note);
			}
		};
		let modal = Modal(open)
			.aria_label("Delete the note?")
			.on_dismiss(set(false, "dismissed"))
			.child(
				Column()
					.class("bg-slate-800 p-1")
					.child(Text("Delete the note?"))
					.child(Button("Remove").on_press(set(false, "removed")))
					.child(Button("Keep").id("keep").on_press(set(false, "kept"))),
			);

		Column()
			.child(Button("Open").on_press(set(true, "opened")))
			.child(Button("Other").on_press(set(open, "other")))
			.child(if initial_focus {
				modal.initial_focus("keep")
			} else {
				modal
			})
	}
}

#[test]
fn a_modal_draws_over_the_screen_and_takes_focus() {
	let page = Arc::new(Mutex::new(Page::default()));
	let sink = session(
		confirm(&page, false),
		vec![plain(KeyCode::Tab), plain(KeyCode::Enter)],
	);
	let frame = sink.frames.last().unwrap();

	assert!(
		find(frame, "Delete the note?").is_some(),
		"{:#?}",
		sink.lines()
	);
	// The first control inside has focus, and the one that opened it lost it.
	assert!(is_focused(frame, "Remove"));
	assert!(find(frame, "Open").is_none_or(|_| !is_focused(frame, "Open")));
}

#[test]
fn a_closed_modal_draws_nothing() {
	let page = Arc::new(Mutex::new(Page::default()));
	let sink = session(confirm(&page, false), vec![plain(KeyCode::Tab)]);

	assert!(find(sink.frames.last().unwrap(), "Delete the note?").is_none());
}

#[test]
fn tab_stays_inside_an_open_modal() {
	let page = Arc::new(Mutex::new(Page::default()));
	session(
		confirm(&page, false),
		vec![
			plain(KeyCode::Tab),
			plain(KeyCode::Enter),
			// Remove, Keep, and round to Remove again: never the page.
			plain(KeyCode::Tab),
			plain(KeyCode::Tab),
			plain(KeyCode::Enter),
		],
	);

	assert_eq!(page.lock().unwrap().log, ["opened", "removed"]);
}

#[test]
fn a_modal_gives_focus_to_the_element_it_names() {
	let page = Arc::new(Mutex::new(Page::default()));
	let sink = session(
		confirm(&page, true),
		vec![plain(KeyCode::Tab), plain(KeyCode::Enter)],
	);
	let frame = sink.frames.last().unwrap();

	assert!(is_focused(frame, "Keep"));
	assert!(!is_focused(frame, "Remove"));
}

#[test]
fn escape_dismisses_a_modal_and_focus_goes_back() {
	let page = Arc::new(Mutex::new(Page::default()));
	let sink = session(
		confirm(&page, false),
		vec![
			plain(KeyCode::Tab),
			plain(KeyCode::Enter),
			plain(KeyCode::Escape),
		],
	);
	let frame = sink.frames.last().unwrap();

	assert_eq!(page.lock().unwrap().log, ["opened", "dismissed"]);
	assert!(find(frame, "Delete the note?").is_none());
	assert!(is_focused(frame, "Open"), "{:#?}", sink.lines());
}

#[test]
fn a_press_on_the_backdrop_dismisses_and_reaches_nothing_behind() {
	let page = Arc::new(Mutex::new(Page::default()));
	let sink = session(
		confirm(&page, false),
		vec![
			plain(KeyCode::Tab),
			plain(KeyCode::Enter),
			// The corner is the page's Open button, behind the backdrop.
			click(0, 0),
		],
	);

	assert_eq!(page.lock().unwrap().log, ["opened", "dismissed"]);
	assert!(find(sink.frames.last().unwrap(), "Delete the note?").is_none());
}

#[test]
fn a_press_inside_a_modal_is_not_a_dismissal() {
	let page = Arc::new(Mutex::new(Page::default()));
	let opened = session(
		confirm(&page, false),
		vec![plain(KeyCode::Tab), plain(KeyCode::Enter)],
	);
	let (column, row) = find(opened.frames.last().unwrap(), "Keep").expect("Keep is on screen");
	*page.lock().unwrap() = Page::default();

	session(
		confirm(&page, false),
		vec![
			plain(KeyCode::Tab),
			plain(KeyCode::Enter),
			click(column, row),
		],
	);

	assert_eq!(page.lock().unwrap().log, ["opened", "kept"]);
}

/// A page with a menu button, and something else to press.
fn menu(page: &Arc<Mutex<Page>>) -> impl FnMut() -> Element + 'static {
	let page = Arc::clone(page);

	move || {
		let open = page.lock().unwrap().open;
		let note = |note: &'static str, change: fn(bool) -> bool| {
			let page = Arc::clone(&page);

			move || {
				let mut page = page.lock().unwrap();
				page.open = change(page.open);
				page.log.push(note);
			}
		};

		View()
			.class("items-start")
			.child(
				Button("Menu")
					.id("menu")
					.on_press(note("menu", |open| !open)),
			)
			.child(Text("Line one"))
			.child(Text("Line two"))
			.child(Text("Line three"))
			.child(Button("Other").on_press(note("other", |open| open)))
			.child(
				Popover("menu", open)
					.on_dismiss(note("dismissed", |_| false))
					.child(
						Column()
							.class("bg-slate-800")
							.child(Button("Profile").on_press(note("profile", |_| false)))
							.child(Button("Sign out").on_press(note("sign out", |_| false))),
					),
			)
	}
}

#[test]
fn a_popover_opens_under_its_anchor_with_focus_inside() {
	let page = Arc::new(Mutex::new(Page::default()));
	let sink = session(
		menu(&page),
		vec![plain(KeyCode::Tab), plain(KeyCode::Enter)],
	);
	let frame = sink.frames.last().unwrap();
	let anchor = find(frame, "Menu").expect("the anchor is on screen");
	let first = find(frame, "Profile").expect("the popover is on screen");

	// Directly below the anchor, over the text that was there.
	assert_eq!(first.1, anchor.1 + 1, "{:#?}", sink.lines());
	assert!(find(frame, "Line one").is_none(), "{:#?}", sink.lines());
	assert!(is_focused(frame, "Profile"));
}

#[test]
fn escape_closes_a_popover_and_focus_goes_back_to_its_anchor() {
	let page = Arc::new(Mutex::new(Page::default()));
	let sink = session(
		menu(&page),
		vec![
			plain(KeyCode::Tab),
			plain(KeyCode::Enter),
			plain(KeyCode::Escape),
		],
	);
	let frame = sink.frames.last().unwrap();

	assert_eq!(page.lock().unwrap().log, ["menu", "dismissed"]);
	assert!(find(frame, "Profile").is_none());
	assert!(find(frame, "Line one").is_some());
	assert!(is_focused(frame, "Menu"));
}

#[test]
fn a_press_outside_a_popover_closes_it_and_still_lands() {
	let page = Arc::new(Mutex::new(Page::default()));
	let opened = session(
		menu(&page),
		vec![plain(KeyCode::Tab), plain(KeyCode::Enter)],
	);
	let (column, row) = find(opened.frames.last().unwrap(), "Other").expect("Other is on screen");
	*page.lock().unwrap() = Page::default();

	session(
		menu(&page),
		vec![
			plain(KeyCode::Tab),
			plain(KeyCode::Enter),
			click(column, row),
		],
	);

	assert_eq!(page.lock().unwrap().log, ["menu", "dismissed", "other"]);
}

#[test]
fn a_press_on_the_anchor_is_left_to_the_anchor() {
	let page = Arc::new(Mutex::new(Page::default()));
	let opened = session(
		menu(&page),
		vec![plain(KeyCode::Tab), plain(KeyCode::Enter)],
	);
	let (column, row) = find(opened.frames.last().unwrap(), "Menu").expect("Menu is on screen");
	*page.lock().unwrap() = Page::default();

	session(
		menu(&page),
		vec![
			plain(KeyCode::Tab),
			plain(KeyCode::Enter),
			click(column, row),
		],
	);

	// The button closes its own menu: no dismissal reopens it.
	let page = page.lock().unwrap();
	assert_eq!(page.log, ["menu", "menu"]);
	assert!(!page.open);
}
