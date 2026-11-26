//! Checks of who owns an endpoint, what an erasure leaves, and the bounds
//! on what one call can make the service read, which both
//! databases run against a service with an APNs transport that reaches no
//! provider and with its workers stopped: a queued delivery stays queued
//! until the check itself moves it, and a background job runs when the
//! check drains it.

use std::future::Future;
use std::time::Duration;

use leptoskit_db::Db;
use leptoskit_db::Read as _;
use leptoskit_db::Statement;
use leptoskit_notifications::SERVICE;
use leptoskit_notifications::service::config;
use leptoskit_notifications::service::push;
use leptoskit_notifications::service::transport::Boundary;
use leptoskit_notifications::service::transport::PushRuntime;
use leptoskit_notifications::service::transport::WireError;
use leptoskit_notifications::service::transport::WireRequest;
use leptoskit_notifications::service::transport::WireResponse;
use leptoskit_rpc as rpc;
use leptoskit_rpc::server::Anonymous;
use leptoskit_rpc::server::ServiceCallers;
use serde_json::Value;
use serde_json::json;
use uuid::Uuid;

use super::sql::Sql;

const TOKEN: &str = "app-token";
const ORIGIN: &str = "http://app.test";
const PREFIX: &str = "/_leptoskit/services/notifications";

const ALICE: Uuid = Uuid::from_u128(0xA110_0000_0000_0000_0000_0000_0000_00E1);
const BOB: Uuid = Uuid::from_u128(0xB0B0_0000_0000_0000_0000_0000_0000_00E2);

/// The words a renewal of a gone enrollment is answered with.
const GONE: &str = "this enrollment is gone";

/// A provider boundary nothing gets through: enrollment needs a transport,
/// and nothing here should reach one.
struct DeadBoundary;

impl Boundary for DeadBoundary {
	fn post(
		&self,
		_request: WireRequest,
	) -> std::pin::Pin<Box<dyn Future<Output = Result<WireResponse, WireError>> + Send>> {
		Box::pin(async { Err(WireError::Unreachable) })
	}
}

/// Serves the notifications routes over `db`, already migrated, with an
/// APNs transport and no workers, and answers where.
pub async fn serve(db: &Db) -> std::net::SocketAddr {
	serve_over(db, None).await
}

/// [`serve`], with an FCM transport as well: what an import of Android
/// tokens needs. The service account is a made-up one over a test-only
/// key, and the boundary still reaches nothing.
pub async fn serve_with_fcm(db: &Db) -> std::net::SocketAddr {
	serve_over(
		db,
		Some(config::FcmConfig {
			service_account_json: json!({
				"project_id": "leptoskit-push",
				"private_key": include_str!("../fixtures/test-only-rsa-key.pem"),
				"client_email": "push@leptoskit-push.iam.gserviceaccount.com",
				"token_uri": "https://oauth2.googleapis.com/token",
			})
			.to_string(),
		}),
	)
	.await
}

async fn serve_over(db: &Db, fcm: Option<config::FcmConfig>) -> std::net::SocketAddr {
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
		fcm,
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
			paused: push::Paused::default(),
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

	address
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

/// Calls `operation` as `person`, or as the app with `None`.
async fn call(
	address: std::net::SocketAddr,
	operation: &str,
	args: Value,
	person: Option<Uuid>,
) -> (u16, String) {
	let token = person.map_or_else(|| TOKEN.to_owned(), signed_in);
	let response = reqwest::Client::new()
		.post(format!("http://{address}{PREFIX}/rpc/{operation}"))
		.bearer_auth(token)
		.json(&args)
		.timeout(Duration::from_secs(60))
		.send()
		.await
		.unwrap();
	(response.status().as_u16(), response.text().await.unwrap())
}

/// Calls `operation` and answers its JSON, which must be a success.
async fn ok(
	address: std::net::SocketAddr,
	operation: &str,
	args: Value,
	person: Option<Uuid>,
) -> Value {
	let (status, body) = call(address, operation, args, person).await;
	assert_eq!(status, 200, "{operation}: {body}");
	serde_json::from_str(&body).unwrap()
}

/// An APNs credential, distinct per `seed`.
fn apns(seed: u8) -> Value {
	json!({ "apns": { "device_token": format!("{seed:02x}").repeat(32) } })
}

/// `person` enrolls `credential` from a gesture: no enrollment renewed.
async fn enroll(address: std::net::SocketAddr, person: Uuid, credential: &Value) -> Uuid {
	let summary = ok(
		address,
		"register_push_endpoint",
		json!({ "credential": credential }),
		Some(person),
	)
	.await;
	summary["id"].as_str().unwrap().parse().unwrap()
}

/// `person`'s launch renews `endpoint` with `credential`.
async fn renew(
	address: std::net::SocketAddr,
	person: Uuid,
	credential: &Value,
	endpoint: Uuid,
) -> (u16, String) {
	call(
		address,
		"register_push_endpoint",
		json!({ "credential": credential, "renews": endpoint }),
		Some(person),
	)
	.await
}

async fn endpoints_of(address: std::net::SocketAddr, person: Uuid) -> Vec<Uuid> {
	let page = ok(
		address,
		"push_endpoints",
		json!({ "page": { "limit": 20 } }),
		Some(person),
	)
	.await;
	page["items"]
		.as_array()
		.unwrap()
		.iter()
		.map(|endpoint| endpoint["id"].as_str().unwrap().parse().unwrap())
		.collect()
}

async fn standing(address: std::net::SocketAddr, person: Uuid, endpoint: Uuid) -> bool {
	ok(
		address,
		"push_endpoint_standing",
		json!({ "endpoint": endpoint }),
		Some(person),
	)
	.await
	.as_bool()
	.unwrap()
}

async fn send_to(address: std::net::SocketAddr, key: &str, recipients: &[Uuid]) {
	ok(
		address,
		"send",
		json!({
			"idempotency_key": key,
			"spec": { "category": "orders", "title": "Shipped", "body": "On its way." },
			"target": { "recipients": recipients },
		}),
		None,
	)
	.await;
}

async fn rows(db: &Db, sql: String) -> Vec<Vec<leptoskit_db::Value>> {
	db.fetch(Statement {
		sql,
		args: Vec::new(),
	})
	.await
	.expect("a read")
}

/// How many rows of `table` name `person`.
async fn held(db: &Db, table: &str, person: Uuid) -> i64 {
	let sql = Sql(db.dialect());
	let rows = rows(
		db,
		format!(
			"SELECT count(*) FROM {table} WHERE person = {}",
			sql.uuid(person)
		),
	)
	.await;

	match rows[0][0] {
		leptoskit_db::Value::Int(count) => count,
		ref other => panic!("not a count: {other:?}"),
	}
}

/// Every table that holds something about a person, and how many rows of
/// each name `person`.
async fn everything_about(db: &Db, person: Uuid) -> Vec<(&'static str, i64)> {
	let mut held_in = Vec::new();

	for table in [
		"notifications",
		"push_endpoints",
		"push_deliveries",
		"category_preferences",
		"topic_subscriptions",
		"person_attributes",
		"audience_members",
		"wakes",
	] {
		held_in.push((table, held(db, table, person).await));
	}

	held_in
}

/// The state and reason of each of `person`'s deliveries.
async fn deliveries_of(db: &Db, person: Uuid) -> Vec<(String, String)> {
	let sql = Sql(db.dialect());
	rows(
		db,
		format!(
			"SELECT state, reason FROM push_deliveries WHERE person = {}",
			sql.uuid(person)
		),
	)
	.await
	.into_iter()
	.map(|row| {
		let text = |value: &leptoskit_db::Value| {
			match value {
				leptoskit_db::Value::Text(text) => text.clone(),
				_ => String::new(),
			}
		};
		(text(&row[0]), text(&row[1]))
	})
	.collect()
}

/// A device whose person never signed out passes to the next person who
/// enrolls on it: presenting the byte-identical credential takes the
/// endpoint over. The previous person's endpoint is revoked — gone from
/// their list, its queued delivery skipped — the new person holds a fresh
/// one, and the previous person's next launch, renewing the enrollment it
/// saved, hears it is gone and takes nothing back.
#[expect(
	clippy::cognitive_complexity,
	reason = "one scenario told in order: each step stands on the last"
)]
pub async fn the_next_person_takes_a_device_over(address: std::net::SocketAddr, db: &Db) {
	let device = apns(0xA1);
	let alices = enroll(address, ALICE, &device).await;
	send_to(address, "before-the-handover", &[ALICE]).await;
	assert_eq!(
		deliveries_of(db, ALICE).await,
		vec![("enqueued".to_owned(), String::new())],
		"a push to alice's device is queued"
	);

	// Alice's session ended without a revoke. Bob signs in on the same
	// device and turns notifications on: the same token.
	let (status, body) = call(
		address,
		"register_push_endpoint",
		json!({ "credential": device }),
		Some(BOB),
	)
	.await;
	assert_eq!(
		status, 200,
		"the next person on a device can enroll on it: {body}"
	);
	let bobs: Uuid = serde_json::from_str::<Value>(&body).unwrap()["id"]
		.as_str()
		.unwrap()
		.parse()
		.unwrap();
	assert_ne!(bobs, alices, "a fresh endpoint, never alice's handed over");
	assert_eq!(endpoints_of(address, BOB).await, vec![bobs]);
	assert!(
		endpoints_of(address, ALICE).await.is_empty(),
		"alice's endpoint is revoked"
	);
	assert_eq!(
		deliveries_of(db, ALICE).await,
		vec![("skipped".to_owned(), "endpoint_revoked".to_owned())],
		"what was queued for alice never reaches the device bob now holds"
	);
	assert!(!standing(address, ALICE, alices).await);
	assert!(standing(address, BOB, bobs).await);
	assert!(
		!standing(address, ALICE, bobs).await,
		"bob's endpoint is nobody else's to ask about"
	);

	// A send to both reaches the device once, as bob's.
	send_to(address, "after-the-handover", &[ALICE, BOB]).await;
	assert_eq!(
		deliveries_of(db, BOB).await,
		vec![("enqueued".to_owned(), String::new())]
	);
	assert_eq!(deliveries_of(db, ALICE).await.len(), 1, "nothing new");

	// Alice's app launches again on that device with the enrollment it
	// saved. A launch renews; it never takes a device back.
	let (status, body) = renew(address, ALICE, &device, alices).await;
	assert_eq!(status, 404, "{body}");
	assert!(body.contains(GONE), "{body}");
	assert_eq!(endpoints_of(address, BOB).await, vec![bobs], "bob keeps it");
	assert!(
		endpoints_of(address, ALICE).await.is_empty(),
		"and alice has none"
	);

	// Her own gesture does: she holds the device and says so.
	let again = enroll(address, ALICE, &device).await;
	assert_ne!(again, alices);
	assert!(
		endpoints_of(address, BOB).await.is_empty(),
		"the device left bob with her gesture"
	);
	assert_eq!(
		deliveries_of(db, BOB).await,
		vec![("skipped".to_owned(), "endpoint_revoked".to_owned())]
	);
}

/// A revoke made on the server sticks: the device's next launch renews the
/// enrollment it saved, hears that it is gone, and nothing is registered —
/// whether the person revoked it from another device or the app's server
/// revoked everything they hold. Only a gesture's enrollment makes a new
/// one. A renewal of an enrollment that stands still works, and brings its
/// details up to date.
#[expect(
	clippy::cognitive_complexity,
	reason = "one scenario told in order: each step stands on the last"
)]
pub async fn a_server_side_revoke_sticks(address: std::net::SocketAddr, db: &Db) {
	let phone = apns(0xB1);
	let tablet = apns(0xB2);
	let on_phone = enroll(address, ALICE, &phone).await;
	let on_tablet = enroll(address, ALICE, &tablet).await;

	// A launch with a standing enrollment renews it in place.
	let (status, body) = call(
		address,
		"register_push_endpoint",
		json!({
			"credential": phone,
			"device": { "app_version": "2.0.0" },
			"renews": on_phone,
		}),
		Some(ALICE),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let renewed: Value = serde_json::from_str(&body).unwrap();
	assert_eq!(renewed["id"], on_phone.to_string());
	assert_eq!(renewed["device"]["app_version"], "2.0.0");

	// From the tablet's device list, alice revokes the phone.
	ok(
		address,
		"revoke_push_endpoint",
		json!({ "endpoint": on_phone }),
		Some(ALICE),
	)
	.await;

	// The phone launches. Whether or not anything about it changed, the
	// renewal registers nothing.
	for device in [json!({}), json!({ "app_version": "2.1.0" })] {
		let (status, body) = call(
			address,
			"register_push_endpoint",
			json!({ "credential": phone, "device": device, "renews": on_phone }),
			Some(ALICE),
		)
		.await;
		assert_eq!(
			status, 404,
			"a revoked enrollment is not renewed into a new one: {body}"
		);
		assert!(body.contains(GONE), "{body}");
	}

	assert_eq!(
		endpoints_of(address, ALICE).await,
		vec![on_tablet],
		"the phone stays revoked"
	);
	assert!(
		!standing(address, ALICE, on_phone).await,
		"which is what an unchanged launch asks, in one read"
	);
	assert!(standing(address, ALICE, on_tablet).await);

	// A rotated token under a revoked enrollment registers nothing either.
	let (status, body) = renew(address, ALICE, &apns(0xB3), on_phone).await;
	assert_eq!(status, 404, "{body}");
	assert_eq!(held(db, "push_endpoints", ALICE).await, 1);

	// Only the app revokes for someone else.
	for caller in [ALICE, BOB] {
		let (status, body) = call(
			address,
			"revoke_person_endpoints",
			json!({ "person": ALICE }),
			Some(caller),
		)
		.await;
		assert_eq!(status, 403, "{body}");
	}

	assert_eq!(held(db, "push_endpoints", ALICE).await, 1);

	// The app's server ends alice's sessions and revokes what she holds.
	send_to(address, "queued-for-the-tablet", &[ALICE]).await;
	let revoked = ok(
		address,
		"revoke_person_endpoints",
		json!({ "person": ALICE }),
		None,
	)
	.await;
	assert_eq!(revoked, json!(1));
	assert_eq!(held(db, "push_endpoints", ALICE).await, 0);
	assert_eq!(
		deliveries_of(db, ALICE).await,
		vec![("skipped".to_owned(), "endpoint_revoked".to_owned())]
	);
	assert_eq!(
		ok(
			address,
			"revoke_person_endpoints",
			json!({ "person": ALICE }),
			None,
		)
		.await,
		json!(0),
		"asked again: nothing left"
	);

	let (status, body) = renew(address, ALICE, &tablet, on_tablet).await;
	assert_eq!(status, 404, "{body}");
	assert!(body.contains(GONE), "{body}");
	assert_eq!(held(db, "push_endpoints", ALICE).await, 0);

	// Her own gesture enrolls again, under a fresh id.
	let fresh = enroll(address, ALICE, &tablet).await;
	assert_ne!(fresh, on_tablet);
	assert!(standing(address, ALICE, fresh).await);
}

/// Forgetting a person erases everything the service holds about them —
/// inbox rows, endpoints with their credentials, deliveries, preferences,
/// topic subscriptions, attributes, the registry row, the wake row — and
/// nothing of anyone else's. It stays erased: their device's next launch
/// registers nothing, and no audience reaches them.
#[expect(
	clippy::cognitive_complexity,
	reason = "one scenario told in order: each step stands on the last"
)]
pub async fn forgetting_erases_everything(address: std::net::SocketAddr, db: &Db) {
	let device = apns(0xC1);

	for (person, credential) in [(ALICE, &device), (BOB, &apns(0xC2))] {
		enroll(address, person, credential).await;
		ok(
			address,
			"set_category_preference",
			json!({ "category": "marketing", "opted_out": true }),
			Some(person),
		)
		.await;
	}

	let alices = endpoints_of(address, ALICE).await[0];
	ok(
		address,
		"set_person_attributes",
		json!({ "persons": [ALICE, BOB], "set": { "plan": "pro" } }),
		None,
	)
	.await;
	ok(
		address,
		"subscribe_to_topic",
		json!({ "topic": "news.product", "persons": [ALICE, BOB] }),
		None,
	)
	.await;

	for key in ["first", "second", "third"] {
		send_to(address, key, &[ALICE, BOB]).await;
	}

	// The wake a send leaves for each person it reached.
	super::engine::drain(db).await;

	let before = everything_about(db, ALICE).await;
	assert_eq!(
		before,
		vec![
			("notifications", 3),
			("push_endpoints", 1),
			("push_deliveries", 3),
			("category_preferences", 1),
			("topic_subscriptions", 1),
			("person_attributes", 1),
			("audience_members", 1),
			("wakes", 1),
		],
		"alice is held in every table"
	);
	let bobs = everything_about(db, BOB).await;
	assert_eq!(bobs, before, "and so is bob");

	// Only the app forgets.
	let (status, body) = call(
		address,
		"forget_person",
		json!({ "person": ALICE }),
		Some(ALICE),
	)
	.await;
	assert_eq!(status, 403, "{body}");

	ok(address, "forget_person", json!({ "person": ALICE }), None).await;

	let after = everything_about(db, ALICE).await;
	assert!(
		after.iter().all(|(_, count)| *count == 0),
		"forgetting a person erases everything about them: {after:?}"
	);
	assert_eq!(
		everything_about(db, BOB).await,
		bobs,
		"and nothing about anyone else"
	);

	// Her inbox answers nothing, and her device's next launch — which
	// renews what it saved — registers nothing and re-registers nobody.
	let inbox = ok(
		address,
		"inbox",
		json!({ "page": { "limit": 10 } }),
		Some(ALICE),
	)
	.await;
	assert_eq!(inbox["items"], json!([]));
	let (status, body) = renew(address, ALICE, &device, alices).await;
	assert_eq!(status, 404, "{body}");
	assert!(body.contains(GONE), "{body}");
	assert!(
		everything_about(db, ALICE)
			.await
			.iter()
			.all(|(_, count)| *count == 0),
		"a launch brings nothing back"
	);

	// No audience reaches her: not everyone, not her old topic, not her
	// old segment.
	for (key, target) in [
		("to-everyone", json!("everyone")),
		("to-the-topic", json!({ "topic": "news.product" })),
		(
			"to-the-segment",
			json!({ "condition": "user.plan == 'pro'" }),
		),
	] {
		ok(
			address,
			"send",
			json!({
				"idempotency_key": key,
				"spec": { "category": "orders", "title": "Hello", "body": "There." },
				"target": target,
			}),
			None,
		)
		.await;
	}

	super::engine::drain(db).await;
	assert_eq!(held(db, "notifications", ALICE).await, 0);
	assert_eq!(held(db, "notifications", BOB).await, 6, "bob got all three");

	// A launch that renews a standing enrollment registers nobody either.
	// Bob was forgotten by a version that left his endpoint behind: his
	// registry row is gone, his endpoint is not. His next launch keeps the
	// endpoint and stays out of every audience.
	let bobs_endpoint = endpoints_of(address, BOB).await[0];
	rows(
		db,
		format!(
			"DELETE FROM audience_members WHERE person = {}",
			Sql(db.dialect()).uuid(BOB)
		),
	)
	.await;
	let (status, body) = renew(address, BOB, &apns(0xC2), bobs_endpoint).await;
	assert_eq!(status, 200, "{body}");
	assert_eq!(
		held(db, "audience_members", BOB).await,
		0,
		"renewing an enrollment is not an act that makes a person known again"
	);

	// Idempotent: again, and for someone the service never knew.
	ok(address, "forget_person", json!({ "person": ALICE }), None).await;
	ok(
		address,
		"forget_person",
		json!({ "person": Uuid::from_u128(0xFEED) }),
		None,
	)
	.await;
}

/// Gives `person` a history of `count` visible inbox rows, each with one
/// accepted delivery to `endpoint`, with plain SQL: that many sends would
/// say the same thing slowly. The row ids are the smallest there are, so
/// they sort before anything the service writes.
async fn seed_history(db: &Db, person: Uuid, endpoint: Uuid, count: u128) {
	let sql = Sql(db.dialect());
	let now = sql.timestamp(&leptoskit_db::now().to_rfc3339());
	let history: Vec<Uuid> = (1..=count).map(Uuid::from_u128).collect();

	for chunk in history.chunks(250) {
		let notifications: Vec<String> = chunk
			.iter()
			.map(|id| {
				format!(
					"({}, 'seeded-{id}', {}, 'orders', 'Shipped', 'On its way.', '{{}}', {now}, \
					 {now}, 'notification')",
					sql.uuid(*id),
					sql.uuid(person),
				)
			})
			.collect();
		rows(
			db,
			format!(
				"INSERT INTO notifications (id, send_key, person, category, title, body, data, \
				 visible_at, created_at, kind) VALUES {}",
				notifications.join(", ")
			),
		)
		.await;
		let deliveries: Vec<String> = chunk
			.iter()
			.map(|id| {
				format!(
					"({}, {}, 'seeded-{id}', {}, 1, {}, 'apns', 'accepted', 1, {now})",
					sql.new_id(),
					sql.uuid(*id),
					sql.uuid(endpoint),
					sql.uuid(person),
				)
			})
			.collect();
		rows(
			db,
			format!(
				"INSERT INTO push_deliveries (id, notification, send_key, endpoint, generation, \
				 person, platform, state, attempts, created_at) VALUES {}",
				deliveries.join(", ")
			),
		)
		.await;
	}
}

/// A person with a long history is erased in batches: the call itself
/// removes everything bounded and one batch of inbox rows, and a job
/// removes the rest, a batch per write. Rows a later send writes are not
/// the erasure's to remove.
pub async fn forgetting_a_long_history_is_batched(address: std::net::SocketAddr, db: &Db) {
	let endpoint = enroll(address, ALICE, &apns(0xD1)).await;
	seed_history(db, ALICE, endpoint, 1_250).await;

	assert_eq!(held(db, "notifications", ALICE).await, 1_250);
	assert_eq!(held(db, "push_deliveries", ALICE).await, 1_250);

	ok(address, "forget_person", json!({ "person": ALICE }), None).await;

	assert_eq!(
		held(db, "notifications", ALICE).await,
		750,
		"the call's own transaction erases one batch, not the whole history"
	);
	assert_eq!(
		held(db, "push_deliveries", ALICE).await,
		750,
		"with exactly that batch's deliveries"
	);
	assert_eq!(held(db, "push_endpoints", ALICE).await, 0);

	// The app names her in a send while the job is still to run.
	send_to(address, "after-the-erasure", &[ALICE]).await;

	super::engine::drain(db).await;

	assert_eq!(
		held(db, "push_deliveries", ALICE).await,
		0,
		"every delivery of the erased history is gone"
	);
	assert_eq!(
		held(db, "notifications", ALICE).await,
		1,
		"the job erases the rest, and leaves what was sent afterwards"
	);
}

/// A pinned inbox page — one whose arguments name the row it ends at — is
/// bounded like any other: naming the oldest possible row doesn't read a
/// person's whole history in one answer.
pub async fn a_pinned_page_is_bounded(address: std::net::SocketAddr, db: &Db) {
	let endpoint = enroll(address, ALICE, &apns(0xE1)).await;
	seed_history(db, ALICE, endpoint, 1_250).await;

	let whole = ok(
		address,
		"inbox",
		json!({ "page": { "limit": 100, "until": format!("\"{}\"", Uuid::nil()) } }),
		Some(ALICE),
	)
	.await;
	let items = whole["items"].as_array().unwrap();
	assert_eq!(
		items.len(),
		500,
		"a pinned page reads a few pages' worth, not everything up to its end"
	);
	assert_eq!(
		items[0]["id"],
		Uuid::from_u128(1_250).to_string(),
		"newest first, as the inbox orders"
	);
	assert_eq!(whole["has_more"], json!(true));

	// A page pinned where a real page ended is unchanged.
	let first = ok(
		address,
		"inbox",
		json!({ "page": { "limit": 100 } }),
		Some(ALICE),
	)
	.await;
	let pinned = ok(
		address,
		"inbox",
		json!({ "page": { "limit": 100, "until": first["end"] } }),
		Some(ALICE),
	)
	.await;
	assert_eq!(pinned["items"], first["items"]);

	// The device list is bounded the same way.
	let devices = ok(
		address,
		"push_endpoints",
		json!({ "page": { "limit": 1, "until": format!("\"{}\"", Uuid::max()) } }),
		Some(ALICE),
	)
	.await;
	assert_eq!(devices["items"].as_array().unwrap().len(), 1);
}

/// One attribute call names a bounded number of keys: a very large `set`
/// or `remove` is refused as invalid before anything reads or writes,
/// instead of overflowing the database's parameter limit.
pub async fn an_attribute_call_names_a_bounded_number_of_keys(
	address: std::net::SocketAddr,
	db: &Db,
) {
	let keys = |count: usize| (0..count).map(|n| format!("k{n:05}"));

	// Past either database's parameter limit.
	let huge: serde_json::Map<String, Value> = keys(70_000).map(|key| (key, json!("v"))).collect();
	let (status, body) = call(
		address,
		"set_person_attributes",
		json!({ "persons": [ALICE], "set": huge }),
		None,
	)
	.await;
	assert_eq!(status, 400, "a very large map is a bad request: {body}");
	assert!(body.contains("at most 100 keys"), "{body}");

	let (status, body) = call(
		address,
		"set_person_attributes",
		json!({ "persons": [ALICE], "remove": keys(70_000).collect::<Vec<_>>() }),
		None,
	)
	.await;
	assert_eq!(status, 400, "{body}");
	assert!(body.contains("at most 100 keys"), "{body}");
	assert_eq!(
		held(db, "audience_members", ALICE).await,
		0,
		"refused before anything was written"
	);

	// The bound itself: a hundred keys across both — as many as a person
	// can hold to set, and as many again to remove — and not one more.
	let fifty: serde_json::Map<String, Value> = keys(50).map(|key| (key, json!("v"))).collect();
	ok(
		address,
		"set_person_attributes",
		json!({ "persons": [ALICE], "set": fifty }),
		None,
	)
	.await;
	let next: serde_json::Map<String, Value> =
		keys(100).skip(50).map(|key| (key, json!("w"))).collect();
	let replaced = ok(
		address,
		"set_person_attributes",
		json!({ "persons": [ALICE], "set": next, "remove": keys(50).collect::<Vec<_>>() }),
		None,
	)
	.await;
	assert_eq!(
		(&replaced["removed"], &replaced["skipped_full"]),
		(&json!(50), &json!(50)),
		"a hundred keys are one call; the person was full when the new ones were set: {replaced}"
	);
	assert_eq!(held(db, "person_attributes", ALICE).await, 0);

	let (status, body) = call(
		address,
		"set_person_attributes",
		json!({ "persons": [ALICE], "set": { "one_more": "v" }, "remove": keys(100).collect::<Vec<_>>() }),
		None,
	)
	.await;
	assert_eq!(status, 400, "a hundred and one: {body}");
}

/// An FCM credential, as an app's server exported it from its own table.
fn fcm(token: &str) -> Value {
	json!({ "fcm": { "registration_token": token } })
}

/// The app imports `endpoints`.
async fn import(address: std::net::SocketAddr, endpoints: Value) -> (u16, String) {
	call(
		address,
		"import_push_endpoints",
		json!({ "endpoints": endpoints }),
		None,
	)
	.await
}

/// What the app's server reads about `person`'s endpoints.
async fn person_endpoints(address: std::net::SocketAddr, person: Uuid) -> Vec<Value> {
	ok(
		address,
		"person_endpoints",
		json!({ "person": person, "page": { "limit": 50 } }),
		None,
	)
	.await["items"]
		.as_array()
		.unwrap()
		.clone()
}

/// Tokens an app's server exported from its Firebase-era table are
/// imported in one app-only call and are pushed to from the next send on,
/// before any device has updated. Importing again changes nothing. When
/// the real app later enrolls the same token, the endpoint keeps its id
/// and its queued deliveries, takes the details the device reports, and
/// stops counting as imported. An import never moves a token between
/// people, while a person who holds the device still takes it over, and
/// no answer carries a credential.
#[expect(
	clippy::cognitive_complexity,
	reason = "one scenario told in order: each step stands on the last"
)]
pub async fn imported_endpoints_work_until_the_app_enrolls(address: std::net::SocketAddr, db: &Db) {
	let alices_iphone = apns(0xC1);
	let batch = json!([
		{
			"person": ALICE,
			"credential": fcm("fcm-exported-alice-android"),
			"device": { "locale": "en_gb", "app_version": "2.9.0" },
		},
		{ "person": ALICE, "credential": alices_iphone },
		{ "person": BOB, "credential": fcm("fcm-exported-bob-android") },
	]);

	// Only the app imports.
	for person in [ALICE, BOB] {
		let (status, body) = call(
			address,
			"import_push_endpoints",
			json!({ "endpoints": batch }),
			Some(person),
		)
		.await;
		assert_eq!(status, 403, "{body}");
	}
	assert_eq!(held(db, "push_endpoints", ALICE).await, 0);

	let (status, body) = import(address, batch.clone()).await;
	assert_eq!(status, 200, "{body}");
	assert_eq!(
		serde_json::from_str::<Value>(&body).unwrap(),
		json!({ "imported": 3, "unchanged": 0, "skipped": [] })
	);
	assert!(
		!body.contains("fcm-exported") && !body.contains("c1c1"),
		"no credential is answered: {body}"
	);

	let listed = person_endpoints(address, ALICE).await;
	assert_eq!(listed.len(), 2, "{listed:?}");
	let android = listed
		.iter()
		.find(|endpoint| endpoint["platform"] == "fcm")
		.expect("the Android token");
	let iphone = listed
		.iter()
		.find(|endpoint| endpoint["platform"] == "apns")
		.expect("the iPhone token");
	assert_eq!(android["imported"], true, "{android}");
	assert_eq!(
		android["device"],
		json!({ "locale": "en-GB", "app_version": "2.9.0" }),
		"an import states what the server knows about the device"
	);
	assert_eq!(iphone["imported"], true, "{iphone}");
	assert_eq!(
		iphone["device"],
		json!({}),
		"and nothing where it knows none"
	);
	assert!(
		!serde_json::to_string(&listed)
			.unwrap()
			.contains("fcm-exported"),
		"{listed:?}"
	);
	// The person sees them in their own device list too.
	assert_eq!(endpoints_of(address, ALICE).await.len(), 2);

	// Day one: a send reaches the imported endpoints.
	send_to(address, "after-the-import", &[ALICE]).await;
	assert_eq!(
		deliveries_of(db, ALICE).await,
		vec![("enqueued".to_owned(), String::new()); 2]
	);

	// Importing the same export again is no change at all.
	let (status, body) = import(address, batch).await;
	assert_eq!(status, 200, "{body}");
	assert_eq!(
		serde_json::from_str::<Value>(&body).unwrap(),
		json!({ "imported": 0, "unchanged": 3, "skipped": [] })
	);
	assert_eq!(person_endpoints(address, ALICE).await, listed);

	// The updated app runs on the iPhone and enrolls the token it holds:
	// the same endpoint, now the device's own.
	let enrolled = ok(
		address,
		"register_push_endpoint",
		json!({
			"credential": alices_iphone,
			"device": { "locale": "fr-FR", "app_version": "3.0.0" },
		}),
		Some(ALICE),
	)
	.await;
	assert_eq!(enrolled["id"], iphone["id"], "the endpoint keeps its id");
	let listed = person_endpoints(address, ALICE).await;
	let iphone = listed
		.iter()
		.find(|endpoint| endpoint["platform"] == "apns")
		.unwrap();
	assert_eq!(
		iphone.get("imported").unwrap_or(&json!(false)),
		false,
		"{iphone}"
	);
	assert_eq!(
		iphone["device"],
		json!({ "locale": "fr-FR", "app_version": "3.0.0" })
	);
	assert_eq!(
		deliveries_of(db, ALICE).await,
		vec![("enqueued".to_owned(), String::new()); 2],
		"what was queued for the imported endpoint still goes to the device"
	);

	// A later import of that token leaves the device's own enrollment as
	// it is.
	let (status, body) = import(
		address,
		json!([{ "person": ALICE, "credential": alices_iphone, "device": { "locale": "de" } }]),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	assert_eq!(person_endpoints(address, ALICE).await, listed);

	// An import proves nothing about who holds a device: a token another
	// person has stays theirs, and the entry is reported by its place in
	// the call.
	let (status, body) = import(
		address,
		json!([
			{ "person": BOB, "credential": alices_iphone },
			{ "person": BOB, "credential": fcm("fcm-exported-alice-android") },
			{ "person": BOB, "credential": fcm("fcm-exported-bob-tablet") },
		]),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	assert_eq!(
		serde_json::from_str::<Value>(&body).unwrap(),
		json!({
			"imported": 1,
			"unchanged": 0,
			"skipped": [
				{ "index": 0, "reason": "held_by_another" },
				{ "index": 1, "reason": "held_by_another" },
			],
		})
	);
	assert_eq!(person_endpoints(address, ALICE).await, listed);

	// Bob signs in on the Android phone Alice's export named and turns
	// notifications on: holding the token is holding the device, imported
	// or not.
	let bobs = enroll(address, BOB, &fcm("fcm-exported-alice-android")).await;
	assert_ne!(json!(bobs), android["id"]);
	assert_eq!(person_endpoints(address, ALICE).await.len(), 1);
	let bobs_now = person_endpoints(address, BOB).await;
	assert_eq!(bobs_now.len(), 3, "{bobs_now:?}");
	let taken = bobs_now
		.iter()
		.find(|endpoint| endpoint["id"] == json!(bobs))
		.unwrap();
	assert_eq!(taken.get("imported").unwrap_or(&json!(false)), false);
}

/// An import is checked whole before anything is written and bounded per
/// call and per person, and what it made is revoked and erased like any
/// other endpoint.
#[expect(
	clippy::cognitive_complexity,
	reason = "each refusal and each bound, one after another"
)]
pub async fn an_import_is_bounded_and_ends_like_any_endpoint(
	address: std::net::SocketAddr,
	db: &Db,
) {
	// One bad entry refuses the call by its place, and nothing is written.
	for (endpoints, names) in [
		(json!([]), "1 to 500"),
		(
			Value::Array(
				(0..501)
					.map(
						|n| json!({ "person": ALICE, "credential": fcm(&format!("fcm-many-{n}")) }),
					)
					.collect(),
			),
			"1 to 500",
		),
		(
			json!([
				{ "person": ALICE, "credential": fcm("fcm-fine") },
				{ "person": ALICE, "credential": { "apns": { "device_token": "not hex" } } },
			]),
			"endpoints[1]",
		),
		(
			json!([{
				"person": ALICE,
				"credential": { "web_push": {
					"endpoint": "https://updates.push.services.mozilla.com/push/1",
					"keys": { "p256dh": "BQUF", "auth": "BwcH" },
				} },
			}]),
			"endpoints[0]",
		),
		(
			json!([{
				"person": ALICE,
				"credential": fcm("fcm-fine"),
				"device": { "time_zone": "/not a zone" },
			}]),
			"endpoints[0]",
		),
	] {
		let (status, body) = import(address, endpoints).await;
		assert_eq!(status, 400, "{names}: {body}");
		assert!(body.contains(names), "{names}: {body}");
	}
	assert_eq!(held(db, "push_endpoints", ALICE).await, 0);

	// A person holds ten live endpoints however they came to be: the
	// eleventh and twelfth are reported, and the rest of the call stands.
	let twelve = Value::Array(
		(0..12)
			.map(|n| json!({ "person": ALICE, "credential": fcm(&format!("fcm-alice-{n}")) }))
			.chain([json!({ "person": BOB, "credential": apns(0xD1) })])
			.collect(),
	);
	let (status, body) = import(address, twelve).await;
	assert_eq!(status, 200, "{body}");
	assert_eq!(
		serde_json::from_str::<Value>(&body).unwrap(),
		json!({
			"imported": 11,
			"unchanged": 0,
			"skipped": [
				{ "index": 10, "reason": "person_full" },
				{ "index": 11, "reason": "person_full" },
			],
		})
	);
	assert_eq!(held(db, "push_endpoints", ALICE).await, 10);

	// A provider invalidates one; importing its stale token again does
	// not bring it back.
	let sql = Sql(db.dialect());
	rows(
		db,
		format!(
			"UPDATE push_endpoints SET invalidated_at = created_at WHERE person = {}",
			sql.uuid(BOB)
		),
	)
	.await;
	let (status, body) = import(
		address,
		json!([{ "person": BOB, "credential": apns(0xD1) }]),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	assert_eq!(
		serde_json::from_str::<Value>(&body).unwrap(),
		json!({ "imported": 0, "unchanged": 1, "skipped": [] })
	);
	let bobs = person_endpoints(address, BOB).await;
	assert!(!bobs[0]["invalidated_at"].is_null(), "{bobs:?}");

	// The server revokes what it imported, and a person is erased with
	// their imported endpoints.
	send_to(address, "to-imported", &[ALICE]).await;
	let revoked = ok(
		address,
		"revoke_person_endpoints",
		json!({ "person": ALICE }),
		None,
	)
	.await;
	assert_eq!(revoked, json!(10));
	assert!(
		endpoints_of(address, ALICE).await.is_empty(),
		"the imported endpoints are revoked"
	);
	assert_eq!(
		deliveries_of(db, ALICE).await,
		vec![("skipped".to_owned(), "endpoint_revoked".to_owned()); 10]
	);

	ok(address, "forget_person", json!({ "person": BOB }), None).await;
	super::engine::drain(db).await;
	assert_eq!(held(db, "push_endpoints", BOB).await, 0);
}

/// An import names only platforms this server can push to.
pub async fn an_import_needs_the_platforms_transport(address: std::net::SocketAddr, db: &Db) {
	let (status, body) = import(
		address,
		json!([
			{ "person": ALICE, "credential": apns(0xE1) },
			{ "person": ALICE, "credential": fcm("fcm-without-a-transport") },
		]),
	)
	.await;
	assert_eq!(status, 400, "{body}");
	assert!(
		body.contains("push for fcm is not configured"),
		"the refusal names the platform: {body}"
	);
	assert_eq!(
		held(db, "push_endpoints", ALICE).await,
		0,
		"nothing was written"
	);
}

/// An Apple endpoint keeps the APNs environment its build reported: the
/// app's server reads it back, a later enrollment replaces it, an import
/// can state it, and an endpoint that is not Apple's stores none.
pub async fn an_endpoint_keeps_the_apns_environment_it_reported(
	address: std::net::SocketAddr,
	db: &Db,
) {
	let phone = apns(0xF1);
	let enroll_with = |credential: Value, device: Value| {
		async move {
			ok(
				address,
				"register_push_endpoint",
				json!({ "credential": credential, "device": device }),
				Some(ALICE),
			)
			.await
		}
	};
	let stored = || {
		async {
			let sql = Sql(db.dialect());
			rows(
				db,
				format!(
					"SELECT platform, apns_environment FROM push_endpoints WHERE person = {} \
					 ORDER BY id",
					sql.uuid(ALICE)
				),
			)
			.await
			.into_iter()
			.map(|row| {
				let text = |value: &leptoskit_db::Value| {
					match value {
						leptoskit_db::Value::Text(text) => Some(text.clone()),
						_ => None,
					}
				};
				(text(&row[0]).unwrap_or_default(), text(&row[1]))
			})
			.collect::<Vec<_>>()
		}
	};

	let enrolled = enroll_with(
		phone.clone(),
		json!({ "apns_environment": "production", "locale": "en" }),
	)
	.await;
	assert_eq!(
		enrolled["device"],
		json!({ "locale": "en", "apns_environment": "production" })
	);
	assert_eq!(
		stored().await,
		vec![("apns".to_owned(), Some("production".to_owned()))]
	);
	assert_eq!(
		person_endpoints(address, ALICE).await[0]["device"]["apns_environment"],
		"production"
	);

	// A development build installed over it says so, on the same endpoint.
	let again = enroll_with(
		phone.clone(),
		json!({ "apns_environment": "sandbox", "locale": "en" }),
	)
	.await;
	assert_eq!(again["id"], enrolled["id"]);
	assert_eq!(
		stored().await,
		vec![("apns".to_owned(), Some("sandbox".to_owned()))]
	);

	// A build that reports nothing leaves none: the service's configured
	// environment is used for it.
	enroll_with(phone, json!({ "locale": "en" })).await;
	assert_eq!(stored().await, vec![("apns".to_owned(), None)]);

	// An import states it for an Apple token, and an FCM token has none
	// whatever the call says.
	let (status, body) = import(
		address,
		json!([
			{
				"person": ALICE,
				"credential": apns(0xF2),
				"device": { "apns_environment": "production" },
			},
			{
				"person": ALICE,
				"credential": fcm("fcm-with-an-environment"),
				"device": { "apns_environment": "production" },
			},
		]),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	assert_eq!(
		stored().await,
		vec![
			("apns".to_owned(), None),
			("apns".to_owned(), Some("production".to_owned())),
			("fcm".to_owned(), None),
		]
	);
}
