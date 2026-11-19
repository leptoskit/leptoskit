//! The admin end to end: signing in through the main site, refusing people
//! who aren't admins, acting on the control service as the signed-in
//! admin, streaming progress, and what each key screen shows.

mod support;

use axum::http::StatusCode;
use leptoskit_deploy_console::control::Phase;
use leptoskit_deploy_console::control::Principal;
use leptoskit_deploy_console::control::RunResult;
use serde_json::json;
use support::*;

/// Replaces what changes between runs: operation IDs and clock times.
fn redact(world: &World, text: &str) -> String {
	let mut text = text.to_owned();
	let operations = world
		.service
		.overview(&Principal::owner())
		.unwrap()
		.operations;

	for (index, operation) in operations.iter().enumerate() {
		text = text.replace(&operation.id, &format!("<operation {index}>"));
		text = text.replace(&operation.id[..8], &format!("<op {index}>"));
	}

	// Clock times on chart axes: `14:05`.
	let chars: Vec<char> = text.chars().collect();
	let mut out = String::with_capacity(text.len());
	let mut index = 0;

	while index < chars.len() {
		let digit = |at: usize| chars.get(at).is_some_and(char::is_ascii_digit);
		let clock = digit(index)
			&& digit(index + 1)
			&& chars.get(index + 2) == Some(&':')
			&& digit(index + 3)
			&& digit(index + 4)
			&& (index == 0 || !digit(index - 1));

		if clock {
			out.push_str("hh:mm");
			index += 5;
		} else {
			out.push(chars[index]);
			index += 1;
		}
	}

	out
}

/// The `<html>` element's own attributes.
fn root(html: &str) -> &str {
	html.split_once("<head>").map_or(html, |(root, _)| root)
}

async fn page(world: &World, browser: &mut Browser, path: &str) -> (StatusCode, String) {
	let response = browser.get(&world.admin, ADMIN, path).await;
	let status = response.status();
	(status, text(response).await)
}

/// Runs a plan to completion as the worker would, with these changes.
fn planned(world: &World, changes: &serde_json::Value) -> String {
	let claimed = world
		.store
		.claim_project(Some("notes-production"))
		.unwrap()
		.unwrap();
	world
		.service
		.progress(&claimed.id, "Generated infra/notes-production/main.tf.json")
		.unwrap();
	world
		.store
		.finish(
			&claimed.id,
			&RunResult {
				succeeded: true,
				message: "Completed. The existing release engine verified its normal safety \
				          checks."
					.into(),
				details: json!({ "code": "completed", "changes": changes }),
			},
		)
		.unwrap();
	claimed.id
}

async fn act(
	world: &World,
	browser: &mut Browser,
	body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
	let response = browser
		.post_json(
			&world.admin,
			ADMIN,
			&format!("{BASE}/api/projects/notes-production/actions"),
			body,
		)
		.await;
	let status = response.status();
	(status, json_body(response).await)
}

#[tokio::test]
async fn the_sign_in_screen_sends_people_to_the_main_site() {
	let world = world().await;
	let mut browser = Browser::default();

	// Every admin page sends someone signed out to sign in.
	let response = browser
		.get(&world.admin, ADMIN, &format!("{BASE}/infra/projects"))
		.await;
	assert_eq!(location(&response), format!("{BASE}/sign-in"));

	let (status, html) = page(&world, &mut browser, &format!("{BASE}/sign-in")).await;
	assert_eq!(status, StatusCode::OK);
	assert!(html.contains("Continue with Notes"));
	insta::assert_snapshot!("sign_in", outline(&html));
}

#[tokio::test]
async fn an_admin_signs_in_through_the_main_site() {
	let world = world().await;
	let mut ada = world.ada().await;

	// The callback lands on the sign-in page, which sends an admin in.
	let response = ada
		.get(
			&world.admin,
			ADMIN,
			&format!("{BASE}/sign-in?auth=signed_in"),
		)
		.await;
	assert_eq!(location(&response), format!("{BASE}/"));
	let response = ada.get(&world.admin, ADMIN, &format!("{BASE}/")).await;
	assert_eq!(
		location(&response),
		format!("{BASE}/infra/projects/notes-production/overview")
	);

	let (status, html) = page(
		&world,
		&mut ada,
		&format!("{BASE}/infra/projects/notes-production/overview"),
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert!(html.contains("Signed in as ada@example.com"), "{html}");
	insta::assert_snapshot!(
		"overview_empty",
		redact(
			&world,
			&regions(&outline(&html), &["top-bar", "rail", "main", "tab-bar"])
		)
	);

	// Signing out ends the admin's session, not the main site's.
	let response = ada
		.post_json(&world.admin, ADMIN, "/auth/sign-out", json!({}))
		.await;
	assert!(response.status().is_success(), "{response:?}");
	let response = ada.get(&world.admin, ADMIN, &format!("{BASE}/learn")).await;
	assert_eq!(location(&response), format!("{BASE}/sign-in"));
}

#[tokio::test]
async fn someone_who_isnt_an_admin_is_refused() {
	let world = world().await;
	let mut grace = Browser::default();
	world.notes_sign_in(&mut grace, "grace@example.com").await;

	let to = world.continue_with_notes(&mut grace).await;
	assert!(to.contains("auth_error=not_allowed"), "{to}");

	let (status, html) = page(&world, &mut grace, &to).await;
	assert_eq!(status, StatusCode::FORBIDDEN);
	assert!(
		html.contains("isn&#39;t an admin here") || html.contains("isn't an admin here"),
		"{html}"
	);
	// Switch account starts sign-in again; the script asks the main site to
	// sign someone in afresh rather than reuse Grace.
	assert!(html.contains("data-testid=\"switch-account\""), "{html}");
	insta::assert_snapshot!("refusal", outline(&html));

	// The gate refused before any session was made.
	let response = grace
		.get(&world.admin, ADMIN, &format!("{BASE}/infra/projects"))
		.await;
	assert_eq!(location(&response), format!("{BASE}/sign-in"));
	let (status, _) = act(
		&world,
		&mut grace,
		json!({ "kind": "plan", "idempotency_key": "grace" }),
	)
	.await;
	assert_eq!(status, StatusCode::UNAUTHORIZED);
	assert!(
		world
			.service
			.overview(&Principal::owner())
			.unwrap()
			.operations
			.is_empty()
	);
}

#[tokio::test]
async fn actions_run_as_the_signed_in_admin_and_need_the_role() {
	let world = world().await;
	let mut ada = world.ada().await;

	let (status, body) = act(
		&world,
		&mut ada,
		json!({ "kind": "plan", "idempotency_key": "one" }),
	)
	.await;
	assert_eq!(status, StatusCode::ACCEPTED, "{body}");
	let id = body["operation"].as_str().unwrap().to_owned();
	assert_eq!(body["href"], format!("{BASE}/infra/operations/{id}"));
	let operation = world.service.operation(&Principal::owner(), &id).unwrap();
	assert_eq!(operation.actor.as_deref(), Some("ada@example.com"));
	assert_eq!(operation.phase, Phase::Queued);

	// One operation at a time, explained.
	let (status, body) = act(
		&world,
		&mut ada,
		json!({ "kind": "status", "idempotency_key": "two" }),
	)
	.await;
	assert_eq!(status, StatusCode::CONFLICT);
	assert_eq!(body["error"], "operation_in_progress");
	assert!(body["message"].as_str().unwrap().contains("one at a time"));

	// A deploy needs a version, and the version must be one the engine takes.
	let (status, body) = act(
		&world,
		&mut ada,
		json!({ "kind": "deploy", "idempotency_key": "three" }),
	)
	.await;
	assert_eq!(status, StatusCode::BAD_REQUEST);
	assert_eq!(body["error"], "invalid_action");

	// When the main site takes the role away, the next request is refused.
	world
		.admin_db
		.run_script("UPDATE admin_roles SET roles = ''")
		.await
		.unwrap();
	let (status, body) = act(
		&world,
		&mut ada,
		json!({ "kind": "backup", "idempotency_key": "four" }),
	)
	.await;
	assert_eq!(status, StatusCode::FORBIDDEN);
	assert_eq!(body["error"], "not_admin");
	let (status, html) = page(&world, &mut ada, &format!("{BASE}/infra/projects")).await;
	assert_eq!(status, StatusCode::FORBIDDEN);
	assert!(html.contains("ada@example.com"));
	assert_eq!(
		world
			.service
			.overview(&Principal::owner())
			.unwrap()
			.operations
			.len(),
		1,
		"nothing else was enqueued"
	);
}

#[tokio::test]
async fn progress_streams_to_the_operation_page() {
	let world = world().await;
	let mut ada = world.ada().await;
	let (_, body) = act(
		&world,
		&mut ada,
		json!({ "kind": "plan", "idempotency_key": "live" }),
	)
	.await;
	let id = body["operation"].as_str().unwrap().to_owned();
	let claimed = world
		.store
		.claim_project(Some("notes-production"))
		.unwrap()
		.unwrap();
	assert_eq!(claimed.id, id);

	let (status, html) = page(&world, &mut ada, &format!("{BASE}/infra/operations/{id}")).await;
	assert_eq!(status, StatusCode::OK);
	assert!(
		html.contains("data-testid=\"live\""),
		"a running operation follows along"
	);

	assert!(
		html.contains("Following live"),
		"the page says it is following: {html}"
	);

	// The page follows over the admin's own live connection, which opens
	// only for a signed-in admin.
	let response = ada
		.get(&world.admin, ADMIN, &format!("{BASE}/live/events"))
		.await;
	assert_eq!(response.status(), StatusCode::OK);
	assert_eq!(response.headers()["content-type"], "text/event-stream");
	let response = Browser::default()
		.get(&world.admin, ADMIN, &format!("{BASE}/live/events"))
		.await;
	assert!(
		response.status().is_client_error(),
		"a stranger can't open it: {}",
		response.status()
	);
}

#[tokio::test]
async fn key_screens() {
	let world = world().await;
	let mut ada = world.ada().await;

	// A plan Ada asked for, which the worker finished.
	let (_, body) = act(
		&world,
		&mut ada,
		json!({ "kind": "plan", "idempotency_key": "plan" }),
	)
	.await;
	let plan = body["operation"].as_str().unwrap().to_owned();
	assert_eq!(
		planned(
			&world,
			&json!([
				{ "address": "hcloud_server.web[1]", "actions": ["create"] },
				{ "address": "cloudflare_record.app", "actions": ["update"] },
				{ "address": "hcloud_volume.data", "actions": ["delete", "create"] },
			]),
		),
		plan
	);

	// Changes: the plan grouped by kind, the blocker, and Workings open.
	let (status, html) = page(
		&world,
		&mut ada,
		&format!("{BASE}/infra/projects/notes-production/changes"),
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert!(
		root(&html).contains("data-admin-drawer=\"plan\""),
		"Workings opens the first times"
	);
	assert!(html.contains("persistent_resource_deletion_blocked"));
	insta::assert_snapshot!(
		"changes",
		redact(&world, &regions(&outline(&html), &["main", "drawer-aside"]))
	);

	// The operation's timeline.
	let (_, html) = page(&world, &mut ada, &format!("{BASE}/infra/operations/{plan}")).await;
	insta::assert_snapshot!(
		"operation",
		redact(&world, &regions(&outline(&html), &["main", "drawer-aside"]))
	);

	// Health, with what one instance reported.
	let analytics = leptoskit::apm::Analytics::new(
		leptoskit::apm::Resource {
			service: "notes-production".into(),
			environment: "production".into(),
			release: "v41".into(),
			instance: "web-0".into(),
		},
		leptoskit::apm::Limits::default(),
		|| {
			std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.unwrap()
				.as_secs()
		},
	)
	.unwrap();

	for (operation, duration) in [("/notes/{id}", 12.0), ("/notes/{id}", 40.0), ("/", 4.0)] {
		analytics.record(
			leptoskit::apm::SeriesKey {
				source: leptoskit::apm::Source::Backend,
				signal: leptoskit::apm::Signal::Http,
				operation: operation.into(),
				outcome: leptoskit::apm::Outcome::Ok,
			},
			duration,
			None,
		);
	}

	world
		.store
		.ingest("notes-production", &analytics.snapshot())
		.unwrap();
	let (status, html) = page(
		&world,
		&mut ada,
		&format!("{BASE}/infra/projects/notes-production/health"),
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert!(html.contains("route-row"));
	insta::assert_snapshot!(
		"health",
		redact(&world, &regions(&outline(&html), &["main", "drawer-aside"]))
	);

	// Learn, with progress read from the projects.
	let (_, html) = page(&world, &mut ada, &format!("{BASE}/learn")).await;
	assert!(html.contains("Infrastructure 1 of 5"), "{html}");
	assert!(
		html.contains("data-testid=\"up-next\""),
		"the first unfinished lesson is offered next"
	);
	insta::assert_snapshot!(
		"learn",
		redact(&world, &regions(&outline(&html), &["main", "drawer-aside"]))
	);

	// With no analytics service connected, an Analytics screen teaches how
	// to connect one rather than inventing numbers.
	let (status, html) = page(&world, &mut ada, &format!("{BASE}/analytics/overview")).await;
	assert_eq!(status, StatusCode::OK);
	assert!(
		html.contains("data-testid=\"analytics-not-configured\""),
		"{html}"
	);
	insta::assert_snapshot!(
		"analytics_not_configured",
		redact(&world, &regions(&outline(&html), &["main", "drawer-aside"]))
	);
}

#[tokio::test]
async fn preferences_follow_the_person() {
	let world = world().await;
	let mut ada = world.ada().await;
	let changes = format!("{BASE}/infra/projects/notes-production/changes");

	let (_, html) = page(&world, &mut ada, &changes).await;
	assert!(
		root(&html).contains("data-admin-mode=\"workings\""),
		"Workings mode is on the first week"
	);
	assert!(html.contains("note:plan-review"));

	for change in [
		json!({ "kind": "workings_mode", "on": false }),
		json!({ "kind": "dismiss_note", "note": "plan-review" }),
		json!({ "kind": "use_note", "note": "plan-workings" }),
		json!({ "kind": "use_note", "note": "plan-workings" }),
		json!({ "kind": "use_note", "note": "plan-workings" }),
	] {
		let response = ada
			.post_json(
				&world.admin,
				ADMIN,
				&format!("{BASE}/api/preferences"),
				change,
			)
			.await;
		assert_eq!(response.status(), StatusCode::OK);
	}

	let (_, html) = page(&world, &mut ada, &changes).await;
	assert!(!root(&html).contains("data-admin-mode=\"workings\""));
	assert!(!html.contains("note:plan-review"));
	assert!(
		!root(&html).contains("data-admin-drawer=\"plan\""),
		"Workings retires after three opens"
	);

	let response = ada
		.post_json(
			&world.admin,
			ADMIN,
			&format!("{BASE}/api/preferences"),
			json!({ "kind": "dismiss_note", "note": "<script>" }),
		)
		.await;
	assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

/// Finishes the oldest queued operation as the worker would.
fn completed(world: &World) -> String {
	let claimed = world
		.store
		.claim_project(Some("notes-production"))
		.unwrap()
		.unwrap();
	world
		.store
		.finish(
			&claimed.id,
			&RunResult {
				succeeded: true,
				message: "Completed.".into(),
				details: json!({ "code": "completed" }),
			},
		)
		.unwrap();
	claimed.id
}

/// The `aria-description` of the element with `test_id`.
fn description_of(html: &str, test_id: &str) -> String {
	let start = html
		.find(&format!("data-testid=\"{test_id}\""))
		.unwrap_or_else(|| panic!("no {test_id}"));
	let tag_start = html[..start].rfind('<').unwrap();
	let tag = &html[tag_start..start + html[start..].find('>').unwrap()];
	let value = tag
		.split("aria-description=\"")
		.nth(1)
		.unwrap_or_else(|| panic!("{test_id} has no description: {tag}"));
	value[..value.find('"').unwrap()].to_owned()
}

#[tokio::test]
async fn a_simulated_admin_says_so_and_a_real_one_never_does() {
	for simulated in [true, false] {
		let world = world_with_options(None, simulated).await;
		let mut ada = world.ada().await;
		let project = format!("{BASE}/infra/projects/notes-production");
		let notice = "plans and applies run against a stand-in cloud; no provider is contacted.";

		// Every screen, signed out or in, carries the word beside the pill.
		let (_, sign_in) = page(&world, &mut Browser::default(), &format!("{BASE}/sign-in")).await;
		let (_, overview) = page(&world, &mut ada, &format!("{project}/overview")).await;
		let (_, health) = page(&world, &mut ada, &format!("{project}/health")).await;
		let (_, learn) = page(&world, &mut ada, &format!("{BASE}/learn")).await;

		for html in [&sign_in, &overview, &health, &learn] {
			assert_eq!(
				html.contains("data-testid=\"simulated\""),
				simulated,
				"simulated = {simulated}"
			);
		}

		// Overview and Changes say what that means; Health doesn't repeat it.
		assert_eq!(overview.contains(notice), simulated);
		assert!(!health.contains("simulated-notice"));

		let (_, changes) = page(&world, &mut ada, &format!("{project}/changes")).await;
		assert_eq!(changes.contains(notice), simulated);
		let plan = description_of(&changes, "action:plan");
		assert_eq!(plan.starts_with("Simulated: compares"), simulated, "{plan}");
		assert!(plan.contains("Planning changes nothing"), "{plan}");

		// A finished plan: its Apply confirmation says where it applies.
		let (_, body) = act(
			&world,
			&mut ada,
			json!({ "kind": "plan", "idempotency_key": "plan" }),
		)
		.await;
		let plan = body["operation"].as_str().unwrap().to_owned();
		planned(
			&world,
			&json!([{ "address": "hcloud_server.web[1]", "actions": ["create"] }]),
		);
		let (_, changes) = page(&world, &mut ada, &format!("{project}/changes")).await;
		let apply = description_of(&changes, &format!("action:apply:{plan}"));

		if simulated {
			assert!(
				apply.starts_with("Simulated: applies exactly this plan to a stand-in cloud"),
				"{apply}"
			);
			assert!(apply.contains("no provider is contacted"), "{apply}");
			assert!(!apply.contains("change to real infrastructure"), "{apply}");
		} else {
			assert_eq!(
				apply,
				"Applies exactly this plan with `lk infra apply`. Apply refuses it if the \
				 configuration, provider lock, or state changed since it was made. 1 change to \
				 real infrastructure."
			);
		}

		// Once it's applied, Learn's evidence doesn't claim real servers.
		let (status, _) = act(
			&world,
			&mut ada,
			json!({ "kind": "apply", "plan_id": plan, "idempotency_key": "apply" }),
		)
		.await;
		assert!(status.is_success(), "{status}");
		completed(&world);
		let (_, learn) = page(&world, &mut ada, &format!("{BASE}/learn")).await;
		assert!(
			learn.contains(if simulated {
				"You applied a plan on notes-production on the simulated cloud "
			} else {
				"You applied a plan on notes-production just now"
			}),
			"simulated = {simulated}: {learn}"
		);
	}
}

#[tokio::test]
async fn simulation_is_clear_for_releases_health_and_backups() {
	for simulated in [true, false] {
		let world = world_with_options(None, simulated).await;
		let mut ada = world.ada().await;
		let project = format!("{BASE}/infra/projects/notes-production");

		for version in ["v1", "v2"] {
			let (status, _) = act(
				&world,
				&mut ada,
				json!({ "kind": "deploy", "version": version, "idempotency_key": version }),
			)
			.await;
			assert!(status.is_success(), "{status}");
			completed(&world);
		}

		for (tab, action) in [
			("overview", "status"),
			("health", "status"),
			("releases", "deploy"),
			("releases", "first-release"),
			("releases", "maintenance"),
			("releases", "rollback:v1"),
			("backups", "backup"),
		] {
			let (_, html) = page(&world, &mut ada, &format!("{project}/{tab}")).await;
			let description = description_of(&html, &format!("action:{action}"));
			assert_eq!(
				description.starts_with("Simulated:"),
				simulated,
				"{description}"
			);
			assert_eq!(
				description.contains("no provider is contacted"),
				simulated,
				"{description}"
			);
		}

		for kind in ["rollback", "backup", "status"] {
			let (status, _) = act(
				&world,
				&mut ada,
				json!({ "kind": kind, "version": "v1", "idempotency_key": kind }),
			)
			.await;
			assert!(status.is_success(), "{status}");
			completed(&world);
		}

		let (_, html) = page(&world, &mut ada, &format!("{BASE}/learn")).await;
		for evidence in [
			"rolled back on the simulated cloud",
			"A simulated backup verification completed",
			"passed a simulated health check",
		] {
			assert_eq!(html.contains(evidence), simulated, "{evidence}");
		}
	}
}
