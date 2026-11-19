//! The races Postgres runs differently: a unique violation aborts the
//! transaction it happens in, so what SQLite could answer by reading again
//! needs the mutation retried whole. On a private, throwaway Postgres —
//! started from local binaries, no credentials, no network — concurrent
//! same-key sends fan out once, concurrent preference settings from two
//! devices land on one row with the last committed setting, an accepted
//! send answers its stored outcome after its expiry, and a zero-fanout
//! send still cancels.
//!
//! Without Postgres binaries the tests are skipped, unless
//! `LEPTOSKIT_REQUIRE_POSTGRES` is set, as the database crate's own tests
//! are.

#![cfg(all(not(target_arch = "wasm32"), feature = "postgres"))]

use std::future::Future;
use std::time::Duration;

use leptoskit_db::Db;
use leptoskit_db::Read as _;
use leptoskit_db::Statement;
use leptoskit_notifications::POSTGRES_MIGRATIONS;
use leptoskit_notifications::SERVICE;
use leptoskit_notifications::SendResult;
use leptoskit_rpc as rpc;
use leptoskit_rpc::server::Anonymous;
use leptoskit_rpc::server::ServiceCallers;
use serde_json::Value;
use serde_json::json;
use uuid::Uuid;

const TOKEN: &str = "app-token";
const ORIGIN: &str = "http://app.test";
const PREFIX: &str = "/_leptoskit/services/notifications";

const ALICE: Uuid = Uuid::from_u128(0xA110_0000_0000_0000_0000_0000_0000_0001);
const BOB: Uuid = Uuid::from_u128(0xB0B0_0000_0000_0000_0000_0000_0000_0002);

#[path = "support/postgres.rs"]
mod postgres;
use postgres::Postgres;

#[cfg(feature = "service")]
#[path = "support/composer.rs"]
mod composer;
#[path = "support/engine.rs"]
mod engine;
#[cfg(feature = "service")]
#[path = "support/local.rs"]
mod local;
#[cfg(feature = "service")]
#[path = "support/opens.rs"]
mod opens;
#[cfg(feature = "service")]
#[path = "support/ownership.rs"]
mod ownership;
#[cfg(feature = "service")]
#[path = "support/sql.rs"]
#[allow(
	dead_code,
	reason = "the SQL spellings are shared with other test files"
)]
mod sql;

struct Service {
	address: std::net::SocketAddr,
	db: Db,
	jobs: Option<rpc::server::JobsHandle>,
}

async fn service(db: Db) -> Service {
	let callers = ServiceCallers::new(Anonymous)
		.caller(TOKEN, rpc::server::APP_CALLER)
		.forwarded_identity(SERVICE, TOKEN);
	let (router, live) =
		leptoskit_notifications::service::routes_with(db.clone(), Some(ORIGIN), callers);
	live.start().await.expect("live starts");
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	tokio::spawn(async move {
		leptoskit_axum::axum::serve(listener, router).await.unwrap();
	});
	let jobs = leptoskit_notifications::service::workers(db.clone())
		.poll(Duration::from_millis(50))
		.start();

	Service {
		address,
		db,
		jobs: Some(jobs),
	}
}

impl Drop for Service {
	fn drop(&mut self) {
		if let Some(jobs) = self.jobs.take() {
			jobs.stop();
		}
	}
}

fn signed_in(person: Uuid) -> String {
	rpc::server::IdentityKey::new(SERVICE, TOKEN)
		.expect("a key")
		.mint(
			person,
			std::time::SystemTime::now(),
			rpc::server::IDENTITY_LIFETIME,
		)
}

async fn post(
	service: &Service,
	operation: &str,
	args: Value,
	token: Option<&str>,
) -> (u16, String) {
	let mut request = reqwest::Client::new()
		.post(format!(
			"http://{}{PREFIX}/rpc/{operation}",
			service.address
		))
		.json(&args);

	if let Some(token) = token {
		request = request.bearer_auth(token);
	}

	let response = request.send().await.unwrap();
	(response.status().as_u16(), response.text().await.unwrap())
}

async fn send(service: &Service, key: &str, spec: Value, recipients: &[Uuid]) -> (u16, String) {
	post(
		service,
		"send",
		json!({
			"idempotency_key": key,
			"spec": spec,
			"target": { "recipients": recipients },
		}),
		Some(TOKEN),
	)
	.await
}

async fn rows(service: &Service, sql: &str) -> Vec<Vec<leptoskit_db::Value>> {
	service
		.db
		.fetch(Statement {
			sql: sql.into(),
			args: Vec::new(),
		})
		.await
		.expect("a read")
}

fn text(value: &leptoskit_db::Value) -> String {
	match value {
		leptoskit_db::Value::Text(text) => text.clone(),
		leptoskit_db::Value::Bool(value) => value.to_string(),
		leptoskit_db::Value::Int(int) => int.to_string(),
		leptoskit_db::Value::Uuid(id) => id.to_string(),
		leptoskit_db::Value::Null => "NULL".into(),
		other => format!("{other:?}"),
	}
}

/// Both devices of one person set the same new category at the same time,
/// wanting opposite things. On Postgres one insert's unique violation
/// aborts its transaction; the mutation runs again, finds the row the
/// other device wrote, and sets it. Neither answer is swallowed, one row
/// exists, and the last setting to commit is the one that holds.
#[tokio::test(flavor = "multi_thread")]
async fn concurrent_preferences_land_on_one_committed_row() {
	let Some(postgres) = Postgres::start() else {
		return;
	};

	let db = Db::connect(&postgres.database())
		.await
		.expect("the database opens");
	db.migrate(POSTGRES_MIGRATIONS)
		.await
		.expect("migrations apply");
	let service = service(db).await;

	let phone = signed_in(ALICE);
	let laptop = signed_in(ALICE);

	let from_the_phone = post(
		&service,
		"set_category_preference",
		json!({ "category": "marketing", "opted_out": true }),
		Some(&phone),
	);
	let from_the_laptop = post(
		&service,
		"set_category_preference",
		json!({ "category": "marketing", "opted_out": false }),
		Some(&laptop),
	);
	let (a, a_body) = from_the_phone.await;
	let (b, b_body) = from_the_laptop.await;

	assert_eq!(a, 200, "the phone's answer: {a_body}");
	assert_eq!(b, 200, "the laptop's answer: {b_body}");

	let stored = rows(
		&service,
		&format!("SELECT opted_out FROM category_preferences WHERE person = '{ALICE}'"),
	)
	.await;
	assert_eq!(stored.len(), 1, "one row, not two: {stored:?}");
	assert!(
		text(&stored[0][0]) == "true" || text(&stored[0][0]) == "false",
		"whichever committed last holds: {stored:?}"
	);

	// Sequentially, the setting is the last one asked for: deterministic.
	let (status, body) = post(
		&service,
		"set_category_preference",
		json!({ "category": "marketing", "opted_out": false }),
		Some(&phone),
	)
	.await;
	assert_eq!(status, 200, "{body}");

	let stored = rows(
		&service,
		&format!("SELECT opted_out FROM category_preferences WHERE person = '{ALICE}'"),
	)
	.await;
	assert_eq!(stored.len(), 1, "{stored:?}");
	assert_eq!(text(&stored[0][0]), "false", "the last committed setting");
}

/// Two sends with one key, in flight together: one of them hits the
/// unique violation on `sends` that aborts a Postgres transaction, runs
/// again, and answers from the store. One fanout, both answered.
#[tokio::test(flavor = "multi_thread")]
async fn concurrent_sends_with_one_key_fan_out_once() {
	let Some(postgres) = Postgres::start() else {
		return;
	};

	let db = Db::connect(&postgres.database())
		.await
		.expect("the database opens");
	db.migrate(POSTGRES_MIGRATIONS)
		.await
		.expect("migrations apply");
	let service = service(db).await;

	let spec = json!({ "category": "news", "title": "Raced" });
	let first = send(&service, "race", spec.clone(), &[ALICE, BOB]);
	let second = send(&service, "race", spec, &[ALICE, BOB]);
	let (a, a_body) = first.await;
	let (b, b_body) = second.await;

	assert_eq!(a, 200, "{a_body}");
	assert_eq!(b, 200, "{b_body}");
	let first: SendResult = serde_json::from_str(&a_body).unwrap();
	let second: SendResult = serde_json::from_str(&b_body).unwrap();
	assert_ne!(
		first.duplicate, second.duplicate,
		"one of each: {first:?} {second:?}"
	);
	assert_eq!(first.delivered, second.delivered, "the same counts");

	let stored = rows(
		&service,
		"SELECT count(*) FROM notifications WHERE send_key = 'race'",
	)
	.await;
	assert_eq!(text(&stored[0][0]), "2", "one row per person, once");
}

/// A topic send walks its audience in batches on Postgres too: 1,201
/// subscribers reach exactly 1,201 rows, once each, and two same-key
/// sends in flight together fan out once between them.
#[tokio::test(flavor = "multi_thread")]
async fn a_topic_send_reaches_its_audience_once_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};

	let db = Db::connect(&postgres.database())
		.await
		.expect("the database opens");
	db.migrate(POSTGRES_MIGRATIONS)
		.await
		.expect("migrations apply");
	let service = service(db).await;

	let audience: Vec<Uuid> = (0..1_201)
		.map(|n| {
			Uuid::from_u128(
				0x0F70_0000_0000_0000_0000_0000_0000_0000 | u128::try_from(n).expect("small"),
			)
		})
		.collect();

	for chunk in audience.chunks(1_000) {
		let (status, body) = post(
			&service,
			"subscribe_to_topic",
			json!({ "topic": "news.pg", "persons": chunk }),
			Some(TOKEN),
		)
		.await;
		assert_eq!(status, 200, "{body}");
	}

	let send = |key: &str| {
		post(
			&service,
			"send",
			json!({
				"idempotency_key": key,
				"spec": { "category": "news", "title": "By topic" },
				"target": { "topic": "news.pg" },
			}),
			Some(TOKEN),
		)
	};
	let first = send("pg-topic");
	let second = send("pg-topic");
	let (a, a_body) = first.await;
	let (b, b_body) = second.await;

	assert_eq!(a, 200, "{a_body}");
	assert_eq!(b, 200, "{b_body}");
	let first: SendResult = serde_json::from_str(&a_body).unwrap();
	let second: SendResult = serde_json::from_str(&b_body).unwrap();
	assert_ne!(
		first.duplicate, second.duplicate,
		"one of each: {first:?} {second:?}"
	);

	// The fanout settles with the whole audience, once each.
	let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
	loop {
		let (status, body) = post(
			&service,
			"send_status",
			json!({ "idempotency_key": "pg-topic" }),
			Some(TOKEN),
		)
		.await;
		assert_eq!(status, 200, "{body}");
		let progress: leptoskit_notifications::SendProgress = serde_json::from_str(&body).unwrap();

		if progress.state == leptoskit_notifications::SendState::Complete {
			assert_eq!(progress.targeted, 1_201, "{progress:?}");
			assert_eq!(progress.delivered, 1_201);

			break;
		}

		assert!(
			tokio::time::Instant::now() < deadline,
			"the fanout never settled: {progress:?}"
		);
		tokio::time::sleep(Duration::from_millis(100)).await;
	}

	let stored = rows(
		&service,
		"SELECT count(*), count(DISTINCT person) FROM notifications WHERE send_key = 'pg-topic'",
	)
	.await;
	assert_eq!(
		(text(&stored[0][0]), text(&stored[0][1])),
		("1201".to_owned(), "1201".to_owned()),
		"one row per person, nobody twice: {stored:?}"
	);
}

/// A send to everyone walks the person registry on Postgres too: 1,201
/// registered persons — set by two attribute batches — reach exactly
/// 1,201 rows, once each, and a same-key resend in flight together fans
/// out once between them.
#[tokio::test(flavor = "multi_thread")]
async fn an_everyone_send_walks_the_registry_once_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};

	let db = Db::connect(&postgres.database())
		.await
		.expect("the database opens");
	db.migrate(POSTGRES_MIGRATIONS)
		.await
		.expect("migrations apply");
	let service = service(db).await;

	let audience: Vec<Uuid> = (0..1_201)
		.map(|n| {
			Uuid::from_u128(
				0x0F70_0000_0000_0000_0000_0000_0000_0000 | u128::try_from(n).expect("small"),
			)
		})
		.collect();

	for chunk in audience.chunks(1_000) {
		let (status, body) = post(
			&service,
			"set_person_attributes",
			json!({ "persons": chunk, "set": { "plan": "member" } }),
			Some(TOKEN),
		)
		.await;
		assert_eq!(status, 200, "{body}");
	}

	let send = |key: &str| {
		post(
			&service,
			"send",
			json!({
				"idempotency_key": key,
				"spec": { "category": "news", "title": "To everyone" },
				"target": "everyone",
			}),
			Some(TOKEN),
		)
	};
	let first = send("pg-everyone");
	let second = send("pg-everyone");
	let (a, a_body) = first.await;
	let (b, b_body) = second.await;

	assert_eq!(a, 200, "{a_body}");
	assert_eq!(b, 200, "{b_body}");
	let first: SendResult = serde_json::from_str(&a_body).unwrap();
	let second: SendResult = serde_json::from_str(&b_body).unwrap();
	assert_ne!(
		first.duplicate, second.duplicate,
		"one of each: {first:?} {second:?}"
	);

	// The fanout settles with the whole registry, once each.
	let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
	loop {
		let (status, body) = post(
			&service,
			"send_status",
			json!({ "idempotency_key": "pg-everyone" }),
			Some(TOKEN),
		)
		.await;
		assert_eq!(status, 200, "{body}");
		let progress: leptoskit_notifications::SendProgress = serde_json::from_str(&body).unwrap();

		if progress.state == leptoskit_notifications::SendState::Complete {
			assert_eq!(progress.targeted, 1_201, "{progress:?}");
			assert_eq!(progress.delivered, 1_201);

			break;
		}

		assert!(
			tokio::time::Instant::now() < deadline,
			"the fanout never settled: {progress:?}"
		);
		tokio::time::sleep(Duration::from_millis(100)).await;
	}

	let stored = rows(
		&service,
		"SELECT count(*), count(DISTINCT person) FROM notifications WHERE send_key = 'pg-everyone'",
	)
	.await;
	assert_eq!(
		(text(&stored[0][0]), text(&stored[0][1])),
		("1201".to_owned(), "1201".to_owned()),
		"one row per registered person, nobody twice: {stored:?}"
	);
}

/// The clock never invalidates an accepted send: after its expiry, the
/// same key with the same intent answers the stored counts, the same key
/// with a different intent conflicts, and a new born-expired send is
/// refused.
#[tokio::test(flavor = "multi_thread")]
async fn a_send_answers_after_its_expiry() {
	let Some(postgres) = Postgres::start() else {
		return;
	};

	let db = Db::connect(&postgres.database())
		.await
		.expect("the database opens");
	db.migrate(POSTGRES_MIGRATIONS)
		.await
		.expect("migrations apply");
	let service = service(db).await;

	let expiring = json!({
		"category": "news",
		"title": "Gone by then",
		"expires_at": (chrono::Utc::now() + chrono::Duration::seconds(1)).to_rfc3339(),
	});
	let (status, body) = send(&service, "fleeting", expiring.clone(), &[ALICE]).await;
	assert_eq!(status, 200, "{body}");

	tokio::time::sleep(Duration::from_millis(1_300)).await;

	let (status, body) = send(&service, "fleeting", expiring, &[ALICE]).await;
	assert_eq!(status, 200, "the stored outcome, not the clock: {body}");
	let again: SendResult = serde_json::from_str(&body).unwrap();
	assert!(again.duplicate, "{body}");
	assert_eq!(again.delivered, 1);

	let (status, body) = send(
		&service,
		"fleeting",
		json!({ "category": "news", "title": "Different" }),
		&[ALICE],
	)
	.await;
	assert_eq!(status, 409, "{body}");

	let (status, body) = send(
		&service,
		"born-expired",
		json!({
			"category": "news",
			"title": "Too late",
			"expires_at": (chrono::Utc::now() - chrono::Duration::seconds(1)).to_rfc3339(),
		}),
		&[ALICE],
	)
	.await;
	assert_eq!(status, 400, "{body}");
}

/// A send every recipient opted out of has no rows, and cancelling it is
/// still cancelling the send.
#[tokio::test(flavor = "multi_thread")]
async fn a_zero_fanout_send_still_cancels() {
	let Some(postgres) = Postgres::start() else {
		return;
	};

	let db = Db::connect(&postgres.database())
		.await
		.expect("the database opens");
	db.migrate(POSTGRES_MIGRATIONS)
		.await
		.expect("migrations apply");
	let service = service(db).await;

	let (status, body) = post(
		&service,
		"set_category_preference",
		json!({ "category": "marketing", "opted_out": true }),
		Some(&signed_in(BOB)),
	)
	.await;
	assert_eq!(status, 200, "{body}");

	let (status, body) = send(
		&service,
		"nobody-sees",
		json!({ "category": "marketing", "title": "A sale" }),
		&[BOB],
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let sent: SendResult = serde_json::from_str(&body).unwrap();
	assert_eq!((sent.delivered, sent.skipped_opted_out), (0, 1));

	let (status, body) = post(
		&service,
		"cancel",
		json!({ "idempotency_key": "nobody-sees" }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	assert_eq!(body, "0");
}

/// Two devices of one person enroll the same credential at the same
/// time: the unique violation on the credential's fingerprint aborts one
/// transaction, and that registration runs again, finding the row the
/// other wrote. One endpoint exists, and both devices hold its id.
///
/// The same credential from two different accounts races the other way:
/// exactly one wins, and the loser is refused rather than reassigned.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn concurrent_enrollments_of_one_credential_land_on_one_endpoint() {
	use leptoskit_notifications::service::config;
	use leptoskit_notifications::service::push;
	use leptoskit_notifications::service::transport::Boundary;
	use leptoskit_notifications::service::transport::PushRuntime;
	use leptoskit_notifications::service::transport::WireError;
	use leptoskit_notifications::service::transport::WireRequest;
	use leptoskit_notifications::service::transport::WireResponse;

	let Some(postgres) = Postgres::start() else {
		return;
	};

	let db = Db::connect(&postgres.database())
		.await
		.expect("the database opens");
	db.migrate(POSTGRES_MIGRATIONS)
		.await
		.expect("migrations apply");
	let service = service(db).await;

	// A runtime with one APNs transport, so enrollment is accepted. The
	// boundary is never called on this path.
	let key_p8 = {
		let rng = ring::rand::SystemRandom::new();
		let document = ring::signature::EcdsaKeyPair::generate_pkcs8(
			&ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING,
			&rng,
		)
		.expect("a key");
		let der = document.as_ref().to_vec();
		format!(
			"-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
			base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &der)
		)
	};
	let config = config::PushConfig {
		apns: Some(config::ApnsConfig {
			key_p8,
			key_id: "KEY".into(),
			team_id: "TEAM".into(),
			topic: "dev.leptoskit.app".into(),
			environment: config::ApnsEnvironment::Sandbox,
		}),
		fcm: None,
		web: None,
	};

	// A second service over the same database, this one carrying an APNs
	// transport so enrollment is accepted: each service's configuration
	// is its own, never the process's.
	let callers = ServiceCallers::new(Anonymous)
		.caller(TOKEN, rpc::server::APP_CALLER)
		.forwarded_identity(SERVICE, TOKEN);
	let (router, live) = leptoskit_notifications::service::routes_with_push(
		service.db.clone(),
		Some(ORIGIN),
		callers,
		push::Push {
			runtime: Some(std::sync::Arc::new(
				PushRuntime::build_over(&config, std::sync::Arc::new(DeadBoundary))
					.expect("the runtime builds"),
			)),
		},
	);
	live.start().await.expect("live starts");
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	tokio::spawn(async move {
		leptoskit_axum::axum::serve(listener, router).await.unwrap();
	});
	let service = Service {
		address,
		db: service.db.clone(),
		jobs: None,
	};

	let token = "0".repeat(64);
	let credential = serde_json::json!({ "apns": { "device_token": token } });
	let alice = signed_in(ALICE);

	let phone = post(
		&service,
		"register_push_endpoint",
		serde_json::json!({ "credential": credential }),
		Some(&alice),
	);
	let laptop = post(
		&service,
		"register_push_endpoint",
		serde_json::json!({ "credential": credential }),
		Some(&alice),
	);
	let (a, a_body) = phone.await;
	let (b, b_body) = laptop.await;

	// Both answer: one did the work, the other found its row — or, if
	// the unique violation landed first, contended and ran again.
	assert_eq!(a, 200, "the phone's answer: {a_body}");
	assert_eq!(b, 200, "the laptop's answer: {b_body}");

	let stored = rows(
		&service,
		&format!("SELECT count(*) FROM push_endpoints WHERE person = '{ALICE}'"),
	)
	.await;
	assert_eq!(
		text(&stored[0][0]),
		"1",
		"one endpoint, not two: {stored:?}"
	);

	// Bob presents the byte-identical credential: the device is in his
	// hands, so it passes to him, and there is still one row for it.
	let bob = signed_in(BOB);
	let next = post(
		&service,
		"register_push_endpoint",
		serde_json::json!({ "credential": credential }),
		Some(&bob),
	);
	let (status, body) = next.await;
	assert_eq!(status, 200, "the next person takes the device over: {body}");
	let stored = rows(&service, "SELECT person FROM push_endpoints").await;
	assert_eq!(
		stored.iter().map(|row| text(&row[0])).collect::<Vec<_>>(),
		vec![BOB.to_string()],
		"one endpoint, bob's"
	);

	// A boundary that answers nothing: enrollment never reaches it, and
	// if a delivery somehow did, its failure is loud.
	struct DeadBoundary;

	impl Boundary for DeadBoundary {
		fn post(
			&self,
			_request: WireRequest,
		) -> std::pin::Pin<Box<dyn Future<Output = Result<WireResponse, WireError>> + Send>>
		{
			Box::pin(async { Err(WireError::Unreachable) })
		}
	}

	let _ = DeadBoundary;
}

/// A migrated database on `postgres` behind a pool of one connection, and
/// the service over it with its workers stopped: the engine checks hand
/// that one connection out in turn, and run their jobs themselves.
async fn one_connection(postgres: &Postgres) -> Service {
	let db = Db::connect_with(
		&postgres.database(),
		leptoskit_db::PoolOptions { max_connections: 1 },
	)
	.await
	.expect("the database opens");
	db.migrate(POSTGRES_MIGRATIONS)
		.await
		.expect("migrations apply");
	let mut service = service(db).await;

	if let Some(jobs) = service.jobs.take() {
		jobs.shutdown().await;
	}

	service
}

/// A step that finds its batch taken by a replay of an earlier step never
/// ends the fanout on Postgres either.
#[tokio::test(flavor = "multi_thread")]
async fn a_lost_batch_never_ends_the_fanout_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	let service = one_connection(&postgres).await;
	engine::a_lost_batch_keeps_the_fanout_going(service.address, &service.db).await;
}

/// A dry run walks its audience with no transaction open on Postgres too.
#[tokio::test(flavor = "multi_thread")]
async fn a_dry_run_holds_up_no_write_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	let service = one_connection(&postgres).await;
	engine::a_dry_run_holds_up_no_write(service.address, &service.db).await;
}

/// A large fanout send's rows are cancelled in batches on Postgres too.
#[tokio::test(flavor = "multi_thread")]
async fn a_large_cancel_is_batched_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	let service = one_connection(&postgres).await;
	engine::a_large_cancel_is_batched(service.address, &service.db).await;
}

/// A test send holds no transaction — and so no connection — while a
/// provider answers: over a pool of one, a write made while its request is
/// in flight finishes before the provider answers it.
#[tokio::test(flavor = "multi_thread")]
async fn a_test_send_holds_up_no_write_on_postgres() {
	use std::sync::Arc;
	use std::sync::atomic::AtomicUsize;
	use std::sync::atomic::Ordering;

	use leptoskit_notifications::service::config;
	use leptoskit_notifications::service::push;
	use leptoskit_notifications::service::transport::Boundary;
	use leptoskit_notifications::service::transport::PushRuntime;
	use leptoskit_notifications::service::transport::WireError;
	use leptoskit_notifications::service::transport::WireRequest;
	use leptoskit_notifications::service::transport::WireResponse;

	/// A provider that holds every request until the test lets it go, and
	/// then takes the message.
	#[derive(Default)]
	struct Held {
		gate: tokio::sync::Notify,
		holding: AtomicUsize,
	}

	impl Boundary for Held {
		fn post(
			&self,
			_request: WireRequest,
		) -> std::pin::Pin<Box<dyn Future<Output = Result<WireResponse, WireError>> + Send + '_>>
		{
			Box::pin(async move {
				let released = self.gate.notified();
				tokio::pin!(released);
				released.as_mut().enable();
				self.holding.fetch_add(1, Ordering::SeqCst);
				released.await;

				Ok(WireResponse {
					status: 200,
					headers: Vec::new(),
					body: Vec::new(),
				})
			})
		}
	}

	let Some(postgres) = Postgres::start() else {
		return;
	};
	let db = Db::connect_with(
		&postgres.database(),
		leptoskit_db::PoolOptions { max_connections: 1 },
	)
	.await
	.expect("the database opens");
	db.migrate(POSTGRES_MIGRATIONS)
		.await
		.expect("migrations apply");

	let key_p8 = {
		let rng = ring::rand::SystemRandom::new();
		let document = ring::signature::EcdsaKeyPair::generate_pkcs8(
			&ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING,
			&rng,
		)
		.expect("a key");
		format!(
			"-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
			base64::Engine::encode(
				&base64::engine::general_purpose::STANDARD,
				document.as_ref()
			)
		)
	};
	let config = config::PushConfig {
		apns: Some(config::ApnsConfig {
			key_p8,
			key_id: "KEY".into(),
			team_id: "TEAM".into(),
			topic: "dev.leptoskit.app".into(),
			environment: config::ApnsEnvironment::Sandbox,
		}),
		fcm: None,
		web: None,
	};
	let provider = Arc::new(Held::default());
	let runtime = PushRuntime::build_over(&config, provider.clone()).expect("the runtime builds");
	let callers = ServiceCallers::new(Anonymous)
		.caller(TOKEN, rpc::server::APP_CALLER)
		.forwarded_identity(SERVICE, TOKEN);
	let (router, live) = leptoskit_notifications::service::routes_with_push(
		db.clone(),
		Some(ORIGIN),
		callers,
		push::Push {
			runtime: Some(Arc::new(runtime)),
		},
	);
	live.start().await.expect("live starts");
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	tokio::spawn(async move {
		leptoskit_axum::axum::serve(listener, router).await.unwrap();
	});
	let service = Service {
		address,
		db,
		jobs: None,
	};

	let (status, body) = post(
		&service,
		"register_push_endpoint",
		json!({ "credential": { "apns": { "device_token": "0".repeat(64) } } }),
		Some(&signed_in(ALICE)),
	)
	.await;
	assert_eq!(status, 200, "{body}");

	let probe = tokio::spawn(async move {
		engine::call(
			address,
			"send_test",
			json!({ "spec": { "category": "news", "title": "Test" }, "person": ALICE }),
		)
		.await
	});

	for _ in 0..400 {
		if provider.holding.load(Ordering::SeqCst) > 0 {
			break;
		}

		tokio::time::sleep(Duration::from_millis(25)).await;
	}
	assert_eq!(
		provider.holding.load(Ordering::SeqCst),
		1,
		"the test send is in flight"
	);

	let (status, body) = tokio::time::timeout(
		Duration::from_secs(10),
		engine::call(
			address,
			"subscribe_to_topic",
			json!({ "topic": "news.meanwhile", "persons": [BOB] }),
		),
	)
	.await
	.expect("the write finishes while the provider still holds the test send");
	assert_eq!(status, 200, "{body}");

	provider.gate.notify_waiters();
	let (status, body) = probe.await.expect("the test send answers");
	assert_eq!(status, 200, "{body}");
	let outcomes: Value = serde_json::from_str(&body).unwrap();
	assert_eq!(outcomes[0]["state"], "accepted", "{outcomes}");
}

/// The device-details migration on Postgres: down drops the three columns
/// and keeps every endpoint, and up brings them back empty.
#[tokio::test(flavor = "multi_thread")]
async fn the_device_details_migration_reverses_and_reapplies_on_postgres() {
	use leptoskit_db::Connection as _;

	let Some(postgres) = Postgres::start() else {
		return;
	};

	let db = Db::connect(&postgres.database())
		.await
		.expect("the database opens");
	// Up to the migration under test, whatever has been added after it.
	let position = POSTGRES_MIGRATIONS
		.iter()
		.position(|migration| migration.name.ends_with("_device_details"))
		.expect("the registry lists device_details");
	let through = &POSTGRES_MIGRATIONS[..=position];
	db.migrate(through).await.expect("migrations apply");
	let last: &str = &through[position].name;

	let columns = |db: Db| {
		async move {
			db.fetch(Statement {
				sql: "SELECT column_name FROM information_schema.columns WHERE table_name = \
				      'push_endpoints' AND column_name IN ('locale', 'time_zone', 'app_version') \
				      ORDER BY column_name"
					.into(),
				args: Vec::new(),
			})
			.await
			.expect("a read")
			.iter()
			.map(|row| text(&row[0]))
			.collect::<Vec<_>>()
		}
	};
	let endpoint = Uuid::from_u128(0xE0E0);
	db.execute(Statement {
		sql: format!(
			"INSERT INTO push_endpoints (id, person, platform, credential, generation, \
			 fingerprint, created_at, locale, time_zone, app_version) VALUES ('{endpoint}', \
			 '{ALICE}', 'apns', 'secret', 1, '{}', now(), 'en-GB', 'Europe/London', '2.1.0')",
			"e".repeat(64)
		),
		args: Vec::new(),
	})
	.await
	.expect("an endpoint with details");
	let up = columns(db.clone()).await;
	println!("up: {up:?}");
	assert_eq!(up, ["app_version", "locale", "time_zone"]);

	let reverted = db.rollback(through, 1).await.expect("down applies");
	println!("down: reverted {reverted:?}");
	assert_eq!(reverted, [last]);
	let down = columns(db.clone()).await;
	println!("down: {down:?}");
	assert!(down.is_empty(), "{down:?}");

	let applied = db.migrate(through).await.expect("up applies again");
	println!("up again: applied {applied:?}");
	assert_eq!(applied, [last]);
	assert_eq!(columns(db.clone()).await, up);

	let row = db
		.fetch(Statement {
			sql: "SELECT credential, locale, time_zone, app_version FROM push_endpoints".into(),
			args: Vec::new(),
		})
		.await
		.expect("a read");
	assert_eq!(
		row[0].iter().map(text).collect::<Vec<_>>(),
		["secret", "NULL", "NULL", "NULL"],
		"the endpoint survived the reversal, its details empty until it re-enrolls"
	);
}

/// A service over `db` with an APNs transport whose boundary answers
/// nothing, so enrollment is accepted and no provider is ever reached.
#[cfg(feature = "service")]
async fn apns_service(db: Db) -> Service {
	use leptoskit_notifications::service::config;
	use leptoskit_notifications::service::push;
	use leptoskit_notifications::service::transport::Boundary;
	use leptoskit_notifications::service::transport::PushRuntime;
	use leptoskit_notifications::service::transport::WireError;
	use leptoskit_notifications::service::transport::WireRequest;
	use leptoskit_notifications::service::transport::WireResponse;

	struct DeadBoundary;

	impl Boundary for DeadBoundary {
		fn post(
			&self,
			_request: WireRequest,
		) -> std::pin::Pin<Box<dyn Future<Output = Result<WireResponse, WireError>> + Send>>
		{
			Box::pin(async { Err(WireError::Unreachable) })
		}
	}

	let key_p8 = {
		let rng = ring::rand::SystemRandom::new();
		let document = ring::signature::EcdsaKeyPair::generate_pkcs8(
			&ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING,
			&rng,
		)
		.expect("a key");
		format!(
			"-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
			base64::Engine::encode(
				&base64::engine::general_purpose::STANDARD,
				document.as_ref()
			)
		)
	};
	let config = config::PushConfig {
		apns: Some(config::ApnsConfig {
			key_p8,
			key_id: "KEY".into(),
			team_id: "TEAM".into(),
			topic: "dev.leptoskit.app".into(),
			environment: config::ApnsEnvironment::Sandbox,
		}),
		fcm: None,
		web: None,
	};
	let callers = ServiceCallers::new(Anonymous)
		.caller(TOKEN, rpc::server::APP_CALLER)
		.forwarded_identity(SERVICE, TOKEN);
	let (router, live) = leptoskit_notifications::service::routes_with_push(
		db.clone(),
		Some(ORIGIN),
		callers,
		push::Push {
			runtime: Some(std::sync::Arc::new(
				PushRuntime::build_over(&config, std::sync::Arc::new(DeadBoundary))
					.expect("the runtime builds"),
			)),
		},
	);
	live.start().await.expect("live starts");
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	tokio::spawn(async move {
		leptoskit_axum::axum::serve(listener, router).await.unwrap();
	});

	Service {
		address,
		db: db.clone(),
		jobs: Some(
			leptoskit_notifications::service::workers(db)
				.poll(Duration::from_millis(50))
				.start(),
		),
	}
}

/// On Postgres, a re-enrollment replaces an endpoint's details without
/// advancing its credential generation, and a condition that asks about
/// devices fans out to the matching person and enqueues a delivery for the
/// matching endpoint alone.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn device_details_are_replaced_and_targeted_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};

	let db = Db::connect(&postgres.database())
		.await
		.expect("the database opens");
	db.migrate(POSTGRES_MIGRATIONS)
		.await
		.expect("migrations apply");
	let service = apns_service(db).await;

	let enroll = |person: Uuid, token: String, device: Value| {
		let service = &service;
		async move {
			let (status, body) = post(
				service,
				"register_push_endpoint",
				json!({ "credential": { "apns": { "device_token": token } }, "device": device }),
				Some(&signed_in(person)),
			)
			.await;
			assert_eq!(status, 200, "{body}");
			serde_json::from_str::<Value>(&body).unwrap()
		}
	};
	let phone = enroll(
		ALICE,
		"1".repeat(64),
		json!({ "locale": "en_gb", "time_zone": "Europe/London", "app_version": "2.0" }),
	)
	.await;
	assert_eq!(phone["device"]["locale"], "en-GB");
	let phone_id: Uuid = phone["id"].as_str().unwrap().parse().unwrap();
	let updated = enroll(
		ALICE,
		"1".repeat(64),
		json!({ "locale": "fr-FR", "time_zone": "Europe/Paris", "app_version": "2.0" }),
	)
	.await;
	assert_eq!(updated["id"], phone["id"]);
	let stored = rows(
		&service,
		&format!(
			"SELECT generation, locale, time_zone FROM push_endpoints WHERE id = '{phone_id}'"
		),
	)
	.await;
	assert_eq!(
		stored[0].iter().map(text).collect::<Vec<_>>(),
		["1", "fr-FR", "Europe/Paris"],
		"replaced, with no rotation"
	);
	enroll(
		BOB,
		"2".repeat(64),
		json!({ "locale": "en-US", "app_version": "2.0" }),
	)
	.await;

	let (status, body) = post(
		&service,
		"send",
		json!({
			"idempotency_key": "french-devices",
			"spec": { "category": "updates", "title": "Bonjour" },
			"target": { "condition": "device.language == 'fr'" },
		}),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");

	let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
	loop {
		let (status, body) = post(
			&service,
			"send_status",
			json!({ "idempotency_key": "french-devices" }),
			Some(TOKEN),
		)
		.await;
		assert_eq!(status, 200, "{body}");
		let progress: Value = serde_json::from_str(&body).unwrap();

		if progress["state"] == "complete" {
			assert_eq!(progress["targeted"], 1, "{progress}");
			break;
		}

		assert!(tokio::time::Instant::now() < deadline, "{progress}");
		tokio::time::sleep(Duration::from_millis(50)).await;
	}

	let delivered = rows(
		&service,
		"SELECT person, endpoint FROM push_deliveries WHERE send_key = 'french-devices'",
	)
	.await;
	assert_eq!(delivered.len(), 1, "{delivered:?}");
	assert_eq!(text(&delivered[0][0]), ALICE.to_string());
	assert_eq!(text(&delivered[0][1]), phone_id.to_string());
}

/// A migrated database on `postgres`, and the ownership checks' service
/// over it: an APNs transport that reaches nothing, and no workers.
#[cfg(feature = "service")]
async fn ownership_service(postgres: &Postgres) -> (std::net::SocketAddr, Db) {
	let db = Db::connect(&postgres.database())
		.await
		.expect("the database opens");
	db.migrate(POSTGRES_MIGRATIONS)
		.await
		.expect("migrations apply");
	let address = ownership::serve(&db).await;

	(address, db)
}

/// [`ownership_service`], over a service that also speaks FCM.
#[cfg(feature = "service")]
async fn ownership_service_with_fcm(postgres: &Postgres) -> (std::net::SocketAddr, Db) {
	let db = Db::connect(&postgres.database())
		.await
		.expect("the database opens");
	db.migrate(POSTGRES_MIGRATIONS)
		.await
		.expect("migrations apply");
	let address = ownership::serve_with_fcm(&db).await;

	(address, db)
}

/// Imported tokens are pushed to, re-imported without change, and become
/// the device's own enrollment on Postgres too.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn imported_endpoints_work_until_the_app_enrolls_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	let (address, db) = ownership_service_with_fcm(&postgres).await;
	ownership::imported_endpoints_work_until_the_app_enrolls(address, &db).await;
}

/// An import's refusals and bounds hold on Postgres, where a refused
/// call's transaction writes nothing either.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn an_import_is_bounded_and_ends_like_any_endpoint_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	let (address, db) = ownership_service_with_fcm(&postgres).await;
	ownership::an_import_is_bounded_and_ends_like_any_endpoint(address, &db).await;
}

/// An import of a platform without a transport is refused on Postgres.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn an_import_needs_the_platforms_transport_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	let (address, db) = ownership_service(&postgres).await;
	ownership::an_import_needs_the_platforms_transport(address, &db).await;
}

/// The APNs environment an endpoint reported is stored and replaced on
/// Postgres too.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn an_endpoint_keeps_the_apns_environment_it_reported_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	let (address, db) = ownership_service_with_fcm(&postgres).await;
	ownership::an_endpoint_keeps_the_apns_environment_it_reported(address, &db).await;
}

/// A device passes to the next person who enrolls on it on Postgres too:
/// the previous owner's row is deleted and the new one inserted under the
/// same unique fingerprint, in one transaction.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn the_next_person_takes_a_device_over_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	let (address, db) = ownership_service(&postgres).await;
	ownership::the_next_person_takes_a_device_over(address, &db).await;
}

/// A server-side revoke is never undone by a launch on Postgres either.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn a_server_side_revoke_sticks_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	let (address, db) = ownership_service(&postgres).await;
	ownership::a_server_side_revoke_sticks(address, &db).await;
}

/// Forgetting a person erases every table's rows about them on Postgres.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn forgetting_erases_everything_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	let (address, db) = ownership_service(&postgres).await;
	ownership::forgetting_erases_everything(address, &db).await;
}

/// A long history is erased in batches on Postgres too.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn forgetting_a_long_history_is_batched_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	let (address, db) = ownership_service(&postgres).await;
	ownership::forgetting_a_long_history_is_batched(address, &db).await;
}

/// A pinned inbox page is bounded on Postgres too.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn a_pinned_page_is_bounded_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	let (address, db) = ownership_service(&postgres).await;
	ownership::a_pinned_page_is_bounded(address, &db).await;
}

/// An attribute call's keys are bounded before they reach Postgres.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn an_attribute_call_names_a_bounded_number_of_keys_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	let (address, db) = ownership_service(&postgres).await;
	ownership::an_attribute_call_names_a_bounded_number_of_keys(address, &db).await;
}

/// A migrated database on `postgres`, and the variant and local-time
/// checks' service over it.
#[cfg(feature = "service")]
async fn local_service(postgres: &Postgres, workers: bool) -> local::Service {
	let db = Db::connect(&postgres.database())
		.await
		.expect("the database opens");
	db.migrate(POSTGRES_MIGRATIONS)
		.await
		.expect("migrations apply");

	local::serve(&db, workers).await
}

/// Each person reads a send in their own language on Postgres too: one row
/// per person, resolved from the batch's attributes and endpoints.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn each_person_reads_their_own_language_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	local::each_person_reads_their_own_language(&local_service(&postgres, true).await).await;
}

/// The endpoint heard from last decides a person's language on Postgres
/// too, through `seen_at`.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn the_most_recently_seen_device_decides_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	local::the_most_recently_seen_device_decides(&local_service(&postgres, true).await).await;
}

/// A topic's fanout and a test send read variants on Postgres too.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn variants_reach_a_topic_and_a_test_send_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	local::variants_reach_a_topic_and_a_test_send(&local_service(&postgres, true).await).await;
}

/// Bad variants and local times are refused by name on Postgres too.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn bad_variants_and_local_times_are_refused_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	local::bad_variants_and_local_times_are_refused(&local_service(&postgres, true).await).await;
}

/// A local-time send arrives zone by zone on Postgres too, with its pushes
/// queued for their instants and a cancel that stops what is still ahead.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn each_person_gets_it_at_their_own_nine_o_clock_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	local::each_person_gets_it_at_their_own_nine_o_clock(&local_service(&postgres, true).await)
		.await;
}

/// A local-time send nobody has seen cancels whole, and one everybody has
/// seen doesn't, on Postgres too.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn a_local_time_send_cancels_like_any_other_at_its_edges_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	local::a_local_time_send_cancels_like_any_other_at_its_edges(
		&local_service(&postgres, true).await,
	)
	.await;
}

/// A local-time fanout wakes once per batch and instant on Postgres too.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn a_fanout_wakes_once_per_batch_and_instant_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	local::a_fanout_wakes_once_per_batch_and_instant(&local_service(&postgres, false).await).await;
}

/// A large local-time send cancelled once part of it is visible stops only
/// what is still ahead on Postgres too, in batches.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn a_large_local_cancel_stops_only_what_is_ahead_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	local::a_large_local_cancel_stops_only_what_is_ahead(&local_service(&postgres, false).await)
		.await;
}

/// A migrated database on `postgres`, and the open-tracking checks'
/// service over it.
#[cfg(feature = "service")]
async fn opens_service(postgres: &Postgres) -> (std::net::SocketAddr, Db) {
	let db = Db::connect(&postgres.database())
		.await
		.expect("the database opens");
	db.migrate(POSTGRES_MIGRATIONS)
		.await
		.expect("migrations apply");
	let address = opens::serve(&db).await;

	(address, db)
}

/// The first open is recorded once and marks the row read on Postgres too.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn the_first_open_is_recorded_and_marks_the_row_read_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	let (address, db) = opens_service(&postgres).await;
	opens::the_first_open_is_recorded_and_marks_the_row_read(address, &db).await;
}

/// Only its owner opens a row on Postgres too.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn only_the_owner_opens_a_row_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	let (address, db) = opens_service(&postgres).await;
	opens::only_the_owner_opens_a_row(address, &db).await;
}

/// A data message has no open on Postgres either.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn a_data_message_has_no_open_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	let (address, db) = opens_service(&postgres).await;
	opens::a_data_message_has_no_open(address, &db).await;
}

/// A send's report counts its rows on Postgres, where the five counts come
/// back as `bigint`.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn a_report_counts_what_became_of_a_sends_rows_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	let (address, db) = opens_service(&postgres).await;
	opens::a_report_counts_what_became_of_a_sends_rows(address, &db).await;
}

/// A report's counts shrink with pruning on Postgres too.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn a_reports_counts_shrink_when_rows_are_pruned_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	let (address, db) = opens_service(&postgres).await;
	opens::a_reports_counts_shrink_when_rows_are_pruned(address, &db).await;
}

/// A label is checked, stored in its `varchar(64)`, and part of the intent
/// on Postgres too.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn a_label_is_checked_stored_and_part_of_the_intent_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	let (address, db) = opens_service(&postgres).await;
	opens::a_label_is_checked_stored_and_part_of_the_intent(address, &db).await;
}

/// The open-tracking migration applies over existing rows on Postgres.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn rows_from_before_the_migration_are_unopened_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	let before = POSTGRES_MIGRATIONS
		.iter()
		.position(|migration| migration.name.ends_with("_endpoint_seen"))
		.expect("the registry lists endpoint_seen")
		+ 1;
	let db = Db::connect(&postgres.database())
		.await
		.expect("the database opens");

	opens::rows_from_before_the_migration_are_unopened(
		&db,
		&POSTGRES_MIGRATIONS[..before],
		POSTGRES_MIGRATIONS,
	)
	.await;
}

/// A migrated database on `postgres`, and the composer checks' service
/// over it: the send history, the audience catalogue, and what stands
/// behind them.
#[cfg(feature = "service")]
async fn composer_service(postgres: &Postgres, workers: bool) -> composer::Service {
	let db = Db::connect(&postgres.database())
		.await
		.expect("the database opens");
	db.migrate(POSTGRES_MIGRATIONS)
		.await
		.expect("migrations apply");

	composer::serve(&db, workers).await
}

/// The list of sends pages newest first on Postgres too.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn the_list_pages_newest_first_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	composer::the_list_pages_newest_first(&composer_service(&postgres, false).await).await;
}

/// Every kind of send is listed with its content on Postgres too.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn each_kind_of_send_is_listed_with_its_content_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	composer::each_kind_of_send_is_listed_with_its_content(
		&composer_service(&postgres, false).await,
	)
	.await;
}

/// A report counts its pushes and its reads by the hour on Postgres, where
/// the hour is cut from a `timestamptz` and the counts come back as `bigint`.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn a_report_describes_the_send_and_counts_its_pushes_and_reads_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	composer::a_report_describes_the_send_and_counts_its_pushes_and_reads(
		&composer_service(&postgres, true).await,
	)
	.await;
}

/// Only the app reads the history on Postgres too.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn only_the_app_reads_the_history_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	composer::only_the_app_reads_the_history(&composer_service(&postgres, false).await).await;
}

/// A recipients send's retry still answers correctly on Postgres, now that
/// its spec is stored.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn a_retried_recipients_send_still_answers_correctly_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	composer::a_retried_recipients_send_still_answers_correctly(
		&composer_service(&postgres, false).await,
	)
	.await;
}

/// A pruned send leaves the list on Postgres too.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn a_pruned_send_leaves_the_list_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	composer::a_pruned_send_leaves_the_list(&composer_service(&postgres, false).await).await;
}

/// The overview counts what the service knows on Postgres too.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn the_overview_counts_what_the_service_knows_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	composer::the_overview_counts_what_the_service_knows(&composer_service(&postgres, false).await)
		.await;
}

/// Topics are listed by the recursive walk on Postgres, where a prefix is
/// matched under the database's own collation.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn topics_are_listed_with_their_subscribers_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	composer::topics_are_listed_with_their_subscribers(&composer_service(&postgres, false).await)
		.await;
}

/// Attribute keys and values are listed on Postgres too.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn attributes_are_listed_with_their_values_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	composer::attributes_are_listed_with_their_values(&composer_service(&postgres, false).await)
		.await;
}

/// Categories are listed from both their sources on Postgres too.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn categories_are_listed_with_opt_outs_and_last_send_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	composer::categories_are_listed_with_opt_outs_and_last_send(
		&composer_service(&postgres, false).await,
	)
	.await;
}

/// An estimate is exact below its cap and says so above it on Postgres too.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn an_estimate_is_exact_below_its_cap_and_says_so_above_it_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	composer::an_estimate_is_exact_below_its_cap_and_says_so_above_it(
		&composer_service(&postgres, false).await,
	)
	.await;
}

/// A person's endpoints are listed without credentials on Postgres too.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn a_persons_endpoints_are_listed_without_credentials_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	composer::a_persons_endpoints_are_listed_without_credentials(
		&composer_service(&postgres, false).await,
	)
	.await;
}

/// Only the app reads the catalogue on Postgres too.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn only_the_app_reads_the_catalogue_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	composer::only_the_app_reads_the_catalogue(&composer_service(&postgres, false).await).await;
}

/// The send-history migration applies over existing sends on Postgres,
/// and leaves them out of the list.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn sends_from_before_the_history_are_not_listed_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	let before = POSTGRES_MIGRATIONS
		.iter()
		.position(|migration| migration.name.ends_with("_open_tracking"))
		.expect("the registry lists open_tracking")
		+ 1;
	let db = Db::connect(&postgres.database())
		.await
		.expect("the database opens");

	composer::sends_from_before_the_history_are_not_listed(
		&db,
		&POSTGRES_MIGRATIONS[..before],
		POSTGRES_MIGRATIONS,
	)
	.await;
}

/// A draft saves incomplete and is revised under its revision on Postgres too.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn a_draft_is_saved_incomplete_and_revised_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	composer::a_draft_is_saved_incomplete_and_revised(&composer_service(&postgres, false).await)
		.await;
}

/// A launch makes one send however often it is pressed on Postgres too.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn a_launch_makes_one_send_however_often_it_is_pressed_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	composer::a_launch_makes_one_send_however_often_it_is_pressed(
		&composer_service(&postgres, false).await,
	)
	.await;
}

/// A launch checks what a send checks on Postgres too.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn a_launch_checks_what_a_send_checks_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	composer::a_launch_checks_what_a_send_checks(&composer_service(&postgres, false).await).await;
}

/// A past schedule is refused and a future one is kept on Postgres too.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn a_past_schedule_is_refused_and_a_future_one_is_kept_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	composer::a_past_schedule_is_refused_and_a_future_one_is_kept(
		&composer_service(&postgres, false).await,
	)
	.await;
}

/// A cancel stops a scheduled campaign and not a seen one on Postgres too.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn a_cancel_stops_a_scheduled_campaign_and_not_a_seen_one_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	composer::a_cancel_stops_a_scheduled_campaign_and_not_a_seen_one(
		&composer_service(&postgres, false).await,
	)
	.await;
}

/// A duplicate is a new draft without the schedule on Postgres too.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn a_duplicate_is_a_new_draft_without_the_schedule_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	composer::a_duplicate_is_a_new_draft_without_the_schedule(
		&composer_service(&postgres, false).await,
	)
	.await;
}

/// A test writes no inbox row and records an event on Postgres too.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn a_test_writes_no_inbox_row_and_records_an_event_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	composer::a_test_writes_no_inbox_row_and_records_an_event(
		&composer_service(&postgres, false).await,
	)
	.await;
}

/// An archived campaign is put away on Postgres too.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn an_archived_campaign_is_put_away_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	composer::an_archived_campaign_is_put_away(&composer_service(&postgres, false).await).await;
}

/// Only the app touches campaigns on Postgres too.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn only_the_app_touches_campaigns_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	composer::only_the_app_touches_campaigns(&composer_service(&postgres, false).await).await;
}

/// Campaigns are bounded on Postgres too.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn campaigns_are_bounded_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	composer::campaigns_are_bounded(&composer_service(&postgres, false).await).await;
}

/// Eight launches of one revision at once make one send on Postgres, where
/// the losers' claims wait on the winner's row and then find it launched.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn racing_launches_make_one_send_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	composer::racing_launches_make_one_send(&composer_service(&postgres, false).await).await;
}

/// Eight saves from one revision at once keep one on Postgres, where each
/// loser's update waits on the winner's row and then matches nothing.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "service")]
async fn racing_saves_keep_one_on_postgres() {
	let Some(postgres) = Postgres::start() else {
		return;
	};
	composer::racing_saves_keep_one(&composer_service(&postgres, false).await).await;
}
