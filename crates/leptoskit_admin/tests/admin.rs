//! The admin end to end: signing in through the main site, refusing people
//! who aren't admins, acting on the control service as the signed-in
//! admin, streaming progress, and what each key screen shows.

mod support;

use std::time::Duration;

use axum::http::StatusCode;
use http_body_util::BodyExt as _;
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

	let response = ada
		.get(
			&world.admin,
			ADMIN,
			&format!("{BASE}/infra/operations/{id}/events"),
		)
		.await;
	assert_eq!(response.status(), StatusCode::OK);
	assert_eq!(response.headers()["content-type"], "text/event-stream");
	let mut body = response.into_body();
	let mut next = async || {
		let frame = tokio::time::timeout(Duration::from_secs(5), body.frame())
			.await
			.expect("an event in time")
			.unwrap()
			.unwrap();
		String::from_utf8(frame.into_data().unwrap().to_vec()).unwrap()
	};

	let first = next().await;
	assert!(first.contains("event: operation"), "{first}");
	assert!(first.contains("ada@example.com"), "{first}");

	world
		.service
		.progress(&id, "Reading the provider state")
		.unwrap();
	let progress = next().await;
	assert!(progress.contains("event: progress"), "{progress}");
	assert!(
		progress.contains("Reading the provider state"),
		"{progress}"
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

	// An Analytics screen teaches rather than inventing numbers.
	let (_, html) = page(&world, &mut ada, &format!("{BASE}/analytics/overview")).await;
	assert!(html.contains("Waiting for your first event"));
	insta::assert_snapshot!(
		"analytics_overview",
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
