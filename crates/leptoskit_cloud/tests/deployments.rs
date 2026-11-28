//! Uploads, deployments, and the worker protocol on a real port over
//! SQLite. The shared scenarios are in `support/shipping.rs`, which
//! `postgres.rs` runs on Postgres too; this file adds what depends on the
//! server's configuration (a price catalogue, a storage cap), the live
//! socket, and the scans for presigned addresses.

#![cfg(not(target_arch = "wasm32"))]

mod support;

use std::time::Duration;

use futures::SinkExt as _;
use futures::StreamExt as _;
use leptoskit_rpc::live::ClientFrame;
use leptoskit_rpc::live::ServerFrame;
use leptoskit_rpc::live::Update;
use serde_json::Value;
use serde_json::json;
use support::backups;
use support::billing;
use support::config;
use support::databases;
use support::domains;
use support::limits;
use support::notifications;
use support::publishing;
use support::review;
use support::shipping;
use support::shipping::Shop;
use support::subscriptions;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

#[tokio::test]
async fn a_publish_carries_its_stores_credentials() {
	publishing::a_publish_carries_its_stores_credentials(&support::app().await).await;
}

#[tokio::test]
async fn a_publish_is_never_run_twice() {
	publishing::a_publish_is_never_run_twice(&support::app().await).await;
}

#[tokio::test]
async fn a_deployment_goes_live() {
	shipping::a_deployment_goes_live(&support::app().await).await;
}

#[tokio::test]
async fn the_cluster_reports_what_it_runs() {
	shipping::the_cluster_reports_what_it_runs(&support::app().await).await;
}

#[tokio::test]
async fn a_printed_secret_is_hidden_from_the_runtime_logs() {
	shipping::a_printed_secret_is_hidden_from_the_runtime_logs(&support::app().await).await;
}

#[tokio::test]
async fn a_report_from_another_cluster_opens_no_secret() {
	shipping::a_report_from_another_cluster_opens_no_secret(&support::app().await).await;
}

#[tokio::test]
async fn a_lapsed_build_is_retried_then_abandoned() {
	shipping::a_lapsed_build_is_retried_then_abandoned(&support::app().await).await;
}

#[tokio::test]
async fn a_lapsed_release_waits_for_reconciliation() {
	shipping::a_lapsed_release_waits_for_reconciliation(&support::app().await).await;
}

#[tokio::test]
async fn cancelling_stops_a_deployment() {
	shipping::cancelling_stops_a_deployment(&support::app().await).await;
}

#[tokio::test]
async fn uploads_are_verified_per_organization() {
	shipping::uploads_are_verified_per_organization(&support::app().await).await;
}

#[tokio::test]
async fn deploying_needs_a_cluster() {
	shipping::deploying_needs_a_cluster(&support::app().await).await;
}

#[tokio::test]
async fn workers_pair_once_and_can_be_revoked() {
	shipping::workers_pair_once_and_can_be_revoked(&support::app().await).await;
}

#[tokio::test]
async fn deleting_an_environment_tears_it_down() {
	shipping::deleting_an_environment_tears_it_down(&support::app().await).await;
}

#[tokio::test]
async fn build_logs_are_ordered_paged_and_expire() {
	shipping::build_logs_are_ordered_paged_and_expire(&support::app().await).await;
}

#[tokio::test]
async fn a_build_that_forgets_migrations_fails() {
	shipping::a_build_that_forgets_migrations_fails(&support::app().await).await;
}

#[tokio::test]
async fn races_have_one_winner() {
	shipping::races_have_one_winner(&support::app().await).await;
}

// What Control 2's security review found: `support/review.rs`.

#[tokio::test]
async fn an_upload_keeps_the_size_it_was_begun_with() {
	review::an_upload_keeps_the_size_it_was_begun_with(&support::app().await).await;
}

#[tokio::test]
async fn a_full_queue_never_hides_a_job_a_worker_may_take() {
	review::a_full_queue_never_hides_a_job_a_worker_may_take(&support::app().await).await;
}

#[tokio::test]
async fn a_build_attempt_has_a_deadline() {
	review::a_build_attempt_has_a_deadline(&support::app().await).await;
}

#[tokio::test]
async fn an_expired_lease_holds_nothing() {
	review::an_expired_lease_holds_nothing(&support::app().await).await;
}

#[tokio::test]
async fn a_build_cannot_name_another_repository() {
	review::a_build_cannot_name_another_repository(&support::app().await).await;
}

#[tokio::test]
async fn rollback_waits_for_a_certain_schema() {
	review::rollback_waits_for_a_certain_schema(&support::app().await).await;
}

#[tokio::test]
async fn the_largest_status_report_reads_back() {
	review::the_largest_status_report_reads_back(&support::app().await).await;
}

#[tokio::test]
async fn a_failed_teardown_stays_actionable() {
	review::a_failed_teardown_stays_actionable(&support::app().await).await;
}

#[tokio::test]
async fn a_swept_archive_never_takes_its_replacement() {
	review::a_swept_archive_never_takes_its_replacement(&support::app().await).await;
}

#[tokio::test]
async fn logs_hide_split_secrets_and_control_characters() {
	review::logs_hide_split_secrets_and_control_characters(&support::app().await).await;
}

#[tokio::test]
async fn a_release_is_reported_after_its_lease() {
	review::a_release_is_reported_after_its_lease(&support::app().await).await;
}

#[tokio::test]
async fn a_removal_that_then_works_is_done() {
	review::a_removal_that_then_works_is_done(&support::app().await).await;
}

#[tokio::test]
async fn a_builds_steps_are_bounded() {
	review::a_builds_steps_are_bounded(&support::app().await).await;
}

#[tokio::test]
async fn build_logs_are_capped() {
	let db = leptoskit_db::testing::sqlite_migrated(leptoskit_cloud::MIGRATIONS).await;
	let app = support::app_with(db, |config| review::settings(&mut config.shipping)).await;
	review::build_logs_are_capped(&app).await;
}

/// With a price catalogue, a preview estimates the month.
#[tokio::test]
async fn a_preview_estimates_with_the_catalogue() {
	let catalogue = leptoskit_cloud::server::catalogue_from_json(
		r#"{
			"currency": "usd",
			"plans": [{ "id": "hobby", "name": "Hobby", "monthly": 500, "included_usage": 500 }],
			"meters": { "compute_seconds_xs": { "minor": 1, "per": 3600 } }
		}"#,
	)
	.expect("a catalogue");
	let db = leptoskit_db::testing::sqlite_migrated(leptoskit_cloud::MIGRATIONS).await;
	let app = support::app_with(db, |config| config.catalogue = Some(catalogue)).await;
	let shop = Shop::open(&app).await;
	let preview = app
		.call(
			"preview_release",
			json!({
				"organization": shop.org,
				"project": "notes",
				"environment": "production",
				"config": { "cloud": { "environments": { "production": { "min": 1 } } } },
			}),
			shop.as_ana(),
		)
		.await;
	let preview = preview.expect(200).json();
	assert_eq!(preview["number"], 1);
	let cost = &preview["cost"];
	assert_eq!(cost["kind"], "estimate", "{preview}");
	assert_eq!(cost["currency"], "usd");
	assert_eq!(cost["plan"], "hobby");
	assert!(
		cost["assumptions"]
			.as_array()
			.is_some_and(|assumptions| !assumptions.is_empty())
	);
	// Nothing has started; an app with no database migrates nothing.
	let stamps: Vec<(&str, &str)> = preview["lines"]
		.as_array()
		.expect("lines")
		.iter()
		.map(|line| {
			(
				line["kind"]["kind"].as_str().expect("a kind"),
				line["stamp"].as_str().expect("a stamp"),
			)
		})
		.collect();
	assert_eq!(
		stamps,
		[
			("server", "pending"),
			("migrations", "skipped"),
			("web_assets", "pending"),
		]
	);
}

/// An organisation's archives are capped: past it, `begin_upload` says so
/// and nothing is stored.
#[tokio::test]
async fn archives_are_capped_per_organization() {
	let db = leptoskit_db::testing::sqlite_migrated(leptoskit_cloud::MIGRATIONS).await;
	let app = support::app_with(db, |config| config.shipping.storage_cap = 40).await;
	let shop = Shop::open(&app).await;
	shop.upload(&[1; 30]).await;
	let began = shipping::begin(&app, shop.as_ana(), &shop.org, "notes", &[2; 30]).await;
	began.refused(409, "storage_full");

	// The same bytes again aren't new storage.
	let again = shipping::begin(&app, shop.as_ana(), &shop.org, "notes", &[1; 30]).await;
	assert_eq!(again.expect(200).json()["status"], "stored");
}

/// Presigned addresses are credentials: none is kept in any table, the
/// RPC's receipts included, and none appears in a deployment's sheet or
/// events.
#[tokio::test]
async fn presigned_addresses_are_never_stored() {
	let app = support::app().await;
	let shop = Shop::open(&app).await;
	let bytes = b"signed and sealed";
	let began = shipping::begin(&app, shop.as_ana(), &shop.org, "notes", bytes).await;
	let ticket = began.expect(200).json()["ticket"].clone();
	let upload_url = ticket["url"].as_str().expect("an address").to_owned();
	assert_eq!(shipping::put(&app, &ticket, bytes).await, 200);
	let digest = shipping::digest_of(bytes);
	shipping::complete(&app, shop.as_ana(), &shop.org, "notes", &digest)
		.await
		.expect(200);
	shop.deploy("production", &digest, json!({}))
		.await
		.expect(200);
	let build = shop.build.take().await;
	let download_url = build["input"]["archive"]["url"]
		.as_str()
		.expect("an address")
		.to_owned();

	for url in [&upload_url, &download_url] {
		let signature = signature(url);
		assert!(signature.len() >= 32, "{url}");
		assert_eq!(
			app.find_in_every_table(&signature).await,
			Vec::<String>::new()
		);
	}

	let sheet = shop.sheet("production", 1).await.to_string();
	let events = shop
		.on("deployment_events", "production", 1)
		.await
		.expect(200)
		.text
		.clone();

	for text in [sheet, events] {
		assert!(!text.contains("sig="), "{text}");
		assert!(!text.contains("_leptoskit/storage"), "{text}");
	}
}

/// The `sig` query parameter of a presigned local address.
fn signature(url: &str) -> String {
	let query = url.split_once('?').map_or("", |(_, query)| query);
	query
		.split('&')
		.find_map(|pair| pair.strip_prefix("sig="))
		.expect("a signature")
		.to_owned()
}

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

async fn send_frame(socket: &mut Socket, frame: ClientFrame) {
	let text = serde_json::to_string(&frame).expect("a frame");
	socket.send(Message::Text(text.into())).await.expect("sent");
}

async fn next_frame(socket: &mut Socket) -> ServerFrame {
	loop {
		let message = tokio::time::timeout(Duration::from_secs(20), socket.next())
			.await
			.expect("a frame arrives")
			.expect("the socket is open")
			.expect("a message");

		match message {
			Message::Text(text) => return serde_json::from_str(&text).expect("a server frame"),
			Message::Ping(_) | Message::Pong(_) => {}
			other => panic!("unexpected frame: {other:?}"),
		}
	}
}

/// The live socket, signed in with Ana's session.
async fn socket(app: &support::App, cookie: &str) -> Socket {
	let mut request = format!("ws://{}/live", app.address)
		.into_client_request()
		.expect("a request");
	let headers = request.headers_mut();
	headers.insert("cookie", cookie.parse().expect("a cookie header"));
	headers.insert("origin", app.origin.parse().expect("an origin header"));
	let (mut socket, _) = tokio_tungstenite::connect_async(request)
		.await
		.expect("the live socket opens");
	assert!(
		matches!(next_frame(&mut socket).await, ServerFrame::Welcome { .. }),
		"welcome first"
	);
	socket
}

/// Subscribes to `query` as subscription `id`, returning its first value.
async fn subscribe(socket: &mut Socket, id: u32, query: &str, args: Value) -> Value {
	send_frame(
		socket,
		ClientFrame::Sub {
			id,
			query: query.to_owned(),
			args,
		},
	)
	.await;

	loop {
		match next_frame(socket).await {
			ServerFrame::Done {
				request,
				ok: Some(value),
				err: None,
				..
			} if request == id => return value,
			ServerFrame::Done { request, err, .. } if request == id => {
				panic!("subscription {id} failed: {err:?}")
			}
			_ => {}
		}
	}
}

/// Waits for subscription `id` to change, and returns whether its new
/// value, a whole value or a patch, mentions `needle`.
async fn changes_to_mention(socket: &mut Socket, id: u32, needle: &str) {
	loop {
		if let ServerFrame::Transition(transition) = next_frame(socket).await {
			for update in transition.updates.iter().filter(|update| update.id == id) {
				let text = match &update.update {
					Update::Ok(value) => value.to_string(),
					Update::Patch(patch) => format!("{patch:?}"),
					Update::Err(error) => panic!("subscription {id} failed: {error:?}"),
				};

				if text.contains(needle) {
					return;
				}
			}
		}
	}
}

/// A deployment's events, its environment's deployments, and the
/// organisation's in-flight list are live: a worker's claim and finish
/// reach an open screen with no polling.
#[tokio::test]
async fn deployments_are_live() {
	let app = support::app().await;
	let shop = Shop::open(&app).await;
	let digest = shop.upload(b"watch me").await;
	shop.deploy("production", &digest, json!({}))
		.await
		.expect(200);
	let mut screen = socket(&app, &shop.ana).await;
	let place = json!({
		"organization": shop.org,
		"project": "notes",
		"environment": "production",
		"number": 1,
	});
	let events = subscribe(&mut screen, 1, "deployment_events", place).await;
	assert!(!events.to_string().contains("claimed"), "{events}");
	let listed = subscribe(
		&mut screen,
		2,
		"in_flight",
		json!({ "organization": shop.org }),
	)
	.await;
	assert!(listed.to_string().contains("queued"), "{listed}");

	let build = shop.build.take().await;
	changes_to_mention(&mut screen, 1, "claimed").await;
	shop.build
		.finish(&build, shipping::built(&build, b"image", &[]))
		.await
		.expect(200);
	changes_to_mention(&mut screen, 1, "rolling_out").await;
	let release = shop.cluster.take().await;
	shop.cluster
		.finish(&release, shipping::applied())
		.await
		.expect(200);
	changes_to_mention(&mut screen, 1, "live").await;
}

#[tokio::test]
async fn variables_and_secrets_reach_the_release() {
	config::variables_and_secrets_reach_the_release(&support::app().await).await;
}

#[tokio::test]
async fn names_and_values_are_checked() {
	config::names_and_values_are_checked(&support::app().await).await;
}

#[tokio::test]
async fn configuration_is_the_tenants_alone() {
	config::configuration_is_the_tenants_alone(&support::app().await).await;
}

#[tokio::test]
async fn a_release_always_finds_its_secrets() {
	config::a_release_always_finds_its_secrets(&support::app().await).await;
}

#[tokio::test]
async fn a_copied_secret_opens_nowhere_else() {
	config::a_copied_secret_opens_nowhere_else(&support::app().await).await;
}

#[tokio::test]
async fn first_secrets_race_to_one_key() {
	config::first_secrets_race_to_one_key(&support::app().await).await;
}

#[tokio::test]
async fn configuration_has_a_size() {
	config::configuration_has_a_size(&support::app().await).await;
}

#[tokio::test]
async fn a_removed_secret_never_comes_back() {
	config::a_removed_secret_never_comes_back(&support::app().await).await;
}

#[tokio::test]
async fn a_verified_domain_reaches_the_release() {
	domains::a_verified_domain_reaches_the_release(&support::app().await).await;
}

#[tokio::test]
async fn domains_are_checked_and_the_tenants_alone() {
	domains::domains_are_checked_and_the_tenants_alone(&support::app().await).await;
}

#[tokio::test]
async fn a_hostname_is_one_environments() {
	domains::a_hostname_is_one_environments(&support::app().await).await;
}

#[tokio::test]
async fn verifying_races_to_one_holder() {
	domains::verifying_races_to_one_holder(&support::app().await).await;
}

#[tokio::test]
async fn a_detached_domain_is_held_until_it_stops_serving() {
	domains::a_detached_domain_is_held_until_it_stops_serving(&support::app().await).await;
}

#[tokio::test]
async fn a_hostname_is_held_while_the_cluster_may_route_it() {
	domains::a_hostname_is_held_while_the_cluster_may_route_it(&support::app().await).await;
}

#[tokio::test]
async fn a_postgres_app_gets_a_database() {
	databases::a_postgres_app_gets_a_database(&support::app().await).await;
}

#[tokio::test]
async fn a_database_password_opens_nowhere_else() {
	databases::a_database_password_opens_nowhere_else(&support::app().await).await;
}

#[tokio::test]
async fn a_rollback_finds_its_database_where_it_is_now() {
	databases::a_rollback_finds_its_database_where_it_is_now(&support::app().await).await;
}

#[tokio::test]
async fn a_deleted_environment_takes_its_database() {
	databases::a_deleted_environment_takes_its_database(&support::app().await).await;
}

#[tokio::test]
async fn a_clusters_postgres_is_checked() {
	databases::a_clusters_postgres_is_checked(&support::app().await).await;
}

#[tokio::test]
async fn a_failed_release_does_not_print_the_password() {
	databases::a_failed_release_does_not_print_the_password(&support::app().await).await;
}

#[tokio::test]
async fn an_app_with_notifications_is_served_by_the_hub() {
	notifications::an_app_with_notifications_is_served_by_the_hub(&support::app().await).await;
}

#[tokio::test]
async fn a_tenant_lives_with_its_releases_and_its_environment() {
	notifications::a_tenant_lives_with_its_releases_and_its_environment(&support::app().await)
		.await;
}

#[tokio::test]
async fn revoking_a_worker_gives_its_tenants_new_values() {
	notifications::revoking_a_worker_gives_its_tenants_new_values(&support::app().await).await;
}

#[tokio::test]
async fn a_hub_serves_only_its_own_cluster() {
	notifications::a_hub_serves_only_its_own_cluster(&support::app().await).await;
}

#[tokio::test]
async fn a_backup_is_taken_and_listed() {
	backups::a_backup_is_taken_and_listed(&support::app().await).await;
}

#[tokio::test]
async fn a_backup_is_restored() {
	backups::a_backup_is_restored(&support::app().await).await;
}

#[tokio::test]
async fn a_quiet_worker_fails_a_backup_and_leaves_a_restore_unknown() {
	backups::a_quiet_worker_fails_a_backup_and_leaves_a_restore_unknown(&support::app().await)
		.await;
}

#[tokio::test]
async fn a_deleted_environment_takes_its_backups() {
	backups::a_deleted_environment_takes_its_backups(&support::app().await).await;
}

#[tokio::test]
async fn a_free_plan_stops_pushing_at_its_allowance() {
	let db = leptoskit_db::testing::sqlite_migrated(leptoskit_cloud::MIGRATIONS).await;
	let app = support::app_with(db, |config| {
		config.catalogue = Some(notifications::catalogue());
	})
	.await;
	notifications::a_free_plan_stops_pushing_at_its_allowance(&app).await;
}

#[tokio::test]
async fn usage_is_counted_priced_and_capped() {
	let db = leptoskit_db::testing::sqlite_migrated(leptoskit_cloud::MIGRATIONS).await;
	let app = support::app_with(db, |config| config.catalogue = Some(billing::catalogue())).await;
	billing::usage_is_counted_priced_and_capped(&app).await;
}

/// Past its spend cap, an organisation's `rollback` is refused as its `deploy`
/// is and makes nothing; under the cap, a rollback still goes out.
#[tokio::test]
async fn rollback_is_refused_once_the_spend_cap_is_reached() {
	let db = leptoskit_db::testing::sqlite_migrated(leptoskit_cloud::MIGRATIONS).await;
	let app = support::app_with(db, |config| config.catalogue = Some(billing::catalogue())).await;
	let shop = Shop::open(&app).await;
	let cap_reached = || {
		async {
			app.call("usage", json!({ "organization": shop.org }), shop.as_ana())
				.await
				.expect(200)
				.json()["cost"]["cap_reached"]
				.clone()
		}
	};
	let rollback = |caller| {
		app.call(
			"rollback",
			json!({
				"organization": shop.org,
				"project": "notes",
				"environment": "production",
				"to": 1,
			}),
			caller,
		)
	};

	// Hobby includes two build minutes, and a build counts as at least one.
	// Two spent: the cap isn't reached, and a rollback goes out.
	shop.ship("production", b"first", &[]).await;
	shop.ship("production", b"second", &[]).await;
	assert_eq!(cap_reached().await, false);
	let rolled = rollback(shop.as_ana()).await;
	let rolled = rolled.expect(200).json();
	assert_eq!(rolled["number"], 3);
	assert_eq!(rolled["rollback_of"], 1);
	let release = shop.cluster.take().await;
	assert_eq!(release["input"]["previous"], 2);
	shop.cluster
		.finish(&release, shipping::applied())
		.await
		.expect(200);
	assert_eq!(
		shop.state("production", 3).await,
		json!({ "state": "live" })
	);

	// The third minute is past a cap of zero. `deploy` is refused, and
	// `rollback` answers exactly as it does: for the owner, a developer and
	// a deploy token alike.
	shop.ship("production", b"third", &[]).await;
	assert_eq!(cap_reached().await, true);
	let digest = shop.upload(b"fourth").await;
	let deployed = shop
		.deploy("production", &digest, json!({ "database": "sqlite" }))
		.await;
	deployed.refused(403, "spend_cap_reached");
	let dev = app
		.join(shop.as_ana(), &shop.org, "dev@example.com", "developer")
		.await;
	let (token, _) = app.token(shop.as_ana(), &shop.org, &["deploy"], None).await;
	for caller in [
		shop.as_ana(),
		support::As::Session(&dev),
		support::As::Bearer(&token),
	] {
		let rolled = rollback(caller).await;
		rolled.refused(403, "spend_cap_reached");
		assert_eq!(rolled.text, deployed.text, "the refusal `deploy` gives");
	}
	// Nothing was made, and nothing that runs was touched.
	shop.on("deployment", "production", 5).await.expect(404);
	assert!(shop.cluster.claim().await.is_none(), "nothing was queued");
	assert_eq!(
		shop.state("production", 4).await,
		json!({ "state": "live" })
	);
}

#[tokio::test]
async fn a_plans_limits_are_counted() {
	let db = leptoskit_db::testing::sqlite_migrated(leptoskit_cloud::MIGRATIONS).await;
	let app = support::app_with(db, |config| config.catalogue = Some(billing::catalogue())).await;
	billing::a_plans_limits_are_counted(&app).await;
}

#[tokio::test]
async fn a_build_whose_lease_ran_out_is_counted() {
	let db = leptoskit_db::testing::sqlite_migrated(leptoskit_cloud::MIGRATIONS).await;
	let app = support::app_with(db, |config| config.catalogue = Some(billing::catalogue())).await;
	billing::a_build_whose_lease_ran_out_is_counted(&app).await;
}

#[tokio::test]
async fn a_plan_follows_the_providers_subscription() {
	let till = subscriptions::Till::default();
	let db = leptoskit_db::testing::sqlite_migrated(leptoskit_cloud::MIGRATIONS).await;
	let app = support::app_with(db, subscriptions::taking_payment(&till)).await;
	subscriptions::a_plan_follows_the_providers_subscription(&app, &till).await;
}

#[tokio::test]
async fn a_missed_event_is_caught_up_with() {
	let till = subscriptions::Till::default();
	let db = leptoskit_db::testing::sqlite_migrated(leptoskit_cloud::MIGRATIONS).await;
	let app = support::app_with(db, subscriptions::taking_payment(&till)).await;
	subscriptions::a_missed_event_is_caught_up_with(&app, &till).await;
}

#[tokio::test]
async fn an_own_cap_is_held_to_the_plan_the_organisation_moves_to() {
	let till = subscriptions::Till::default();
	let db = leptoskit_db::testing::sqlite_migrated(leptoskit_cloud::MIGRATIONS).await;
	let app = support::app_with(db, subscriptions::taking_payment_for_two_plans(&till)).await;
	subscriptions::an_own_cap_is_held_to_the_plan_the_organisation_moves_to(&app, &till).await;
}

#[tokio::test]
async fn an_estimate_prices_a_table() {
	let db = leptoskit_db::testing::sqlite_migrated(leptoskit_cloud::MIGRATIONS).await;
	let app = support::app_with(db, |config| config.catalogue = Some(billing::catalogue())).await;
	billing::an_estimate_prices_a_table(&app).await;
}

#[tokio::test]
async fn a_caller_who_asks_too_often_waits() {
	let db = leptoskit_db::testing::sqlite_migrated(leptoskit_cloud::MIGRATIONS).await;
	let app = support::app_with(db, limits::slow).await;
	limits::a_caller_who_asks_too_often_waits(&app).await;
}

#[tokio::test]
async fn payment_pages_are_handed_out_slowly() {
	let till = subscriptions::Till::default();
	let db = leptoskit_db::testing::sqlite_migrated(leptoskit_cloud::MIGRATIONS).await;
	let app = support::app_with(db, limits::two_pages(&till)).await;
	limits::payment_pages_are_handed_out_slowly(&app, &till).await;
}

#[tokio::test]
async fn without_payment_nothing_is_sold() {
	subscriptions::without_payment_nothing_is_sold(&support::app().await).await;
}

#[tokio::test]
async fn plan_scale_limits_block_oversized_environments_and_deploys() {
	let catalogue = leptoskit_cloud::server::catalogue_from_json(
		r#"{
			"currency": "usd",
			"plans": [{
				"id": "hobby",
				"name": "Hobby",
				"monthly": 0,
				"limits": { "max_size": "xs", "max_replicas": 1, "require_sleep": true }
			}],
			"meters": {
				"compute_seconds_xs": { "minor": 1, "per": 3600 },
				"build_minutes_linux": { "minor": 10 }
			}
		}"#,
	)
	.expect("a catalogue");
	let db = leptoskit_db::testing::sqlite_migrated(leptoskit_cloud::MIGRATIONS).await;
	let app = support::app_with(db, |config| config.catalogue = Some(catalogue)).await;
	let shop = Shop::open(&app).await;

	// Oversized process rejected on update_environment.
	app.call(
		"update_environment",
		json!({
			"organization": shop.org,
			"project": "notes",
			"environment": "production",
			"revision": 0,
			"scale": {
				"app": { "size": "xl", "min": 0, "max": 1, "sleep": "when-idle" },
				"services": {}
			}
		}),
		shop.as_ana(),
	)
	.await
	.refused(403, "quota_exceeded");

	// Always-on process rejected when plan requires sleep.
	app.call(
		"update_environment",
		json!({
			"organization": shop.org,
			"project": "notes",
			"environment": "production",
			"revision": 0,
			"scale": {
				"app": { "size": "xs", "min": 1, "max": 1, "sleep": "never" },
				"services": {}
			}
		}),
		shop.as_ana(),
	)
	.await
	.refused(403, "quota_exceeded");

	// Multiple replicas rejected when max_replicas == 1.
	app.call(
		"update_environment",
		json!({
			"organization": shop.org,
			"project": "notes",
			"environment": "production",
			"revision": 0,
			"scale": {
				"app": { "size": "xs", "min": 0, "max": 2, "sleep": "when-idle" },
				"services": {}
			}
		}),
		shop.as_ana(),
	)
	.await
	.refused(403, "quota_exceeded");
}

#[tokio::test]
async fn bootstrap_operator_is_idempotent_without_email_sign_in() {
	let db = leptoskit_db::testing::sqlite_migrated(leptoskit_cloud::MIGRATIONS).await;
	let first = leptoskit_cloud::server::bootstrap_operator(&db, "ops@example.com")
		.await
		.expect("bootstrap succeeds");
	let second = leptoskit_cloud::server::bootstrap_operator(&db, "OPS@EXAMPLE.COM")
		.await
		.expect("second bootstrap is idempotent");
	assert_eq!(first, second);
	let list = leptoskit_cloud::server::operators(&db).await.unwrap();
	assert_eq!(list, vec!["ops@example.com".to_owned()]);
}

#[tokio::test]
async fn update_environment_scales_live_deployment_and_next_deploy_clears_scale_differs() {
	let app = support::app().await;
	let shop = Shop::open(&app).await;
	let first = shop.ship("production", b"v1", &[]).await;
	assert_eq!(first, 1);

	// Read current revision before scaling.
	let envs = app
		.call(
			"environments",
			json!({ "organization": shop.org, "project": "notes" }),
			shop.as_ana(),
		)
		.await
		.expect(200)
		.json();
	let rev = envs[0]["revision"].as_i64().unwrap();

	// Updating scale on a live environment queues an immediate cluster release
	// job.
	let updated = app
		.call(
			"update_environment",
			json!({
				"organization": shop.org,
				"project": "notes",
				"environment": "production",
				"revision": rev,
				"scale": {
					"app": { "size": "s", "min": 1, "max": 1, "sleep": "never" },
					"services": {}
				}
			}),
			shop.as_ana(),
		)
		.await
		.expect(200)
		.json();
	assert_eq!(updated["scale_differs"], true);

	// The cluster worker can immediately claim and finish the scaling release
	// job.
	let claimed = shop
		.cluster
		.claim()
		.await
		.expect("immediate scaling release job");
	assert_eq!(
		claimed["input"]["kind"], "release",
		"expected immediate release job: {claimed}"
	);
	let spec = &claimed["input"]["spec"];
	assert_eq!(spec["processes"][0]["resources"]["memory_mib"], 256);
	assert_eq!(spec["processes"][0]["scale"]["min"], 1);
	assert_eq!(spec["processes"][0]["scale"]["max"], 1);
	shop.cluster
		.finish(&claimed, shipping::applied())
		.await
		.expect(200);

	// A subsequent deploy consumes the override and clears scale_differs so
	// future leptoskit.toml edits are not permanently locked out.
	let _second = shop.ship("production", b"v2", &[]).await;
	let envs_after = app
		.call(
			"environments",
			json!({ "organization": shop.org, "project": "notes" }),
			shop.as_ana(),
		)
		.await
		.expect(200)
		.json();
	assert_eq!(envs_after[0]["scale_differs"], false);
}

#[tokio::test]
async fn manage_only_token_saves_scale_without_triggering_live_cluster_rollout() {
	let app = support::app().await;
	let shop = Shop::open(&app).await;
	shop.ship("production", b"v1", &[]).await;

	let (manage_token, _) = app.token(shop.as_ana(), &shop.org, &["manage"], None).await;
	let envs = app
		.call(
			"environments",
			json!({ "organization": shop.org, "project": "notes" }),
			shop.as_ana(),
		)
		.await
		.expect(200)
		.json();
	let rev = envs[0]["revision"].as_i64().unwrap();

	// Manage-only token updates the saved scale but does NOT queue a cluster
	// release because it lacks Permission::Deploy.
	let updated = app
		.call(
			"update_environment",
			json!({
				"organization": shop.org,
				"project": "notes",
				"environment": "production",
				"revision": rev,
				"scale": {
					"app": { "size": "s", "min": 1, "max": 1, "sleep": "never" },
					"services": {}
				}
			}),
			support::As::Bearer(&manage_token),
		)
		.await
		.expect(200)
		.json();
	assert_eq!(updated["scale_differs"], true);
	assert!(
		shop.cluster.claim().await.is_none(),
		"manage-only token must not queue a live cluster rollout"
	);
}

#[tokio::test]
async fn cluster_environments_returns_placed_environments_for_rehydration() {
	let app = support::app().await;
	let shop = Shop::open(&app).await;
	shop.ship("production", b"v1", &[]).await;

	// Cluster worker sees placed environments for startup rehydration.
	let listed = shop
		.cluster
		.call("cluster_environments", json!({}))
		.await
		.expect(200)
		.json();
	let items = listed.as_array().expect("array");
	assert_eq!(items.len(), 1);
	assert_eq!(items[0]["environment"]["organization"]["slug"], shop.org);
	assert_eq!(items[0]["environment"]["project"]["slug"], "notes");
	assert_eq!(items[0]["environment"]["environment"]["slug"], "production");

	// Build worker is refused.
	shop.build
		.call("cluster_environments", json!({}))
		.await
		.refused(403, "cluster_worker_required");
}

#[tokio::test]
async fn plans_expose_scale_limits_and_catalogue_rejects_zero_max_replicas() {
	assert!(
		leptoskit_cloud::server::catalogue_from_json(
			r#"{
				"currency": "usd",
				"plans": [{
					"id": "broken",
					"name": "Broken",
					"monthly": 0,
					"limits": { "max_replicas": 0 }
				}],
				"meters": {}
			}"#,
		)
		.is_err()
	);

	let catalogue = leptoskit_cloud::server::catalogue_from_json(
		r#"{
			"currency": "usd",
			"plans": [{
				"id": "hobby",
				"name": "Hobby",
				"monthly": 0,
				"limits": { "max_size": "xs", "max_replicas": 1, "require_sleep": true }
			}],
			"meters": {}
		}"#,
	)
	.expect("valid catalogue");
	let db = leptoskit_db::testing::sqlite_migrated(leptoskit_cloud::MIGRATIONS).await;
	let app = support::app_with(db, |config| config.catalogue = Some(catalogue)).await;
	let ana = app.sign_in("ana@example.com").await;
	let plans = app
		.call("plans", json!({}), support::As::Session(&ana))
		.await
		.expect(200)
		.json();
	let limits = &plans["plans"][0]["limits"];
	assert_eq!(limits["max_size"], "xs");
	assert_eq!(limits["max_replicas"], 1);
	assert_eq!(limits["require_sleep"], true);
}

#[tokio::test]
async fn bootstrap_operator_rejects_invalid_and_disabled_accounts_and_handles_concurrency() {
	let db = leptoskit_db::testing::sqlite_migrated(leptoskit_cloud::MIGRATIONS).await;
	let invalid = leptoskit_cloud::server::bootstrap_operator(&db, "not-an-email").await;
	assert!(matches!(
		invalid,
		Err(leptoskit_cloud::server::OperatorError::InvalidEmail(_))
	));

	// Concurrent bootstrap for the same email succeeds cleanly.
	let (first, second) = tokio::join!(
		leptoskit_cloud::server::bootstrap_operator(&db, "ops@example.com"),
		leptoskit_cloud::server::bootstrap_operator(&db, "OPS@EXAMPLE.COM"),
	);
	let id1 = first.expect("first concurrent bootstrap");
	let id2 = second.expect("second concurrent bootstrap");
	assert_eq!(id1, id2);

	// Disabled accounts cannot be bootstrapped into operators.
	db.run_script("UPDATE \"auth_accounts\" SET \"disabled_at\" = '2026-01-01T00:00:00Z'")
		.await
		.expect("disable account");
	let disabled = leptoskit_cloud::server::bootstrap_operator(&db, "ops@example.com").await;
	assert!(matches!(
		disabled,
		Err(leptoskit_cloud::server::OperatorError::DisabledAccount(_))
	));
}

#[tokio::test]
async fn runtime_log_cap_tracks_usage_and_recovers_after_expiry_sweep() {
	let db = leptoskit_db::testing::sqlite_migrated(leptoskit_cloud::MIGRATIONS).await;
	let app = support::app_with(db, |config| {
		// Fit only one small chunk (~95 bytes) before dropping overflow.
		config.shipping.organization_runtime_log_cap = 140;
		config.shipping.runtime_logs = Duration::from_millis(50);
	})
	.await;
	let shop = Shop::open(&app).await;
	shop.ship("production", b"v1", &[]).await;

	let envs = app
		.call(
			"environments",
			json!({ "organization": shop.org, "project": "notes" }),
			shop.as_ana(),
		)
		.await
		.expect(200)
		.json();
	let env_id = envs[0]["id"].as_str().unwrap();
	let listed = shop
		.cluster
		.call("cluster_environments", json!({}))
		.await
		.expect(200)
		.json();
	let env_ref = &listed[0]["environment"];

	// First status report stores its log chunk.
	let first = shop
		.cluster
		.call(
			"report_status",
			json!({
				"environment": env_id,
				"observed": { "environment": env_ref, "processes": [] },
				"logs": [{
					"process": "app",
					"pod": "app-0",
					"at_millis": 1_700_000_000_000_i64,
					"message": "hello first chunk"
				}]
			}),
		)
		.await
		.expect(200)
		.json();
	assert_eq!(first["log_lines"], 1);

	// Second report exceeds cap: status succeeds, log lines dropped.
	let second = shop
		.cluster
		.call(
			"report_status",
			json!({
				"environment": env_id,
				"observed": { "environment": env_ref, "processes": [] },
				"logs": [{
					"process": "app",
					"pod": "app-0",
					"at_millis": 1_700_000_001_000_i64,
					"message": "this second chunk exceeds the 220 byte cap"
				}]
			}),
		)
		.await
		.expect(200)
		.json();
	assert_eq!(second["log_lines"], 0);

	// After retention elapses and sweeper runs, runtime_bytes decrements and
	// fresh log lines are accepted again.
	tokio::time::sleep(Duration::from_millis(80)).await;
	app.control.sweep().await.expect("sweep");
	let third = shop
		.cluster
		.call(
			"report_status",
			json!({
				"environment": env_id,
				"observed": { "environment": env_ref, "processes": [] },
				"logs": [{
					"process": "app",
					"pod": "app-0",
					"at_millis": 1_700_000_002_000_i64,
					"message": "accepted after sweep"
				}]
			}),
		)
		.await
		.expect(200)
		.json();
	assert_eq!(third["log_lines"], 1);
}
