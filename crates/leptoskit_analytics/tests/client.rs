//! The server's client against the service on a real port: events queued
//! in the background are stored as the server's, and a service that can't
//! be reached keeps them queued.

#![cfg(not(target_arch = "wasm32"))]

use leptoskit_analytics::client::server::Analytics;
use leptoskit_analytics::client::server::Config;
use leptoskit_analytics::props;
use leptoskit_analytics::service;
use leptoskit_analytics::service::IngestConfig;
use leptoskit_db::Db;
use leptoskit_db::Read as _;
use leptoskit_db::Statement;
use leptoskit_rpc as rpc;
use rpc::server::Anonymous;
use rpc::server::ServiceCallers;

const TOKEN: &str = "app-token";

async fn start() -> (String, Db) {
	let db = leptoskit_db::testing::sqlite_migrated(service::MIGRATIONS).await;
	let router = service::routes_with(
		db.clone(),
		None,
		IngestConfig::default(),
		ServiceCallers::new(Anonymous).caller(TOKEN, rpc::server::APP_CALLER),
	);
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	tokio::spawn(async move {
		leptoskit_axum::axum::serve(listener, router).await.unwrap();
	});
	(format!("http://{address}"), db)
}

async fn scalar(db: &Db, sql: &str) -> String {
	let rows = db
		.fetch(Statement {
			sql: sql.into(),
			args: Vec::new(),
		})
		.await
		.unwrap();
	format!("{:?}", rows[0][0])
}

#[tokio::test(flavor = "multi_thread")]
async fn the_server_s_events_are_stored_as_the_server_s() {
	let (url, db) = start().await;
	let analytics = Analytics::new(&url, Some(TOKEN.into()), Config::default());

	for n in 0..250 {
		analytics.capture(
			&format!("user-{}", n % 5),
			"invoice_paid",
			props! { "amount_cents" => n * 100, "currency" => "GBP" },
		);
	}
	analytics.identify("user-1", "anon-1", props! { "email" => "ada@example.com" });
	analytics.group_identify("company", "acme", props! { "name" => "Acme" });
	analytics.flush().await.unwrap();
	assert_eq!(analytics.waiting(), 0);

	assert_eq!(scalar(&db, "SELECT COUNT(*) FROM events").await, "Int(252)");
	assert_eq!(
		scalar(&db, "SELECT COUNT(*) FROM events WHERE source = 2").await,
		"Int(252)",
		"signed as the app, so recorded as the server's"
	);
	assert_eq!(
		scalar(
			&db,
			"SELECT COUNT(*) FROM events WHERE name = 'invoice_paid' AND distinct_id = 'user-3'"
		)
		.await,
		"Int(50)"
	);
	assert_eq!(
		scalar(
			&db,
			"SELECT properties FROM events WHERE name = '$identify'"
		)
		.await,
		r#"Text("{\"$anon_distinct_id\":\"anon-1\",\"$set\":{\"email\":\"ada@example.com\"}}")"#
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_background_task_sends_without_a_flush() {
	let (url, db) = start().await;
	let analytics = Analytics::new(
		&url,
		Some(TOKEN.into()),
		Config {
			interval: std::time::Duration::from_millis(20),
			..Config::default()
		},
	);
	analytics.capture("someone", "signed_up", props! {});
	for _ in 0..200 {
		if scalar(&db, "SELECT COUNT(*) FROM events").await == "Int(1)" {
			return;
		}
		tokio::time::sleep(std::time::Duration::from_millis(10)).await;
	}
	panic!("the event wasn't sent");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreachable_service_keeps_the_events_queued() {
	// Nothing listens on a port that was just freed.
	let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
	let url = format!("http://{}", listener.local_addr().unwrap());
	drop(listener);

	let analytics = Analytics::new(&url, None, Config::default());
	analytics.capture("someone", "signed_up", props! {});
	let error = analytics.flush().await.unwrap_err();
	assert_eq!(error.waiting, 1);
	assert_eq!(analytics.waiting(), 1);
}
