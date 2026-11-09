//! The join screen accepts a local dev address and refuses anything else.

use std::sync::Arc;
use std::sync::Mutex;

use dev_client::home::Session;
use dev_client::home::home;
use leptoskit::prelude::RwSignal;
use leptoskit_testing::Finder;
use leptoskit_testing::Queries;
use leptoskit_testing::WidgetTester;

fn open(
	initial: &str,
	connected: &Arc<Mutex<Vec<String>>>,
	scanned: Option<String>,
) -> WidgetTester {
	let address = RwSignal::new(initial.to_owned());
	let status = RwSignal::new(String::new());
	let connected = Arc::clone(connected);
	WidgetTester::new(
		move || {
			let connected = Arc::clone(&connected);
			let scanned = scanned.clone();
			home(Session {
				platform: "android",
				address,
				status,
				connect: Arc::new(move |url| connected.lock().unwrap().push(url)),
				scan: Arc::new(move || scanned.clone()),
			})
		},
		390.0,
		844.0,
	)
	.unwrap()
}

#[test]
fn connecting_an_empty_field_explains_what_to_enter() {
	let connected = Arc::new(Mutex::new(Vec::new()));
	let mut tester = open("", &connected, None);
	tester.tap(&Finder::text("Connect")).unwrap();
	tester
		.get_by_text("Enter the address from leptoskit dev")
		.unwrap();
	assert!(connected.lock().unwrap().is_empty());
}

#[test]
fn connecting_a_local_address_waits_for_the_guest() {
	let connected = Arc::new(Mutex::new(Vec::new()));
	let mut tester = open("http://192.168.1.20:43123/_leptoskit/dev", &connected, None);
	tester.tap(&Finder::text("Connect")).unwrap();
	tester.get_by_text("Waiting for the app…").unwrap();
	assert_eq!(
		connected.lock().unwrap().as_slice(),
		["http://192.168.1.20:43123/_leptoskit/dev/android/guest"]
	);
}

#[test]
fn a_public_address_is_refused() {
	let connected = Arc::new(Mutex::new(Vec::new()));
	let mut tester = open("http://8.8.8.8:43123/_leptoskit/dev", &connected, None);
	tester.tap(&Finder::text("Connect")).unwrap();
	tester
		.get_by_text("The address must be on this network")
		.unwrap();
	assert!(connected.lock().unwrap().is_empty());
}

#[test]
fn scanning_a_code_connects_with_its_address() {
	let connected = Arc::new(Mutex::new(Vec::new()));
	let mut tester = open(
		"",
		&connected,
		Some("http://10.0.2.2:9/_leptoskit/dev".into()),
	);
	tester.tap(&Finder::text("Scan")).unwrap();
	assert_eq!(
		connected.lock().unwrap().as_slice(),
		["http://10.0.2.2:9/_leptoskit/dev/android/guest"]
	);
}

#[test]
fn scanning_without_a_reader_asks_for_the_address() {
	let connected = Arc::new(Mutex::new(Vec::new()));
	let mut tester = open("", &connected, None);
	tester.tap(&Finder::text("Scan")).unwrap();
	tester
		.get_by_text("Paste the address from the terminal.")
		.unwrap();
}
