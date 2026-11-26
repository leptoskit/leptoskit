//! Person attributes, the registry, and the audiences that walk it, end
//! to end on a real port: the app describes people with attributes and
//! only the app can; the registry fills from what people do — a topic,
//! a preference, an enrollment — and from the migration that backfills
//! it; a send to everyone reaches every registered person exactly once;
//! a condition anchored on attributes walks the registry while one
//! anchored on a topic still walks the topic index; a forgotten person
//! belongs to no audience anymore; and the dry run counts every shape.

#![cfg(not(target_arch = "wasm32"))]

use std::time::Duration;

use leptoskit_db::Db;
use leptoskit_db::Read as _;
use leptoskit_db::Statement;
use leptoskit_notifications::PersonAttributeChanges;
use leptoskit_notifications::SERVICE;
use leptoskit_notifications::SendProgress;
use leptoskit_notifications::SendResult;
use leptoskit_notifications::SendState;
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

/// How long a fanout may take to walk its batches: each step is a job on
/// a test-fast poll, but a loaded machine can be slower.
const SETTLES: Duration = Duration::from_secs(30);

struct Service {
	address: std::net::SocketAddr,
	db: Db,
	jobs: Option<rpc::server::JobsHandle>,
}

/// A fresh, migrated service on a free port, with `open` declaring which
/// topics self-subscription may reach, and its workers on a test-fast
/// poll so a fanout walks its batches promptly.
async fn start(db: Db, open: &str) -> Service {
	let open_topics = leptoskit_notifications::service::config::OpenTopics::from_vars([(
		"LEPTOSKIT_NOTIFICATIONS_OPEN_TOPICS",
		open,
	)])
	.expect("the patterns are usable");
	let callers = ServiceCallers::new(Anonymous)
		.caller(TOKEN, rpc::server::APP_CALLER)
		.forwarded_identity(SERVICE, TOKEN);
	let (router, live) = leptoskit_notifications::service::routes_with_settings(
		db.clone(),
		Some(ORIGIN),
		callers,
		leptoskit_notifications::service::push::Push {
			paused: leptoskit_notifications::service::push::Paused::default(),
			runtime: None,
		},
		open_topics,
	);
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

/// [`start`], with an APNs transport configured over a dead boundary, so
/// enrollment is accepted and the enrollment-registry path can run.
async fn start_with_push(db: Db) -> Service {
	use std::future::Future;
	use std::sync::Arc;

	use leptoskit_notifications::service::config;
	use leptoskit_notifications::service::transport::Boundary;
	use leptoskit_notifications::service::transport::PushRuntime;
	use leptoskit_notifications::service::transport::WireError;
	use leptoskit_notifications::service::transport::WireRequest;
	use leptoskit_notifications::service::transport::WireResponse;

	/// A boundary that answers nothing: enrollment never reaches it.
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

	let open_topics = config::OpenTopics::default();
	let callers = ServiceCallers::new(Anonymous)
		.caller(TOKEN, rpc::server::APP_CALLER)
		.forwarded_identity(SERVICE, TOKEN);
	let (router, live) = leptoskit_notifications::service::routes_with_settings(
		db.clone(),
		Some(ORIGIN),
		callers,
		leptoskit_notifications::service::push::Push {
			paused: leptoskit_notifications::service::push::Paused::default(),
			runtime: Some(Arc::new(
				PushRuntime::build_over(&config, Arc::new(DeadBoundary)).expect("the runtime"),
			)),
		},
		open_topics,
	);
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

async fn service() -> Service {
	start(
		leptoskit_db::testing::sqlite_migrated(leptoskit_notifications::MIGRATIONS).await,
		"news.*",
	)
	.await
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

/// The app sets attributes, checking they land.
async fn set_attributes(
	service: &Service,
	persons: &[Uuid],
	set: Value,
	remove: &[&str],
) -> PersonAttributeChanges {
	let (status, body) = post(
		service,
		"set_person_attributes",
		json!({
			"persons": persons,
			"set": set,
			"remove": remove,
		}),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	serde_json::from_str(&body).unwrap()
}

/// The app sets one attribute for `persons`.
async fn attribute(service: &Service, key: &str, value: &str, persons: &[Uuid]) {
	let changes = set_attributes(service, persons, json!({ key: value }), &[]).await;
	assert_eq!(
		(changes.set, changes.skipped_full),
		(persons.len() as u32, 0),
		"one attribute each"
	);
}

/// The app reads one person's attributes, parsed.
async fn attributes_of(service: &Service, person: Uuid) -> Value {
	let (status, body) = post(
		service,
		"person_attributes",
		json!({ "person": person }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	serde_json::from_str(&body).unwrap()
}

/// Sends to `target` as the app, checking it lands. The category is
/// `news`.
async fn send(service: &Service, key: &str, target: Value) -> SendResult {
	let (status, body) = post(
		service,
		"send",
		json!({
			"idempotency_key": key,
			"spec": { "category": "news", "title": "Hello segments" },
			"target": target,
		}),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	serde_json::from_str(&body).unwrap()
}

/// `send_status` until the send leaves `fanning_out`, or panics.
async fn settled(service: &Service, key: &str) -> SendProgress {
	let deadline = tokio::time::Instant::now() + SETTLES;

	loop {
		let (status, body) = post(
			service,
			"send_status",
			json!({ "idempotency_key": key }),
			Some(TOKEN),
		)
		.await;
		assert_eq!(status, 200, "{body}");
		let progress: SendProgress = serde_json::from_str(&body).unwrap();

		if progress.state != SendState::FanningOut {
			return progress;
		}

		assert!(
			tokio::time::Instant::now() < deadline,
			"the fanout never settled: {progress:?}"
		);
		tokio::time::sleep(Duration::from_millis(50)).await;
	}
}

/// A person's inbox count.
async fn inbox_len(service: &Service, person: Uuid) -> usize {
	let (_, body) = post(
		service,
		"inbox",
		json!({ "page": { "limit": 100 }, "show": "all" }),
		Some(&signed_in(person)),
	)
	.await;
	let page: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
	page["items"].as_array().map_or(0, Vec::len)
}

/// `person` is a BLOB column: a uuid as a SQLite hex literal.
fn uuid_blob(person: Uuid) -> String {
	let mut hex = String::with_capacity(34);
	hex.push_str("x'");
	for byte in person.as_bytes() {
		use std::fmt::Write as _;
		let _ = write!(hex, "{byte:02x}");
	}
	hex.push('\'');
	hex
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

fn count(value: &leptoskit_db::Value) -> usize {
	match value {
		leptoskit_db::Value::Text(text) => text.parse().unwrap_or(0),
		leptoskit_db::Value::Int(int) => usize::try_from(*int).unwrap_or(0),
		other => panic!("not a count: {other:?}"),
	}
}

/// `count` people the app knows, as distinct ids in `group`'s range.
fn group(group: u128, count: usize) -> Vec<Uuid> {
	(0..count)
		.map(move |n| {
			Uuid::from_u128(
				(group << 48)
					| 0x0F70_0000_0000_0000_0000_0000_0000_0000
					| u128::try_from(n).expect("small"),
			)
		})
		.collect()
}

/// The app describes people with attributes in idempotent batches, and
/// reads back what a segment will see. Bad keys, bad values, an empty
/// batch, an oversized batch, and a call that changes nothing are
/// refused.
#[tokio::test(flavor = "multi_thread")]
async fn the_app_sets_and_removes_attributes_in_batches() {
	let service = service().await;
	let audience = group(0, 3);

	let changes = set_attributes(
		&service,
		&audience,
		json!({ "plan": "pro", "country": "de" }),
		&[],
	)
	.await;
	assert_eq!(
		(
			changes.registered,
			changes.set,
			changes.removed,
			changes.skipped_full
		),
		(3, 6, 0, 0),
		"three persons, two new attributes each"
	);

	// The same values again change nothing and count nothing.
	let changes = set_attributes(
		&service,
		&audience,
		json!({ "plan": "pro", "country": "de" }),
		&[],
	)
	.await;
	assert_eq!(
		(changes.registered, changes.set, changes.removed),
		(0, 0, 0),
		"idempotent"
	);

	// A changed value is one write; a removed key is one removal, and a
	// key in both is removed, never set.
	let changes = set_attributes(
		&service,
		&audience[..1],
		json!({ "plan": "enterprise", "country": "at" }),
		&["country", "plan"],
	)
	.await;
	assert_eq!(
		(changes.set, changes.removed),
		(0, 2),
		"remove is the last word for both keys"
	);
	let stored = attributes_of(&service, audience[0]).await;
	assert_eq!(stored, json!({}), "everything removed: {stored}");

	let changes = set_attributes(&service, &audience[1..], json!({ "plan": "free" }), &[]).await;
	assert_eq!(changes.set, 2, "the changed plan for the rest");

	// The read-back is keyed and sorted, per person.
	let stored = attributes_of(&service, audience[1]).await;
	assert_eq!(
		stored,
		json!({ "country": "de", "plan": "free" }),
		"{stored}"
	);

	// A person the service doesn't know answers an empty map.
	let stored = attributes_of(&service, BOB).await;
	assert_eq!(stored, json!({}), "{stored}");

	// Bounds and shapes.
	let too_long_key = {
		let mut set = serde_json::Map::new();
		set.insert("k".repeat(65), json!("v"));
		Value::Object(set)
	};
	for (name, persons, set, remove) in [
		("empty persons", vec![], json!({ "plan": "pro" }), vec![]),
		(
			"too many persons",
			group(9, 1_001),
			json!({ "plan": "pro" }),
			vec![],
		),
		("nothing to do", vec![ALICE], json!({}), vec![]),
		(
			"bad key case",
			vec![ALICE],
			json!({ "Plan": "pro" }),
			vec![],
		),
		(
			"bad key charset",
			vec![ALICE],
			json!({ "a-b": "pro" }),
			vec![],
		),
		("bad key length", vec![ALICE], too_long_key, vec![]),
		(
			"bad value length",
			vec![ALICE],
			json!({ "plan": "v".repeat(257) }),
			vec![],
		),
		("bad remove key", vec![ALICE], json!({}), vec!["NOT A KEY"]),
	] {
		let (status, body) = post(
			&service,
			"set_person_attributes",
			json!({ "persons": persons, "set": set, "remove": remove }),
			Some(TOKEN),
		)
		.await;
		assert_eq!(status, 400, "{name}: {body}");
	}

	// Nothing above the refused calls registered anyone.
	let stored = rows(&service, "SELECT count(*) FROM audience_members").await;
	assert_eq!(count(&stored[0][0]), 3, "only the audience of three");
}

/// A person holds at most fifty attributes; past the bound a new key is
/// skipped and counted per person and key, and the rest of the batch
/// still lands.
#[tokio::test(flavor = "multi_thread")]
async fn attributes_hold_fifty_per_person() {
	let service = service().await;
	let full = group(1, 1);
	let fresh = group(2, 1);

	let fifty: std::collections::BTreeMap<String, String> =
		(0..50).map(|n| (format!("k{n}"), "v".to_owned())).collect();
	let set = serde_json::to_value(&fifty).unwrap();
	let changes = set_attributes(&service, &full, set, &[]).await;
	assert_eq!((changes.set, changes.skipped_full), (50, 0));

	// The fifty-first key is skipped, counted, and never lands; another
	// person in the same batch is unaffected.
	let changes = set_attributes(&service, &full, json!({ "k50": "v", "k51": "v" }), &[]).await;
	assert_eq!(changes.skipped_full, 2, "both new keys hit the bound");

	let changes = set_attributes(&service, &[full[0], fresh[0]], json!({ "k50": "v" }), &[]).await;
	assert_eq!((changes.set, changes.skipped_full), (1, 1), "per person");

	// An existing key still updates at the bound: the bound is on held
	// keys, not on writes.
	let changes = set_attributes(&service, &full, json!({ "k0": "changed" }), &[]).await;
	assert_eq!((changes.set, changes.skipped_full), (1, 0));

	// A removal makes room again.
	let changes = set_attributes(&service, &full, json!({}), &["k0"]).await;
	assert_eq!(changes.removed, 1);
	let changes = set_attributes(&service, &full, json!({ "k50": "v" }), &[]).await;
	assert_eq!((changes.set, changes.skipped_full), (1, 0), "room was made");

	let stored = attributes_of(&service, full[0]).await;
	assert_eq!(
		stored.as_object().map(serde_json::Map::len),
		Some(50),
		"fifty"
	);
}

/// Attributes are the app's own statement about a person: a signed-in
/// person can't set or read anyone's — their own included — and neither
/// can an anonymous caller. Segments must not be forgeable from the
/// client side.
#[tokio::test(flavor = "multi_thread")]
async fn only_the_app_sets_or_reads_attributes() {
	let service = service().await;
	let alice = signed_in(ALICE);

	for (name, token) in [
		("an anonymous caller", None),
		("a person", Some(alice.as_str())),
	] {
		let (status, body) = post(
			&service,
			"set_person_attributes",
			json!({ "persons": [ALICE], "set": { "plan": "pro" }, "remove": [] }),
			token,
		)
		.await;
		assert_eq!(status, 403, "{name} can't set attributes: {body}");

		let (status, body) = post(
			&service,
			"person_attributes",
			json!({ "person": ALICE }),
			token,
		)
		.await;
		assert_eq!(status, 403, "{name} can't read attributes: {body}");

		let (status, body) =
			post(&service, "forget_person", json!({ "person": ALICE }), token).await;
		assert_eq!(status, 403, "{name} can't forget anyone: {body}");
	}

	let stored = rows(&service, "SELECT count(*) FROM person_attributes").await;
	assert_eq!(count(&stored[0][0]), 0, "nothing was written");
	let stored = rows(&service, "SELECT count(*) FROM audience_members").await;
	assert_eq!(count(&stored[0][0]), 0, "nobody was registered");
}

/// The registry fills from what people do: a topic subscription — the
/// app's batch or the person's own — a category preference, and a push
/// enrollment. Being named in a recipients send does not register
/// anyone: an audience is joined, never inherited.
#[tokio::test(flavor = "multi_thread")]
async fn the_registry_fills_from_what_people_do() {
	let service = service().await;

	// A batch subscription registers its holders.
	let (status, body) = post(
		&service,
		"subscribe_to_topic",
		json!({ "topic": "news.registry", "persons": [ALICE] }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let stored = rows(
		&service,
		&format!(
			"SELECT count(*) FROM audience_members WHERE person = {}",
			uuid_blob(ALICE)
		),
	)
	.await;
	assert_eq!(count(&stored[0][0]), 1, "the batch registered alice");

	// A self-subscription registers too.
	let (status, body) = post(
		&service,
		"subscribe",
		json!({ "topic": "news.mine" }),
		Some(&signed_in(BOB)),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let stored = rows(
		&service,
		&format!(
			"SELECT count(*) FROM audience_members WHERE person = {}",
			uuid_blob(BOB)
		),
	)
	.await;
	assert_eq!(
		count(&stored[0][0]),
		1,
		"bob's own subscription registered him"
	);

	// A category preference registers.
	const CAROL: Uuid = Uuid::from_u128(0xCA40_0000_0000_0000_0000_0000_0000_0003);
	let (status, body) = post(
		&service,
		"set_category_preference",
		json!({ "category": "marketing", "opted_out": true }),
		Some(&signed_in(CAROL)),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let stored = rows(
		&service,
		&format!(
			"SELECT count(*) FROM audience_members WHERE person = {}",
			uuid_blob(CAROL)
		),
	)
	.await;
	assert_eq!(count(&stored[0][0]), 1, "a preference registered carol");

	// A recipients send names Dave without registering him.
	const DAVE: Uuid = Uuid::from_u128(0xDA7E_0000_0000_0000_0000_0000_0000_0004);
	let (status, body) = post(
		&service,
		"send",
		json!({
			"idempotency_key": "named-only",
			"spec": { "category": "news", "title": "Just you" },
			"target": { "recipients": [DAVE] },
		}),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let stored = rows(
		&service,
		&format!(
			"SELECT count(*) FROM audience_members WHERE person = {}",
			uuid_blob(DAVE)
		),
	)
	.await;
	assert_eq!(
		count(&stored[0][0]),
		0,
		"being named in a send registers nobody"
	);
	assert_eq!(inbox_len(&service, DAVE).await, 1, "the row itself stands");

	// An enrollment registers, over a configured transport.
	const ERIN: Uuid = Uuid::from_u128(0xE411_0000_0000_0000_0000_0000_0000_0005);
	let push = start_with_push(
		leptoskit_db::testing::sqlite_migrated(leptoskit_notifications::MIGRATIONS).await,
	)
	.await;
	let (status, body) = post(
		&push,
		"register_push_endpoint",
		json!({ "credential": { "apns": { "device_token": "0".repeat(64) } } }),
		Some(&signed_in(ERIN)),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let stored = rows(
		&push,
		&format!(
			"SELECT count(*) FROM audience_members WHERE person = {}",
			uuid_blob(ERIN)
		),
	)
	.await;
	assert_eq!(count(&stored[0][0]), 1, "enrollment registered erin");
}

/// The migration backfills the registry from the tables that already
/// hold persons: topic subscriptions, category preferences, and push
/// endpoints. A recipients-only person stays unregistered.
#[tokio::test(flavor = "multi_thread")]
async fn the_migration_backfills_known_persons() {
	let db =
		leptoskit_db::testing::sqlite_migrated(&leptoskit_notifications::MIGRATIONS[..2]).await;
	let now = leptoskit_db::now().to_rfc3339();
	const CAROL: Uuid = Uuid::from_u128(0xCA40_0000_0000_0000_0000_0000_0000_0003);

	// One person per source table, in SQL the way the tables' own rows
	// would sit: a topic subscription for alice, a preference for bob,
	// an endpoint for carol.
	db.fetch(Statement {
		sql: format!(
			"INSERT INTO topic_subscriptions (id, person, topic, created_at) VALUES \
			 (randomblob(16), {}, 'news.old', '{now}')",
			uuid_blob(ALICE)
		),
		args: Vec::new(),
	})
	.await
	.expect("a subscription");
	db.fetch(Statement {
		sql: format!(
			"INSERT INTO category_preferences (id, person, category, opted_out, created_at) \
			 VALUES (randomblob(16), {}, 'news', 1, '{now}')",
			uuid_blob(BOB)
		),
		args: Vec::new(),
	})
	.await
	.expect("a preference");
	db.fetch(Statement {
		sql: format!(
			"INSERT INTO push_endpoints (id, person, platform, credential, generation, \
			 fingerprint, invalidated_at, created_at) VALUES (randomblob(16), {}, 'apns', '{{}}', \
			 1, '{}', NULL, '{now}')",
			uuid_blob(CAROL),
			"a".repeat(64)
		),
		args: Vec::new(),
	})
	.await
	.expect("an endpoint");

	db.migrate(leptoskit_notifications::MIGRATIONS)
		.await
		.expect("the registry migration applies");

	let stored = db
		.fetch(Statement {
			sql: "SELECT count(*), count(DISTINCT person) FROM audience_members".into(),
			args: Vec::new(),
		})
		.await
		.expect("the registry");
	assert_eq!(
		(count(&stored[0][0]), count(&stored[0][1])),
		(3, 3),
		"alice, bob, and carol, once each"
	);

	for person in [ALICE, BOB, CAROL] {
		let stored = db
			.fetch(Statement {
				sql: format!(
					"SELECT count(*) FROM audience_members WHERE person = {}",
					uuid_blob(person)
				),
				args: Vec::new(),
			})
			.await
			.expect("a person");
		assert_eq!(count(&stored[0][0]), 1, "{person} is registered");
	}

	// Attributes arrive empty: the migration registers persons, it
	// invents nothing about them.
	let stored = db
		.fetch(Statement {
			sql: "SELECT count(*) FROM person_attributes".into(),
			args: Vec::new(),
		})
		.await
		.expect("the attributes");
	assert_eq!(count(&stored[0][0]), 0);
}

/// A send to everyone reaches every registered person exactly once —
/// more than two batches of them — and nobody the registry doesn't
/// hold, whoever else a recipients send reached.
#[tokio::test(flavor = "multi_thread")]
async fn everyone_reaches_the_whole_registry_exactly_once() {
	let service = service().await;
	let audience = group(2, 1_201);

	// Registering the audience is two attribute batches, the way FCM's
	// batch bounds make it.
	for chunk in audience.chunks(1_000) {
		attribute(&service, "plan", "member", chunk).await;
	}

	// Bob is a person the service has delivered to but never registered.
	let (status, body) = post(
		&service,
		"send",
		json!({
			"idempotency_key": "named-bob",
			"spec": { "category": "news", "title": "Just bob" },
			"target": { "recipients": [BOB] },
		}),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");

	let sent = send(&service, "to-everyone", json!("everyone")).await;
	assert_eq!(sent.state, SendState::FanningOut, "{sent:?}");
	assert_eq!((sent.delivered, sent.targeted), (0, 0), "nothing yet");

	let progress = settled(&service, "to-everyone").await;
	assert_eq!(progress.state, SendState::Complete);
	assert_eq!(progress.targeted, 1_201, "{progress:?}");
	assert_eq!(progress.delivered, 1_201);
	assert_eq!(progress.skipped_opted_out, 0);

	let stored = rows(
		&service,
		"SELECT count(*), count(DISTINCT person) FROM notifications WHERE send_key = 'to-everyone'",
	)
	.await;
	assert_eq!(
		(count(&stored[0][0]), count(&stored[0][1])),
		(1_201, 1_201),
		"one row per registered person, nobody twice"
	);
	assert_eq!(inbox_len(&service, audience[0]).await, 1, "the audience");
	assert_eq!(
		inbox_len(&service, BOB).await,
		1,
		"bob keeps his named row and no other"
	);
}

/// A condition anchored on attributes walks the registry: it reaches a
/// person holding no topics at all. A condition anchored on a topic
/// still walks the topic index: the attribute alone doesn't reach
/// anyone the topic doesn't.
#[tokio::test(flavor = "multi_thread")]
async fn conditions_pick_their_walk_by_anchoring() {
	let service = service().await;
	let pro_no_topics = group(3, 2);
	let pro_beta = group(4, 2);
	let free_no_topics = group(5, 2);
	let free_beta = group(6, 2);

	attribute(&service, "plan", "pro", &pro_no_topics).await;
	attribute(&service, "plan", "pro", &pro_beta).await;
	attribute(&service, "plan", "free", &free_no_topics).await;
	attribute(&service, "plan", "free", &free_beta).await;

	for person in pro_beta.iter().chain(&free_beta) {
		let (status, body) = post(
			&service,
			"subscribe_to_topic",
			json!({ "topic": "beta", "persons": [person] }),
			Some(TOKEN),
		)
		.await;
		assert_eq!(status, 200, "{body}");
	}

	// Unanchored: `user.plan == 'pro' && !('beta' in topics)`, a person
	// with no topics included.
	send(
		&service,
		"unanchored",
		json!({ "condition": "user.plan == 'pro' && !('beta' in topics)" }),
	)
	.await;
	let progress = settled(&service, "unanchored").await;
	assert_eq!(
		(progress.targeted, progress.delivered),
		(2, 2),
		"exactly the pro people who aren't in beta: {progress:?}"
	);

	for person in &pro_no_topics {
		assert_eq!(
			inbox_len(&service, *person).await,
			1,
			"{person} holds no topics"
		);
	}
	for person in pro_beta.iter().chain(&free_no_topics).chain(&free_beta) {
		assert_eq!(
			inbox_len(&service, *person).await,
			0,
			"{person} doesn't match"
		);
	}

	// Anchored: the same plan, but through the topic's subscribers.
	send(
		&service,
		"anchored",
		json!({ "condition": "user.plan == 'pro' && 'beta' in topics" }),
	)
	.await;
	let progress = settled(&service, "anchored").await;
	assert_eq!(
		(progress.targeted, progress.delivered),
		(2, 2),
		"{progress:?}"
	);

	// The unanchored send skipped the beta holders — its condition says
	// so — so the anchored one is their first.
	for person in &pro_beta {
		assert_eq!(
			inbox_len(&service, *person).await,
			1,
			"{person} matched the anchored send"
		);
	}
	for person in &pro_no_topics {
		assert_eq!(
			inbox_len(&service, *person).await,
			1,
			"{person}: the topic anchors the walk, and they don't hold it"
		);
	}
	for person in &free_beta {
		assert_eq!(
			inbox_len(&service, *person).await,
			0,
			"{person} isn't a pro"
		);
	}

	// A condition anchored only by attributes but naming a topic it
	// negates still walks the registry, above. One more shape: `in` and
	// `has` in the same condition.
	attribute(&service, "country", "de", &pro_no_topics).await;
	send(
		&service,
		"mixed",
		json!({ "condition": "user.country in ['de', 'at'] && has(user.plan)" }),
	)
	.await;
	settled(&service, "mixed").await;
	for person in &pro_no_topics {
		assert_eq!(
			inbox_len(&service, *person).await,
			2,
			"{person} matches the mixed shape"
		);
	}
}

/// `validate_only` counts every target's audience now and writes
/// nothing — everyone, an unanchored condition, and an anchored one —
/// with the category's opt-outs subtracted.
#[tokio::test(flavor = "multi_thread")]
async fn validate_only_counts_every_target() {
	let service = service().await;
	let audience = group(7, 3);
	attribute(&service, "plan", "pro", &audience).await;

	let (status, _) = post(
		&service,
		"subscribe_to_topic",
		json!({ "topic": "beta", "persons": [audience[0]] }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200);

	// One of the pros opted out of the send's category.
	let (status, body) = post(
		&service,
		"set_category_preference",
		json!({ "category": "news", "opted_out": true }),
		Some(&signed_in(audience[2])),
	)
	.await;
	assert_eq!(status, 200, "{body}");

	for (name, target, expected) in [
		("everyone", json!("everyone"), (3_u32, 2_u32, 1_u32)),
		(
			"an unanchored condition",
			json!({ "condition": "user.plan == 'pro'" }),
			(3, 2, 1),
		),
		(
			"an anchored condition",
			json!({ "condition": "user.plan == 'pro' && 'beta' in topics" }),
			(1, 1, 0),
		),
	] {
		let (status, body) = post(
			&service,
			"send",
			json!({
				"idempotency_key": format!("dry-{name}"),
				"spec": { "category": "news", "title": "Just looking" },
				"target": target,
				"validate_only": true,
			}),
			Some(TOKEN),
		)
		.await;
		assert_eq!(status, 200, "{name}: {body}");
		let result: SendResult = serde_json::from_str(&body).unwrap();
		assert_eq!(
			(result.targeted, result.delivered, result.skipped_opted_out),
			expected,
			"{name}: {result:?}"
		);
		assert_eq!(result.state, SendState::FanningOut, "{name}");
		assert!(!result.duplicate, "{name}");
	}

	for table in ["notifications", "sends"] {
		let stored = rows(&service, &format!("SELECT count(*) FROM {table}")).await;
		assert_eq!(count(&stored[0][0]), 0, "{table} stayed empty");
	}
}

/// Forgetting a person removes them from every future audience —
/// everyone's and a condition's — and takes their attributes, their topic
/// subscriptions, and their inbox rows with them.
#[tokio::test(flavor = "multi_thread")]
async fn a_forgotten_person_leaves_every_audience() {
	let service = service().await;
	attribute(&service, "plan", "pro", &[ALICE]).await;
	attribute(&service, "plan", "free", &[BOB]).await;

	let (status, body) = post(
		&service,
		"subscribe_to_topic",
		json!({ "topic": "news.stays", "persons": [ALICE] }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");

	send(
		&service,
		"before-forgetting",
		json!({ "condition": "user.plan == 'pro'" }),
	)
	.await;
	settled(&service, "before-forgetting").await;
	assert_eq!(inbox_len(&service, ALICE).await, 1, "alice was reached");

	let (status, body) = post(
		&service,
		"forget_person",
		json!({ "person": ALICE }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");

	// Attributes, subscriptions, the registry row, and the inbox row are
	// all gone.
	let stored = attributes_of(&service, ALICE).await;
	assert_eq!(stored, json!({}), "{stored}");
	let stored = rows(
		&service,
		&format!(
			"SELECT count(*) FROM audience_members WHERE person = {}",
			uuid_blob(ALICE)
		),
	)
	.await;
	assert_eq!(count(&stored[0][0]), 0, "no registry row");

	let (status, body) = post(
		&service,
		"my_topics",
		json!({ "page": { "limit": 10 } }),
		Some(&signed_in(ALICE)),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let page: Value = serde_json::from_str(&body).unwrap();
	assert_eq!(page["items"].as_array().map(Vec::len), Some(0), "{page}");

	assert_eq!(inbox_len(&service, ALICE).await, 0, "the inbox is erased");

	// Neither audience reaches her now.
	send(&service, "after-forgetting-everyone", json!("everyone")).await;
	settled(&service, "after-forgetting-everyone").await;
	send(
		&service,
		"after-forgetting-condition",
		json!({ "condition": "user.plan == 'pro' || user.plan == 'enterprise'" }),
	)
	.await;
	settled(&service, "after-forgetting-condition").await;

	assert_eq!(inbox_len(&service, ALICE).await, 0, "nothing new arrived");
	assert_eq!(
		inbox_len(&service, BOB).await,
		1,
		"bob was reached by everyone"
	);

	// Forgetting an unknown person is a no-op that succeeds.
	let (status, body) = post(
		&service,
		"forget_person",
		json!({ "person": group(8, 1)[0] }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");
}

/// Sends to everyone and to a segment are idempotent by key and intent,
/// conflict on a different intent, and answer their stored state.
#[tokio::test(flavor = "multi_thread")]
async fn everyone_and_segment_sends_are_idempotent_by_key_and_intent() {
	let service = service().await;
	attribute(&service, "plan", "pro", &[ALICE, BOB]).await;

	let first = send(&service, "idempotent-everyone", json!("everyone")).await;
	assert!(!first.duplicate);
	let done = settled(&service, "idempotent-everyone").await;
	assert_eq!(done.delivered, 2);

	let again = send(&service, "idempotent-everyone", json!("everyone")).await;
	assert!(again.duplicate, "{again:?}");
	assert_eq!(again.state, SendState::Complete);
	assert_eq!(again.delivered, 2);

	let (status, body) = post(
		&service,
		"send",
		json!({
			"idempotency_key": "idempotent-everyone",
			"spec": { "category": "news", "title": "A different send" },
			"target": "everyone",
		}),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 409, "a changed intent conflicts: {body}");

	let segment = json!({ "condition": "user.plan == 'pro'" });
	let first = send(&service, "idempotent-segment", segment.clone()).await;
	assert!(!first.duplicate);
	settled(&service, "idempotent-segment").await;

	let again = send(&service, "idempotent-segment", segment).await;
	assert!(again.duplicate, "{again:?}");
	assert_eq!(again.delivered, 2);

	let (status, body) = post(
		&service,
		"send",
		json!({
			"idempotency_key": "idempotent-segment",
			"spec": { "category": "news", "title": "Hello segments" },
			"target": { "condition": "user.plan == 'free'" },
		}),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 409, "a changed target conflicts too: {body}");

	let stored = rows(
		&service,
		"SELECT count(*) FROM notifications WHERE send_key = 'idempotent-segment'",
	)
	.await;
	assert_eq!(count(&stored[0][0]), 2, "nothing was written twice");
}
