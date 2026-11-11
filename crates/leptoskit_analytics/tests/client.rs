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

/// Collected at build time and registered by the server's client.
#[derive(leptoskit_analytics::Event)]
#[event(name = "invoice_sent")]
struct InvoiceSent {
	amount_cents: u32,
	currency: String,
}

#[tokio::test(flavor = "multi_thread")]
async fn the_server_registers_its_typed_events_and_they_arrive_named() {
	use leptoskit_analytics::Event as _;

	let (url, db) = start().await;
	let analytics = Analytics::new(&url, Some(TOKEN.into()), Config::default());
	analytics.capture_event(
		"user-9",
		InvoiceSent {
			amount_cents: 2500,
			currency: "EUR".into(),
		},
	);
	analytics.flush().await.unwrap();

	assert_eq!(
		scalar(
			&db,
			&format!(
				"SELECT name FROM event_schemas WHERE schema_id = {}",
				InvoiceSent::SCHEMA_ID as i64
			)
		)
		.await,
		r#"Text("invoice_sent")"#,
		"registered from the build"
	);
	assert_eq!(
		scalar(
			&db,
			"SELECT properties FROM events WHERE name = 'invoice_sent'"
		)
		.await,
		r#"Text("{\"amount_cents\":2500,\"currency\":\"EUR\"}")"#
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_schema_the_service_doesn_t_know_is_sent_again_with_names() {
	use leptoskit_analytics::client::Limits;
	use leptoskit_analytics::client::Queue;
	use leptoskit_analytics::wire;

	static FIELDS: &[(&str, &str)] = &[("plan", "String")];
	let (url, db) = start().await;
	let mut queue = Queue::new(Limits::default(), vec![]);
	let now = 1_762_646_400_000;
	let id = leptoskit_analytics::schema_id("never_registered", FIELDS);
	queue.remember_schema(id, FIELDS);
	queue.push(
		wire::Entry {
			uuid: wire::uuid_v7(now, [3; 10]),
			distinct_id: "user-1".into(),
			session_id: None,
			context: 0,
			body: wire::Body::TypedEvent {
				name: "never_registered".into(),
				schema_id: id,
				values: vec![wire::Value::Text("pro".into())],
				properties: vec![],
			},
		},
		now,
	);

	let post = |body: Vec<u8>| {
		let url = url.clone();
		async move {
			let response = reqwest::Client::new()
				.post(format!("{url}/_leptoskit/services/analytics/v1/batch"))
				.bearer_auth(TOKEN)
				.body(body)
				.send()
				.await
				.unwrap();
			response.json::<serde_json::Value>().await.unwrap()
		}
	};
	let answer = post(queue.take(now, [0; 10]).unwrap().body).await;
	assert_eq!(answer["refused"][0]["reason"], "unknown_schema");
	queue.settle_with_refusals(&[(0, "unknown_schema")], now, 0.0);
	assert_eq!(queue.len(), 1, "queued again");

	let again = queue.take(now, [0; 10]).unwrap();
	let resent = wire::decode(&again.body, &wire::Limits::default()).unwrap();
	assert!(matches!(resent.entries[0].body, wire::Body::Event { .. }));
	let answer = post(again.body).await;
	assert_eq!(answer["stored"], 1, "{answer}");
	assert_eq!(
		scalar(
			&db,
			"SELECT properties FROM events WHERE name = 'never_registered'"
		)
		.await,
		r#"Text("{\"plan\":\"pro\"}")"#
	);
}
