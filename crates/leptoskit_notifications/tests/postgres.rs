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
