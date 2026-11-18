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
use support::shipping;
use support::shipping::Shop;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

#[tokio::test]
async fn a_deployment_goes_live() {
	shipping::a_deployment_goes_live(&support::app().await).await;
}

#[tokio::test]
async fn the_cluster_reports_what_it_runs() {
	shipping::the_cluster_reports_what_it_runs(&support::app().await).await;
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
