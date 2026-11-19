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

#[path = "support/engine.rs"]
mod engine;

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

	// Bob's raced enrollment of the same credential is refused, and the
	// row stays alice's.
	let bob = signed_in(BOB);
	let stranger = post(
		&service,
		"register_push_endpoint",
		serde_json::json!({ "credential": credential }),
		Some(&bob),
	);
	let (status, body) = stranger.await;
	assert_eq!(status, 409, "a stranger's enrollment is refused: {body}");

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
	db.migrate(POSTGRES_MIGRATIONS)
		.await
		.expect("migrations apply");
	let last: &str = &POSTGRES_MIGRATIONS.last().expect("migrations").name;
	assert!(last.ends_with("_device_details"), "{last}");

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

	let reverted = db
		.rollback(POSTGRES_MIGRATIONS, 1)
		.await
		.expect("down applies");
	println!("down: reverted {reverted:?}");
	assert_eq!(reverted, [last]);
	let down = columns(db.clone()).await;
	println!("down: {down:?}");
	assert!(down.is_empty(), "{down:?}");

	let applied = db
		.migrate(POSTGRES_MIGRATIONS)
		.await
		.expect("up applies again");
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
