//! The notebook's screen mounts, its page loads with the sketch it ships,
//! and its buttons change the document.

use std::future::Future;

use leptoskit::remember::ScreenState;
use leptoskit::view;
use leptoskit_canvist::Format;
use leptoskit_testing::Finder;
use leptoskit_testing::WidgetTester;
use reactive_graph::owner::Owner;
use reactive_graph::traits::Get;
use sketchnote_example::COLOURS;
use sketchnote_example::NIBS;
use sketchnote_example::Notebook;
use sketchnote_example::PAGE;
use sketchnote_example::Sketchnote;
use sketchnote_example::Summary;

/// Run a future that is ready at once, as the editor's are wherever the
/// document lives in the app's own process.
fn now<T>(future: impl Future<Output = T>) -> T {
	let mut future = std::pin::pin!(future);
	let mut context = std::task::Context::from_waker(std::task::Waker::noop());

	match future.as_mut().poll(&mut context) {
		std::task::Poll::Ready(value) => value,
		std::task::Poll::Pending => panic!("the document is in this process"),
	}
}

fn mount(notebook: Notebook) -> WidgetTester {
	let owner = Owner::new();
	let state = owner.with(ScreenState::new);
	owner.with(|| state.provide());
	WidgetTester::new(
		move || owner.with(|| state.build(|| view! { <Sketchnote notebook=notebook /> })),
		800.0,
		900.0,
	)
	.expect("the screen mounts")
}

#[test]
fn the_page_it_ships_is_a_document_with_one_sketch() {
	let saved = Summary::of(PAGE);

	assert_eq!(saved.sketches, 1);
	assert_eq!(saved.strokes, 43);
	assert!(saved.words > 60, "{saved}");

	// The editor reads it, and writes the same page back.
	let notebook = Notebook::open();
	let json = now(notebook.note.export(Format::Json)).expect("a live editor");

	assert_eq!(Summary::of(&json), saved);
	assert_eq!(notebook.summary.get(), saved);
	notebook.note.dispose();
}

#[test]
fn the_screen_names_everything_a_person_can_press() {
	let notebook = Notebook::open();
	let tester = mount(notebook);

	for text in ["Sketchnote", "Add a sketch", "Undo", "Redo"] {
		tester
			.find(&Finder::text(text))
			.unwrap_or_else(|_| panic!("the screen shows {text}"));
	}

	for (name, _) in NIBS {
		tester
			.find(&Finder::text(name))
			.unwrap_or_else(|_| panic!("the screen offers the {name} nib"));
	}

	for colour in COLOURS {
		tester
			.find(&Finder::label(format!("{} pen", colour.name)))
			.unwrap_or_else(|_| panic!("the screen offers the {} pen", colour.name));
	}

	tester
		.find(&Finder::label("Notebook page"))
		.expect("the page is named");
	tester
		.find(&Finder::text(Summary::of(PAGE).to_string()))
		.expect("the status line counts the page");
	notebook.note.dispose();
}

#[test]
fn the_buttons_change_the_document_through_the_handle() {
	let notebook = Notebook::open();
	let mut tester = mount(notebook);
	let sketches = || {
		let json = now(notebook.note.export(Format::Json)).expect("a live editor");
		Summary::of(&json).sketches
	};

	tester
		.tap(&Finder::text("Add a sketch"))
		.expect("press the button");
	assert_eq!(sketches(), 2);

	tester.tap(&Finder::text("Undo")).expect("press undo");
	assert_eq!(sketches(), 1);

	tester.tap(&Finder::text("Redo")).expect("press redo");
	assert_eq!(sketches(), 2);

	// Counting again brings the status line up to date.
	now(notebook.recount());
	assert_eq!(notebook.summary.get().sketches, 2);

	tester
		.tap(&Finder::label("Rose pen"))
		.expect("pick a colour");
	tester.tap(&Finder::text("Marker")).expect("pick a nib");
	assert_eq!(notebook.note.pen(), sketchnote_example::pen(1, 1));
	notebook.note.dispose();
}
