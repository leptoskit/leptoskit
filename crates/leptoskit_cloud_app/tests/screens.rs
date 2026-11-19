//! Every screen's main states, drawn headless at a phone's width and a
//! desktop's from the console's real state and routes, with no server: the
//! query cache is filled before the console is made, as the server's
//! rendered page fills it in the browser.
//!
//! Each state's tree (with its layout) and what a screen reader hears are
//! snapshots, and so are its colours in both appearances, with every piece
//! of text checked for contrast against what it sits on.

use std::fmt::Write as _;

use insta::assert_snapshot;
use leptoskit::auth::IdentityInfo;
use leptoskit::auth::PasskeyInfo;
use leptoskit::auth::ProviderInfo;
use leptoskit::auth::PublicSession;
use leptoskit::auth::WalletSettings;
use leptoskit::idp::DeviceRequest;
use leptoskit::leptos::prelude::Owner;
use leptoskit::leptos::prelude::untrack;
use leptoskit::native::Color;
use leptoskit::native::NativeKind;
use leptoskit::prelude::*;
use leptoskit::query::QueryClient;
use leptoskit::rpc::client::Args;
use leptoskit::rpc::client::query_scope;
use leptoskit_cloud::account::OrganizationSummary;
use leptoskit_cloud::account::Organizations;
use leptoskit_cloud::account::WhoAmI;
use leptoskit_cloud::account::Whoami;
use leptoskit_cloud::projects::ProjectSummary;
use leptoskit_cloud::projects::Projects;
use leptoskit_cloud_app::components::CommandBlock;
use leptoskit_cloud_app::components::Connection;
use leptoskit_cloud_app::components::CostMeter;
use leptoskit_cloud_app::components::OfflineChip;
use leptoskit_cloud_app::components::QrCode;
use leptoskit_cloud_app::components::ShippingChip;
use leptoskit_cloud_app::components::Turned;
use leptoskit_cloud_app::console::Console;
use leptoskit_cloud_app::console::Services;
use leptoskit_cloud_app::data::InFlight;
use leptoskit_cloud_app::data::ShippingRelease;
use leptoskit_cloud_app::data::Spend;
use leptoskit_cloud_app::routes::Device;
use leptoskit_cloud_app::routes::Navigation;
use leptoskit_cloud_app::routes::Org;
use leptoskit_cloud_app::routes::Route;
use leptoskit_cloud_app::routes::SignIn;
use leptoskit_cloud_app::routes::SignInEmail;
use leptoskit_cloud_app::screens::device::DeviceSheet;
use leptoskit_cloud_app::style;
use leptoskit_cloud_app::style::Appearance;
use leptoskit_router::Routable as _;
use leptoskit_testing::Finder;
use leptoskit_testing::Queries as _;
use leptoskit_testing::WidgetTester;
use serde_json::json;

/// A phone.
const PHONE: f32 = 400.0;
/// A desktop.
const DESKTOP: f32 = 1280.0;
/// Tall enough that nothing the snapshots read is scrolled away.
const HEIGHT: f32 = 1600.0;

/// An executor that drops what it's given: the console's fetches never
/// run, so every screen draws from the cache the test filled.
struct Still;

impl any_spawner::CustomExecutor for Still {
	fn spawn(&self, _task: any_spawner::PinnedFuture<()>) {}

	fn spawn_local(&self, _task: any_spawner::PinnedLocalFuture<()>) {}

	fn poll_local(&self) {}
}

fn still() {
	let _ = any_spawner::Executor::init_local_custom_executor(Still);
}

fn session() -> PublicSession {
	serde_json::from_value(json!({
		"id": "00000000-0000-4000-8000-000000000001",
		"account_id": "00000000-0000-4000-8000-000000000002",
		"aal": "aal1",
		"authenticated_at": "2026-11-18T09:00:00Z",
		"mfa_at": null,
		"last_seen_at": "2026-11-18T09:00:00Z",
		"idle_expires_at": "2026-12-18T09:00:00Z",
		"expires_at": "2027-02-18T09:00:00Z",
		"device": "Firefox on macOS",
		"revoked_at": null,
		"created_at": "2026-11-18T09:00:00Z",
	}))
	.expect("a session")
}

fn organization(slug: &str, name: &str, personal: bool, role: &str) -> OrganizationSummary {
	serde_json::from_value(json!({
		"id": "00000000-0000-4000-8000-0000000000aa",
		"slug": slug,
		"name": name,
		"plan": "hobby",
		"personal": personal,
		"role": role,
	}))
	.expect("an organisation")
}

fn me(organizations: &[OrganizationSummary]) -> WhoAmI {
	serde_json::from_value(json!({
		"account": "00000000-0000-4000-8000-000000000002",
		"email": "ana@example.com",
		"via": "web",
		"operator": false,
		"organizations": organizations,
		"token": null,
	}))
	.expect("a person")
}

fn project(slug: &str, name: &str) -> ProjectSummary {
	serde_json::from_value(json!({
		"id": "00000000-0000-4000-8000-0000000000bb",
		"slug": slug,
		"name": name,
		"region": "eu-central",
		"created_at": "2026-11-02T10:00:00Z",
	}))
	.expect("a project")
}

fn providers() -> Vec<ProviderInfo> {
	vec![
		ProviderInfo {
			id: "github".to_owned(),
			name: "GitHub".to_owned(),
		},
		ProviderInfo {
			id: "google".to_owned(),
			name: "Google".to_owned(),
		},
	]
}

/// What the cache holds when a screen draws.
#[derive(Clone, Default)]
struct World {
	signed_in: bool,
	providers: Vec<ProviderInfo>,
	me: Option<WhoAmI>,
	projects: Option<(String, Vec<ProjectSummary>)>,
	organizations: Option<Vec<OrganizationSummary>>,
	device: Option<DeviceRequest>,
	identities: Option<Vec<IdentityInfo>>,
	passkeys: Option<Vec<PasskeyInfo>>,
	devices: Option<Vec<leptoskit::auth::Device>>,
}

impl World {
	fn signed_out() -> Self {
		Self {
			providers: providers(),
			..Self::default()
		}
	}

	fn ana(projects: Vec<ProjectSummary>) -> Self {
		Self {
			signed_in: true,
			providers: providers(),
			me: Some(me(&[organization("ana", "ana", true, "owner")])),
			projects: Some(("ana".to_owned(), projects)),
			..Self::default()
		}
	}

	fn fill(&self, client: &QueryClient) {
		use leptoskit::auth::client;

		client.set_query_data(&client::session(), (), self.signed_in.then(session));
		client.set_query_data(&client::providers(), (), self.providers.clone());
		client.set_query_data(
			&client::wallet_settings(),
			(),
			WalletSettings {
				enabled: false,
				sign_in: false,
				chain_id: "mainnet".to_owned(),
			},
		);

		if let Some(me) = &self.me {
			client.set_query_data(&query_scope::<Whoami>(), Args::new(Whoami {}), me.clone());
		}

		if let Some((organization, projects)) = &self.projects {
			client.set_query_data(
				&query_scope::<Projects>(),
				Args::new(Projects {
					organization: organization.clone(),
				}),
				projects.clone(),
			);
		}

		if let Some(organizations) = &self.organizations {
			client.set_query_data(
				&query_scope::<Organizations>(),
				Args::new(Organizations {}),
				organizations.clone(),
			);
		}

		if let Some(request) = &self.device {
			client.set_query_data(
				&leptoskit::idp::client::device_request(),
				request.user_code.clone(),
				request.clone(),
			);
		}

		if let Some(identities) = &self.identities {
			client.set_query_data(&client::identities(), (), identities.clone());
		}

		if let Some(passkeys) = &self.passkeys {
			client.set_query_data(&client::passkeys(), (), passkeys.clone());
		}

		if let Some(devices) = &self.devices {
			client.set_query_data(&client::devices(), (), devices.clone());
		}
	}
}

/// A screen, drawn.
struct Screen {
	tester: WidgetTester,
	console: Console,
	_owner: Owner,
}

impl Screen {
	fn open(route: impl Into<Route>, world: &World, width: f32) -> Self {
		still();
		style::install(Appearance::Light);
		let owner = Owner::new();
		let route = route.into();
		let (navigation, console) = owner.with(|| {
			let client = QueryClient::new();
			client.provide();
			world.fill(&client);
			let navigation = Navigation::new(route);
			let console = Console::new(
				&navigation,
				Services::default(),
				"cloud.example.test".to_owned(),
			)
			.provide();
			(navigation, console)
		});
		let drawing = owner.clone();
		let tester = WidgetTester::new(
			// The tester draws again on `pump`, rather than when a signal
			// changes, so drawing tracks nothing.
			move || drawing.with(|| untrack(|| navigation.render())),
			width,
			HEIGHT,
		)
		.expect("the screen draws");

		Self {
			tester,
			console,
			_owner: owner,
		}
	}

	/// Draws with `view` in place of a route's page, for states the cache
	/// can't put a page in, such as a decision already made.
	fn view(
		world: &World,
		width: f32,
		view: impl Fn(&Console, &Navigation) -> Element + 'static,
	) -> Self {
		still();
		style::install(Appearance::Light);
		let owner = Owner::new();
		let (navigation, console) = owner.with(|| {
			let client = QueryClient::new();
			client.provide();
			world.fill(&client);
			let navigation = Navigation::new(Route::Home);
			let console = Console::new(
				&navigation,
				Services::default(),
				"cloud.example.test".to_owned(),
			)
			.provide();
			(navigation, console)
		});
		let drawing = owner.clone();
		let tester = WidgetTester::new(
			move || drawing.with(|| untrack(|| view(&console, &navigation))),
			width,
			HEIGHT,
		)
		.expect("the view draws");

		Self {
			tester,
			console,
			_owner: owner,
		}
	}

	fn tree(&self) -> String {
		self.tester.tree()
	}

	fn semantics(&self) -> String {
		self.tester.semantics()
	}

	/// Every visible piece of text with its colour, what it sits on, and the
	/// contrast between them, in `appearance`.
	fn palette(&mut self, appearance: Appearance) -> String {
		style::install(appearance);
		self.tester.pump();
		let surface = self.tester.surface();
		let paper = Color::hex(style::TOKENS[0].value(appearance)).expect("paper is a colour");
		let mut output = String::new();
		let mut pending = surface
			.root()
			.map(|id| (id, paper))
			.into_iter()
			.collect::<Vec<_>>();

		while let Some((id, behind)) = pending.pop() {
			let props = surface.props(id);

			if props.hidden == Some(true) {
				continue;
			}

			let behind = props
				.background
				.filter(|color| color.0 & 0xff == 0xff)
				.unwrap_or(behind);

			if surface.kind(id) == NativeKind::Text
				&& let (Some(text), Some(color)) = (props.text.as_deref(), props.color)
				&& !text.trim().is_empty()
			{
				let ratio = contrast(color, behind);
				let _ = writeln!(
					output,
					"{:<44} {} on {} {ratio:>5.2}",
					truncate(text),
					hex(color),
					hex(behind)
				);
			}

			pending.extend(
				surface
					.children(id)
					.iter()
					.rev()
					.map(|&child| (child, behind)),
			);
		}

		style::install(Appearance::Light);
		output
	}
}

fn truncate(text: &str) -> String {
	let text: String = text.chars().take(40).collect();
	format!("{text:?}")
}

fn hex(color: Color) -> String {
	format!("#{:06x}", color.0 >> 8)
}

fn luminance(color: Color) -> f64 {
	let channel = |shift: u32| {
		let value = f64::from((color.0 >> shift) & 0xff) / 255.0;
		if value <= 0.039_28 {
			value / 12.92
		} else {
			((value + 0.055) / 1.055).powf(2.4)
		}
	};
	0.2126 * channel(24) + 0.7152 * channel(16) + 0.0722 * channel(8)
}

fn contrast(text: Color, behind: Color) -> f64 {
	let (a, b) = (luminance(text), luminance(behind));
	let (light, dark) = if a > b { (a, b) } else { (b, a) };
	(light + 0.05) / (dark + 0.05)
}

/// Checks every line of `palette` reaches 4.5:1, WCAG 2.2's minimum for
/// text, and returns it for its snapshot.
fn readable(palette: String) -> String {
	for line in palette.lines() {
		let ratio: f64 = line
			.rsplit(' ')
			.next()
			.and_then(|ratio| ratio.trim().parse().ok())
			.expect("each line ends in its contrast");
		assert!(ratio >= 4.5, "too little contrast: {line}");
	}

	palette
}

/// A screen's snapshots: its tree at both widths, what a screen reader hears,
/// and its colours in both appearances.
fn snapshot_screen(name: &str, open: impl Fn(f32) -> Screen) {
	let mut desktop = open(DESKTOP);
	let phone = open(PHONE);
	assert_snapshot!(format!("{name}_desktop"), desktop.tree());
	assert_snapshot!(format!("{name}_phone"), phone.tree());
	assert_snapshot!(format!("{name}_semantics"), desktop.semantics());
	assert_snapshot!(
		format!("{name}_light"),
		readable(desktop.palette(Appearance::Light))
	);
	assert_snapshot!(
		format!("{name}_dark"),
		readable(desktop.palette(Appearance::Dark))
	);
}

fn sign_in(next: Option<&str>) -> SignIn {
	SignIn {
		next: next.map(str::to_owned),
		via: None,
		auth: None,
		auth_error: None,
		auth_factors: None,
	}
}

fn device_request(code: &str) -> DeviceRequest {
	serde_json::from_value(json!({
		"app": { "client_id": "cli", "name": "LeptosKit CLI" },
		"scopes": [
			{ "name": "openid", "description": "Know who you are." },
			{ "name": "email", "description": "See your email address." },
			{ "name": "cloud", "description": "See your organisations and projects, ship releases and change settings." },
		],
		"user_code": code,
	}))
	.expect("a device request")
}

#[test]
fn sign_in_offers_github_first_then_an_emailed_code_then_google() {
	snapshot_screen("sign_in", |width| {
		Screen::open(sign_in(None), &World::signed_out(), width)
	});

	let screen = Screen::open(sign_in(None), &World::signed_out(), DESKTOP);
	let semantics = screen.semantics();
	let github = semantics.find("Continue with GitHub").expect("GitHub");
	let email = semantics.find("Email me a code").expect("email");
	let google = semantics.find("Google").expect("Google");
	assert!(github < email && email < google);
	// Passkeys need the browser, and wallets need one the browser offers.
	assert!(!semantics.contains("Passkey"));
	assert!(!semantics.contains("Solana wallet"));
}

#[test]
fn sign_in_shows_only_the_providers_the_server_set_up() {
	let world = World {
		providers: Vec::new(),
		..World::signed_out()
	};
	let screen = Screen::open(sign_in(None), &world, DESKTOP);
	let semantics = screen.semantics();
	assert!(!semantics.contains("Continue with GitHub"));
	assert!(!semantics.contains("Google"));
	assert!(semantics.contains("Email me a code"));
}

#[test]
fn sign_in_from_a_terminal_says_the_terminal_is_waiting_and_names_a_failed_provider() {
	let route = SignIn {
		next: Some("/device?user_code=WDJB-MJHT".to_owned()),
		via: Some("github".to_owned()),
		auth: Some("error".to_owned()),
		auth_error: Some("cancelled".to_owned()),
		auth_factors: None,
	};
	let screen = Screen::open(route, &World::signed_out(), PHONE);
	assert_snapshot!("sign_in_from_terminal_semantics", screen.semantics());
	screen
		.tester
		.get_by_text("Your terminal is waiting. Sign in, then approve it.")
		.expect("the note");
	screen
		.tester
		.get_by_text(
			"✕ GitHub didn't sign you in: the request was cancelled. Try again, or use another \
			 way.",
		)
		.expect("the problem");
}

#[test]
fn an_address_is_checked_before_a_code_is_sent() {
	let mut screen = Screen::open(sign_in(None), &World::signed_out(), DESKTOP);
	screen
		.tester
		.enter_text(&Finder::label("Email"), "ana")
		.expect("typing");
	screen
		.tester
		.tap(&Finder::label("Email me a code"))
		.expect("pressing");
	screen
		.tester
		.get_by_text("✕ That doesn't look like an email address.")
		.expect("the problem");
	assert!(!screen.console.sign_in.send.is_pending());
}

#[test]
fn the_code_sheet_says_where_the_code_went() {
	snapshot_screen("sign_in_email", |width| {
		Screen::open(
			SignInEmail {
				to: Some("ana@example.com".to_owned()),
				next: None,
			},
			&World::signed_out(),
			width,
		)
	});
}

#[test]
fn welcome_greets_a_new_account_and_asks_one_question() {
	snapshot_screen("welcome", |width| {
		Screen::open(Route::Welcome, &World::ana(Vec::new()), width)
	});
}

#[test]
fn device_asks_whether_the_terminal_shows_the_code() {
	let world = World {
		device: Some(device_request("WDJB-MJHT")),
		..World::ana(Vec::new())
	};
	snapshot_screen("device", |width| {
		Screen::open(
			Device {
				user_code: Some("wdjb-mjht".to_owned()),
			},
			&world,
			width,
		)
	});
	let screen = Screen::open(
		Device {
			user_code: Some("WDJBMJHT".to_owned()),
		},
		&world,
		DESKTOP,
	);
	let code = screen
		.tester
		.find(&Finder::test_id("device-code"))
		.expect("the code");
	assert_eq!(
		code.props.accessibility_label.as_deref(),
		Some("W D J B, M J H T")
	);
	screen
		.tester
		.get_by_text("Your account is ready.")
		.expect("a new account is told so");
}

#[test]
fn device_without_a_code_asks_for_one() {
	let screen = Screen::open(Device { user_code: None }, &World::ana(Vec::new()), PHONE);
	assert_snapshot!("device_enter_semantics", screen.semantics());
}

#[test]
fn device_after_a_decision() {
	let world = World::ana(vec![project("notes", "notes")]);
	let approved = Screen::view(&world, PHONE, |console, navigation| {
		leptoskit_cloud_app::screens::device::view(
			console,
			navigation,
			&DeviceSheet::Approved {
				organization: Some("ana".to_owned()),
				has_projects: true,
			},
		)
	});
	assert_snapshot!("device_approved_semantics", approved.semantics());
	let denied = Screen::view(&world, PHONE, |console, navigation| {
		leptoskit_cloud_app::screens::device::view(console, navigation, &DeviceSheet::Denied)
	});
	assert_snapshot!("device_denied_semantics", denied.semantics());
	let expired = Screen::view(&world, PHONE, |console, navigation| {
		leptoskit_cloud_app::screens::device::view(
			console,
			navigation,
			&DeviceSheet::Expired {
				code: "WDJB-MJHT".to_owned(),
			},
		)
	});
	assert_snapshot!("device_expired_semantics", expired.semantics());
}

#[test]
fn projects_are_cards_with_workings() {
	let world = World::ana(vec![project("notes", "notes"), project("blog", "Blog")]);
	snapshot_screen("projects", |width| {
		Screen::open(
			Org {
				org: "ana".to_owned(),
			},
			&world,
			width,
		)
	});

	let mut screen = Screen::open(
		Org {
			org: "ana".to_owned(),
		},
		&world,
		DESKTOP,
	);
	assert!(
		screen
			.tester
			.find(&Finder::label(
				"Show workings for notes: projects::projects"
			))
			.is_ok(),
		"each card has its own mint tab"
	);
	screen
		.tester
		.tap(&Finder::label("Show workings for Blog: projects::projects"))
		.expect("the mint tab turns its card");
	screen.tester.pump();
	screen
		.tester
		.get_by_text("Workings: Blog")
		.expect("the card shows its back");
	screen
		.tester
		.get_by_text("Workings: notes")
		.expect_err("the other card stays on its front");
	screen
		.tester
		.tap(&Finder::label("Turn back: Blog"))
		.expect("the card turns back");
	screen.tester.pump();
	screen
		.tester
		.get_by_text("Workings: Blog")
		.expect_err("the card is on its front again");
}

#[test]
fn an_organisation_with_no_projects_offers_three_ways_in() {
	snapshot_screen("projects_empty", |width| {
		Screen::open(
			Org {
				org: "ana".to_owned(),
			},
			&World::ana(Vec::new()),
			width,
		)
	});
}

#[test]
fn an_organisation_you_cannot_see_is_not_there() {
	let screen = Screen::open(
		Org {
			org: "acme".to_owned(),
		},
		&World::ana(Vec::new()),
		DESKTOP,
	);
	screen
		.tester
		.get_by_text("There's no organisation at acme, or you can't see it.")
		.expect("not found");
}

#[test]
fn organizations_lists_each_with_your_role_and_its_plan() {
	let organizations = vec![
		organization("ana", "ana", true, "owner"),
		organization("acme", "Acme", false, "developer"),
	];
	let world = World {
		me: Some(me(&organizations)),
		organizations: Some(organizations),
		..World::ana(Vec::new())
	};
	snapshot_screen("organizations", |width| {
		Screen::open(Route::Organizations, &world, width)
	});
}

#[test]
fn the_organisation_switch_shows_only_with_more_than_one() {
	let one = Screen::open(
		Org {
			org: "ana".to_owned(),
		},
		&World::ana(Vec::new()),
		DESKTOP,
	);
	assert!(!one.semantics().contains("Switch organisation"));

	let world = World {
		me: Some(me(&[
			organization("ana", "ana", true, "owner"),
			organization("acme", "Acme", false, "developer"),
		])),
		..World::ana(Vec::new())
	};
	let two = Screen::open(
		Org {
			org: "ana".to_owned(),
		},
		&world,
		DESKTOP,
	);
	assert!(two.semantics().contains("Switch organisation"));
}

#[test]
fn a_new_organisations_slug_is_checked_as_it_is_typed() {
	let world = World {
		organizations: Some(vec![organization("ana", "ana", true, "owner")]),
		..World::ana(Vec::new())
	};
	let mut screen = Screen::open(Route::Organizations, &world, DESKTOP);
	screen
		.tester
		.tap(&Finder::label("Create an organisation"))
		.expect("opening the form");
	screen
		.tester
		.enter_text(&Finder::label("Name"), "Acme Labs")
		.expect("naming it");
	screen
		.tester
		.get_by_text("Its projects will be at /acme-labs.")
		.expect("the slug follows the name");
	screen
		.tester
		.enter_text(
			&Finder::label("Slug: the address of its projects"),
			"device",
		)
		.expect("typing a slug");
	screen
		.tester
		.get_by_text("✕ “device” is one of the console's own addresses; choose another.")
		.expect("the problem");
}

#[test]
fn account_lists_sign_in_methods_devices_and_appearance() {
	let current: leptoskit::auth::Device = serde_json::from_value(json!({
		"id": "00000000-0000-4000-8000-000000000001",
		"account_id": "00000000-0000-4000-8000-000000000002",
		"aal": "aal1",
		"authenticated_at": "2026-11-18T09:00:00Z",
		"mfa_at": null,
		"last_seen_at": "2026-11-18T09:00:00Z",
		"idle_expires_at": "2026-12-18T09:00:00Z",
		"expires_at": "2027-02-18T09:00:00Z",
		"device": "Firefox on macOS",
		"revoked_at": null,
		"created_at": "2026-11-18T09:00:00Z",
		"current": true,
	}))
	.expect("a device");
	let world = World {
		identities: Some(Vec::new()),
		passkeys: Some(Vec::new()),
		devices: Some(vec![current]),
		..World::ana(Vec::new())
	};
	snapshot_screen("account", |width| {
		Screen::open(Route::Account, &world, width)
	});
}

#[test]
fn signed_out_console_pages_offer_to_sign_in_and_come_back() {
	let screen = Screen::open(Route::Account, &World::signed_out(), DESKTOP);
	assert_snapshot!("signed_out_semantics", screen.semantics());
}

#[test]
fn an_unknown_address_is_not_found() {
	let screen = Screen::open(
		Route::parse("/ana/notes/production/42").expect("any address parses"),
		&World::ana(Vec::new()),
		DESKTOP,
	);
	assert_snapshot!("not_found_semantics", screen.semantics());
}

#[test]
fn chips_say_only_what_the_console_knows() {
	let chips = Screen::view(&World::ana(Vec::new()), DESKTOP, |_, _| {
		View()
			.children(
				ShippingChip {
					in_flight: InFlight::NotAvailable,
				}
				.build(),
			)
			.children(
				ShippingChip {
					in_flight: InFlight::Releases(vec![ShippingRelease {
						project: "notes".to_owned(),
						number: 42,
						stage: "Building 1:12".to_owned(),
						href: "/ana/notes/production/42".to_owned(),
					}]),
				}
				.build(),
			)
			.child(
				CostMeter {
					plan: "hobby".to_owned(),
					spend: Spend::NotAvailable,
					href: None,
				}
				.build(),
			)
			.child(
				CostMeter {
					plan: "pro".to_owned(),
					spend: Spend::Known {
						spent_cents: 412,
						cap_cents: 2000,
						forecast_cents: 2400,
					},
					href: Some("/acme/settings/billing".to_owned()),
				}
				.build(),
			)
			.children(
				OfflineChip {
					connection: Connection::Reconnecting {
						next_in: std::time::Duration::from_secs(8),
					},
				}
				.build(),
			)
			.children(
				OfflineChip {
					connection: Connection::Fine,
				}
				.build(),
			)
	});
	assert_snapshot!("chips_semantics", chips.semantics());
}

#[test]
fn copying_a_command_says_so_and_is_announced() {
	let mut screen = Screen::view(&World::ana(Vec::new()), DESKTOP, |console, _| {
		CommandBlock {
			commands: vec!["lk cloud login".to_owned(), "lk cloud launch".to_owned()],
		}
		.build(console.copied)
	});
	screen
		.tester
		.tap(&Finder::label("Copy lk cloud login, lk cloud launch"))
		.expect("copying");
	screen
		.tester
		.get_by_text("Copied")
		.expect("the button says so");
	assert!(screen.semantics().contains("Copied to the clipboard."));
}

#[test]
fn a_qr_code_is_drawn_from_its_link_and_shows_the_link() {
	let screen = Screen::view(&World::ana(Vec::new()), PHONE, |_, _| {
		QrCode {
			url: "https://cloud.example.test/device?user_code=WDJB-MJHT".to_owned(),
			label: "QR code to approve on your phone".to_owned(),
		}
		.build()
	});
	screen
		.tester
		.find(&Finder::label("QR code to approve on your phone"))
		.expect("the code");
	screen
		.tester
		.get_by_text("https://cloud.example.test/device?user_code=WDJB-MJHT")
		.expect("the link");
}

#[test]
fn w_turns_the_focused_card_shift_w_turns_them_all_and_escape_turns_them_back() {
	still();
	let owner = Owner::new();
	owner.with(|| {
		let turned = Turned::new();
		assert!(!turned.turn_focused(), "nothing is focused");
		turned.focus(Some("project-notes"));
		assert!(turned.turn_focused());
		assert!(turned.is_turned("project-notes"));
		assert!(!turned.is_turned("project-blog"));
		turned.turn_all();
		assert!(turned.is_turned("project-blog"));
		assert!(turned.turn_back_all());
		assert!(!turned.is_turned("project-notes"));
		assert!(!turned.is_turned("project-blog"));
		assert!(!turned.turn_back_all(), "nothing left to turn back");
	});
}
