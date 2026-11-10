//! Every example mounts, and a few respond to a press.

use exact_examples::EXAMPLES;
use exact_examples::find;
use leptoskit::remember::ScreenState;
use leptoskit_testing::Finder;
use leptoskit_testing::WidgetTester;
use reactive_graph::owner::Owner;

fn mount(render: fn() -> leptoskit::prelude::Element) -> WidgetTester {
	let owner = Owner::new();
	let state = owner.with(ScreenState::new);
	owner.with(|| state.provide());
	WidgetTester::new(move || owner.with(|| state.build(render)), 390.0, 844.0)
		.expect("the screen mounts")
}

#[test]
fn every_example_shows_its_title() {
	for example in EXAMPLES {
		let tester = mount(example.render);
		tester
			.find(&Finder::text(example.title))
			.unwrap_or_else(|_| panic!("{} did not show its title", example.name));
	}
}

#[test]
fn caltrain_changes_the_departure_board() {
	let mut tester = mount(find("caltrain").expect("caltrain").render);
	tester
		.tap(&Finder::text("San Jose"))
		.expect("tap the station");
	tester
		.find(&Finder::text("San Jose departures"))
		.expect("the board follows the station");
}

#[test]
fn todo_counts_what_is_left() {
	let mut tester = mount(find("todo").expect("todo").render);
	tester
		.find(&Finder::text("2 left"))
		.expect("two open tasks");
	tester.tap(&Finder::text("Add")).expect("add a task");
	tester
		.find(&Finder::text("3 left"))
		.expect("the new task counts");
}

#[test]
fn shelf_opens_a_recipe() {
	let mut tester = mount(find("shelf").expect("shelf").render);
	tester
		.tap(&Finder::text("Citrus salad"))
		.expect("open the salad");
	tester
		.find(&Finder::text("Orange, fennel, olive oil"))
		.expect("the ingredients");
}

#[test]
fn carousel_windows_the_first_cards() {
	let tester = mount(find("carousel").expect("carousel").render);
	tester
		.find(&Finder::text("Card 1"))
		.expect("the first card");
	assert!(
		tester.find(&Finder::text("Card 25000")).is_err(),
		"a card off the window is not built"
	);
}

#[test]
fn screen_query_names_caltrain() {
	assert_eq!(
		exact_examples::screen_in_query("?screen=caltrain").map(|example| example.name),
		Some("caltrain")
	);
}
