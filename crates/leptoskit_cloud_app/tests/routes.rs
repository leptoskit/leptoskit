//! The console's routes: the generated module is current, every address
//! parses to the screen it names, and no organisation can take a path the
//! console uses.

use std::path::Path;

use leptoskit_cloud::access::RESERVED_ORGANIZATION_SLUGS;
use leptoskit_cloud_app::routes::Device;
use leptoskit_cloud_app::routes::Org;
use leptoskit_cloud_app::routes::Route;
use leptoskit_cloud_app::routes::SignIn;
use leptoskit_cloud_app::routes::SignInEmail;
use leptoskit_router::Routable as _;
use leptoskit_router_codegen::Status;

#[test]
fn the_route_module_is_current() {
	let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/routes");
	let generated = leptoskit_router_codegen::generate(&dir).expect("the routes generate");
	let output = leptoskit_router_codegen::output_path(&dir);

	assert_eq!(
		leptoskit_router_codegen::status(&output, &generated).expect("the module reads"),
		Status::Current,
		"src/routes.rs is out of date: run `lk plugin:router routes` in crates/leptoskit_cloud_app"
	);
}

#[test]
fn every_page_has_its_address() {
	let cases = [
		("/", Route::Home),
		(
			"/sign-in",
			Route::SignIn(SignIn {
				next: None,
				via: None,
				auth: None,
				auth_error: None,
				auth_factors: None,
			}),
		),
		(
			"/sign-in?next=%2Fdevice%3Fuser_code%3DWDJB-MJHT",
			Route::SignIn(SignIn {
				next: Some("/device?user_code=WDJB-MJHT".to_owned()),
				via: None,
				auth: None,
				auth_error: None,
				auth_factors: None,
			}),
		),
		(
			"/sign-in/email?to=ana%40example.com",
			Route::SignInEmail(SignInEmail {
				to: Some("ana@example.com".to_owned()),
				next: None,
			}),
		),
		("/welcome", Route::Welcome),
		(
			"/device?user_code=WDJB-MJHT",
			Route::Device(Device {
				user_code: Some("WDJB-MJHT".to_owned()),
			}),
		),
		("/device", Route::Device(Device { user_code: None })),
		("/organizations", Route::Organizations),
		("/account", Route::Account),
		(
			"/acme",
			Route::Org(Org {
				org: "acme".to_owned(),
			}),
		),
	];

	for (url, route) in cases {
		assert_eq!(Route::parse(url).expect(url), route, "{url}");
		let href = route.href().to_string();
		assert_eq!(Route::parse(&href).expect(&href), route, "{href}");
	}

	assert!(matches!(
		Route::parse("/acme/notes/production/42"),
		Ok(Route::NotFound(_))
	));
}

#[test]
fn no_organisation_can_take_a_path_the_console_uses() {
	let words: Vec<String> = leptoskit_cloud_app::routes::axum_route_list()
		.into_iter()
		.filter_map(|path| {
			let first = path.trim_start_matches('/').split('/').next()?.to_owned();
			(!first.is_empty() && !first.starts_with(['{', ':'])).then_some(first)
		})
		.collect();

	// The list is read from the routes, so check it found the pages.
	for page in ["sign-in", "welcome", "device", "organizations", "account"] {
		assert!(words.iter().any(|word| word == page), "{page} in {words:?}");
	}

	for word in words {
		assert!(
			RESERVED_ORGANIZATION_SLUGS.contains(&word.as_str()),
			"`/{word}` is a console page, so `{word}` must be a reserved organisation slug"
		);
	}
}
