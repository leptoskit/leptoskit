//! Incoming URLs and system Back drive the typed router.

use file_routes_example::routes::Navigation;
use file_routes_example::routes::Note;
use file_routes_example::routes::Route;
use leptoskit_router::ActivationRef;
use leptoskit_router::OpenMode;
use leptoskit_router::SystemBack;
use leptoskit_router::apply_activations;
use leptoskit_router::clear_system_back_handler;
use leptoskit_router::dispatch_system_back;

#[test]
fn a_custom_scheme_opens_a_typed_route() {
	let navigation = Navigation::new(Route::Today);
	navigation
		.open_url("field://notes/3?highlight=1")
		.expect("the deep link matches");
	assert_eq!(
		navigation.current(),
		Route::Note(Note {
			note: 3,
			highlight: Some(1),
		})
	);
}

#[test]
fn cold_start_replace_leaves_nowhere_to_go_back() {
	let navigation = Navigation::new(Route::Today);
	navigation
		.open_url_with("field:///settings", OpenMode::Replace)
		.expect("settings opens");
	assert_eq!(navigation.current(), Route::Settings);
	assert!(!navigation.can_go_back());
}

#[test]
fn activations_batch_into_navigation() {
	let navigation = Navigation::new(Route::Today);
	let results = apply_activations(
		&navigation,
		[ActivationRef::Url("field://you")],
		OpenMode::Navigate,
	);
	assert_eq!(results, vec![Ok(())]);
	assert_eq!(navigation.current(), Route::You);
}

#[test]
fn system_back_pops_then_propagates() {
	clear_system_back_handler();
	let navigation = Navigation::new(Route::Today);
	navigation.bind_system_back();
	navigation.navigate(Route::Notes);
	assert_eq!(dispatch_system_back(), SystemBack::Consumed);
	assert_eq!(navigation.current(), Route::Today);
	assert_eq!(dispatch_system_back(), SystemBack::Propagate);
	clear_system_back_handler();
}
