//! Device details and device targeting, end to end on a real port: what
//! each endpoint says about itself is stored, normalized, replaced on
//! re-enrollment without rotating its credential, and read back only by
//! its owner; and a condition's `device.` predicates are judged per
//! device — the person once with no device and once per live endpoint,
//! one inbox row when any holds, a push only to the endpoints it holds for
//! — on immediate and scheduled sends, refused by name when they ask about
//! something a device doesn't report, and reversible in the migration that
//! added them.
//!
//! The transports run over an injected boundary that records every
//! provider request and accepts it, so a test can count which platforms a
//! send reached. Nothing leaves the machine.

#![cfg(not(target_arch = "wasm32"))]

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use leptoskit_db::Connection as _;
use leptoskit_db::Db;
use leptoskit_db::Read as _;
use leptoskit_db::Statement;
use leptoskit_notifications::MIGRATIONS;
use leptoskit_notifications::SERVICE;
use leptoskit_notifications::SendProgress;
use leptoskit_notifications::SendResult;
use leptoskit_notifications::SendState;
use leptoskit_notifications::service::config;
use leptoskit_notifications::service::push;
use leptoskit_notifications::service::transport::Boundary;
use leptoskit_notifications::service::transport::PushRuntime;
use leptoskit_notifications::service::transport::WireError;
use leptoskit_notifications::service::transport::WireRequest;
use leptoskit_notifications::service::transport::WireResponse;
use leptoskit_rpc as rpc;
use rpc::server::Anonymous;
use rpc::server::ServiceCallers;
use serde_json::Value;
use serde_json::json;
use uuid::Uuid;

const TOKEN: &str = "app-token";
const ORIGIN: &str = "http://app.test";
const PREFIX: &str = "/_leptoskit/services/notifications";

const APNS_HOST: &str = "api.sandbox.push.apple.com";
const FCM_HOST: &str = "fcm.googleapis.com";
const FCM_TOKEN_HOST: &str = "oauth2.googleapis.com";

const ALICE: Uuid = Uuid::from_u128(0xA110_0000_0000_0000_0000_0000_0000_0001);
const BOB: Uuid = Uuid::from_u128(0xB0B0_0000_0000_0000_0000_0000_0000_0002);
const CAROL: Uuid = Uuid::from_u128(0xCA20_0000_0000_0000_0000_0000_0000_0003);
const DAVE: Uuid = Uuid::from_u128(0xDA7E_0000_0000_0000_0000_0000_0000_0004);
const ERIN: Uuid = Uuid::from_u128(0xE210_0000_0000_0000_0000_0000_0000_0005);
const FRANK: Uuid = Uuid::from_u128(0xF2A2_0000_0000_0000_0000_0000_0000_0006);

/// How long a fanout or a push round may take on a loaded machine.
const SETTLES: Duration = Duration::from_secs(30);

/// A test-only 2048-bit RSA key, the shape Google's service-account JSON
/// carries; it signs nothing outside these tests, whose provider sockets
/// are all the recorder below.
const TEST_RSA_KEY: &str = include_str!("fixtures/test-only-rsa-key.pem");

/// The provider boundary: it records each request's URL, answers the FCM
/// token exchange with an access token, and accepts everything else.
#[derive(Default)]
struct Recorder {
	urls: std::sync::Mutex<Vec<String>>,
}

impl Recorder {
	/// How many requests went to `host`.
	fn sent_to(&self, host: &str) -> usize {
		self.urls
			.lock()
			.unwrap()
			.iter()
			.filter(|url| url.split('/').nth(2) == Some(host))
			.count()
	}
}

impl Boundary for Recorder {
	fn post(
		&self,
		request: WireRequest,
	) -> Pin<Box<dyn Future<Output = Result<WireResponse, WireError>> + Send + '_>> {
		let exchange = request.url.contains(FCM_TOKEN_HOST);
		self.urls.lock().unwrap().push(request.url);

		Box::pin(async move {
			Ok(if exchange {
				WireResponse {
					status: 200,
					headers: vec![("content-type".into(), "application/json".into())],
					body: json!({ "access_token": "ya29.devices", "expires_in": 3600 })
						.to_string()
						.into_bytes(),
				}
			} else {
				WireResponse {
					status: 200,
					headers: Vec::new(),
					body: Vec::new(),
				}
			})
		})
	}
}

struct Service {
	address: std::net::SocketAddr,
	db: Db,
	recorder: Arc<Recorder>,
	jobs: Option<rpc::server::JobsHandle>,
}

impl Drop for Service {
	fn drop(&mut self) {
		if let Some(jobs) = self.jobs.take() {
			jobs.stop();
		}
	}
}

/// A fresh, migrated service with APNs and FCM over the recorder, and its
/// workers on a test-fast poll.
async fn service() -> Service {
	let db = leptoskit_db::testing::sqlite_migrated(MIGRATIONS).await;
	let recorder = Arc::new(Recorder::default());
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
		fcm: Some(config::FcmConfig {
			service_account_json: json!({
				"type": "service_account",
				"project_id": "leptoskit-devices",
				"private_key_id": "0123456789abcdef0123456789abcdef01234567",
				"private_key": TEST_RSA_KEY,
				"client_email": "push@leptoskit-devices.iam.gserviceaccount.com",
				"client_id": "123456789000000000000",
			})
			.to_string(),
		}),
		web: None,
	};
	let push = push::Push {
		runtime: Some(Arc::new(
			PushRuntime::build_over(&config, recorder.clone()).expect("the runtime builds"),
		)),
	};
	let callers = ServiceCallers::new(Anonymous)
		.caller(TOKEN, rpc::server::APP_CALLER)
		.forwarded_identity(SERVICE, TOKEN);
	let (router, live) = leptoskit_notifications::service::routes_with_push(
		db.clone(),
		Some(ORIGIN),
		callers,
		push.clone(),
	);
	live.start().await.expect("live starts");
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	tokio::spawn(async move {
		leptoskit_axum::axum::serve(listener, router).await.unwrap();
	});
	let jobs = leptoskit_notifications::service::workers_with_push(db.clone(), push)
		.poll(Duration::from_millis(50))
		.start();

	Service {
		address,
		db,
		recorder,
		jobs: Some(jobs),
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

/// An APNs credential, distinct per `seed`.
fn apns(seed: u8) -> Value {
	json!({ "apns": { "device_token": format!("{seed:02x}").repeat(32) } })
}

/// An FCM credential, distinct per `seed`.
fn fcm(seed: u8) -> Value {
	json!({ "fcm": { "registration_token": format!("fcm-device-{seed}:token") } })
}

/// `person` enrolls `credential` reporting `device`, and the answer — the
/// owner's endpoint summary — comes back.
async fn enroll(service: &Service, person: Uuid, credential: &Value, device: Value) -> Value {
	let (status, body) = post(
		service,
		"register_push_endpoint",
		json!({ "credential": credential, "device": device }),
		Some(&signed_in(person)),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	serde_json::from_str(&body).unwrap()
}

/// The person's own endpoint list.
async fn endpoints_of(service: &Service, person: Uuid) -> Vec<Value> {
	let (status, body) = post(
		service,
		"push_endpoints",
		json!({ "page": { "limit": 100 } }),
		Some(&signed_in(person)),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let page: Value = serde_json::from_str(&body).unwrap();
	page["items"].as_array().cloned().unwrap_or_default()
}

/// The app gives `persons` the attribute `plan`.
async fn plan(service: &Service, value: &str, persons: &[Uuid]) {
	let (status, body) = post(
		service,
		"set_person_attributes",
		json!({ "persons": persons, "set": { "plan": value }, "remove": [] }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");
}

/// The app sends to `condition`, with `extra` merged into the spec.
async fn send_condition(
	service: &Service,
	key: &str,
	condition: &str,
	extra: Value,
	validate_only: bool,
) -> (u16, String) {
	let mut spec = json!({ "category": "updates", "title": "Please update" });
	if let (Some(spec), Some(extra)) = (spec.as_object_mut(), extra.as_object()) {
		spec.extend(extra.clone());
	}
	post(
		service,
		"send",
		json!({
			"idempotency_key": key,
			"spec": spec,
			"target": { "condition": condition },
			"validate_only": validate_only,
		}),
		Some(TOKEN),
	)
	.await
}

/// `send_status` until the send leaves `fanning_out`.
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

/// A person's inbox, every visible item.
async fn inbox_len(service: &Service, person: Uuid) -> usize {
	let (status, body) = post(
		service,
		"inbox",
		json!({ "page": { "limit": 100 }, "show": "all" }),
		Some(&signed_in(person)),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let page: Value = serde_json::from_str(&body).unwrap();
	page["items"].as_array().map_or(0, Vec::len)
}

/// The app's page of a send's deliveries.
async fn deliveries(service: &Service, key: &str) -> Vec<Value> {
	let (status, body) = post(
		service,
		"push_deliveries",
		json!({ "send_key": key, "page": { "limit": 100 } }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let page: Value = serde_json::from_str(&body).unwrap();
	page["items"].as_array().cloned().unwrap_or_default()
}

/// The deliveries of `key`, once every one has left the queue.
async fn finished(service: &Service, key: &str) -> Vec<Value> {
	let deadline = tokio::time::Instant::now() + SETTLES;

	loop {
		let rows = deliveries(service, key).await;

		if rows
			.iter()
			.all(|row| !matches!(row["state"].as_str(), Some("enqueued" | "retrying")))
		{
			return rows;
		}

		assert!(
			tokio::time::Instant::now() < deadline,
			"the deliveries never finished: {rows:?}"
		);
		tokio::time::sleep(Duration::from_millis(50)).await;
	}
}

/// Who and where a send's deliveries went: `(person, endpoint)` pairs,
/// sorted.
fn reached(rows: &[Value]) -> Vec<(Uuid, Uuid)> {
	let mut pairs: Vec<(Uuid, Uuid)> = rows
		.iter()
		.map(|row| {
			(
				row["person"].as_str().unwrap().parse().unwrap(),
				row["endpoint"].as_str().unwrap().parse().unwrap(),
			)
		})
		.collect();
	pairs.sort();
	pairs
}

/// A uuid as a SQLite blob literal.
fn blob(id: Uuid) -> String {
	use std::fmt::Write as _;

	let mut hex = String::from("x'");
	for byte in id.as_bytes() {
		let _ = write!(hex, "{byte:02x}");
	}
	hex.push('\'');
	hex
}

async fn rows(db: &Db, sql: &str) -> Vec<Vec<leptoskit_db::Value>> {
	db.fetch(Statement {
		sql: sql.into(),
		args: Vec::new(),
	})
	.await
	.expect("a read")
}

fn text(value: &leptoskit_db::Value) -> String {
	match value {
		leptoskit_db::Value::Text(text) => text.clone(),
		leptoskit_db::Value::Int(int) => int.to_string(),
		leptoskit_db::Value::Null => "NULL".into(),
		other => format!("{other:?}"),
	}
}

/// An endpoint's stored generation, details, and credential, as text.
async fn stored(db: &Db, endpoint: Uuid) -> Vec<String> {
	rows(
		db,
		&format!(
			"SELECT generation, locale, time_zone, app_version, credential FROM push_endpoints \
			 WHERE id = {}",
			blob(endpoint)
		),
	)
	.await[0]
		.iter()
		.map(text)
		.collect()
}

fn id_of(summary: &Value) -> Uuid {
	summary["id"]
		.as_str()
		.expect("an id")
		.parse()
		.expect("a uuid")
}

/// What a device reports, as the clients send it.
fn reports(locale: &str, time_zone: &str, app_version: &str) -> Value {
	json!({ "locale": locale, "time_zone": time_zone, "app_version": app_version })
}

/// Details are stored normalized, answered back to their owner, and
/// replaced by a re-enrollment — with the credential's generation and the
/// queued delivery left exactly as they were — and an enrollment that
/// reports none replaces them with none.
#[tokio::test(flavor = "multi_thread")]
async fn details_are_stored_and_replaced_without_a_credential_rotation() {
	let service = service().await;
	let phone = apns(1);

	let summary = enroll(
		&service,
		ALICE,
		&phone,
		json!({ "locale": " en_gb ", "time_zone": "Europe/London", "app_version": "2.1.0" }),
	)
	.await;
	let endpoint = id_of(&summary);
	assert_eq!(
		summary["device"],
		reports("en-GB", "Europe/London", "2.1.0"),
		"normalized: {summary}"
	);

	let first = stored(&service.db, endpoint).await;
	assert_eq!(first[..4], ["1", "en-GB", "Europe/London", "2.1.0"]);

	// A scheduled send leaves a delivery in the queue for this endpoint.
	let later = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
	let (status, body) = post(
		&service,
		"send",
		json!({
			"idempotency_key": "queued-for-alice",
			"spec": { "category": "updates", "title": "Later", "visible_at": later },
			"target": { "recipients": [ALICE] },
		}),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let queued = |db: Db| {
		async move {
			rows(
				&db,
				&format!(
					"SELECT generation, state, next_attempt_at, attempts FROM push_deliveries \
					 WHERE endpoint = {}",
					blob(endpoint)
				),
			)
			.await
			.into_iter()
			.map(|row| row.iter().map(text).collect::<Vec<_>>())
			.collect::<Vec<_>>()
		}
	};
	let before = queued(service.db.clone()).await;
	assert_eq!(before.len(), 1, "{before:?}");
	assert_eq!(before[0][..2], ["1", "enqueued"]);

	// The app updated and the person changed language and moved: the same
	// device re-enrolls with new details.
	let summary = enroll(
		&service,
		ALICE,
		&phone,
		reports("fr-CA", "America/Toronto", "2.2.0"),
	)
	.await;
	assert_eq!(id_of(&summary), endpoint, "the same endpoint");
	assert_eq!(
		summary["device"],
		reports("fr-CA", "America/Toronto", "2.2.0")
	);

	let second = stored(&service.db, endpoint).await;
	assert_eq!(
		second[..4],
		["1", "fr-CA", "America/Toronto", "2.2.0"],
		"the details changed and the generation didn't"
	);
	assert_eq!(second[4], first[4], "the credential is untouched");
	assert_eq!(
		queued(service.db.clone()).await,
		before,
		"the queued delivery is exactly as it was: same generation, state, and schedule"
	);

	// The same details again change nothing; an enrollment that reports
	// none — an older client — replaces them with none.
	enroll(
		&service,
		ALICE,
		&phone,
		reports("fr-CA", "America/Toronto", "2.2.0"),
	)
	.await;
	assert_eq!(stored(&service.db, endpoint).await, second);
	let (status, body) = post(
		&service,
		"register_push_endpoint",
		json!({ "credential": phone }),
		Some(&signed_in(ALICE)),
	)
	.await;
	assert_eq!(
		status, 200,
		"a request with no details still enrolls: {body}"
	);
	let summary: Value = serde_json::from_str(&body).unwrap();
	assert_eq!(summary["device"], json!({}));
	assert_eq!(
		stored(&service.db, endpoint).await[..4],
		["1", "NULL", "NULL", "NULL"]
	);
	assert_eq!(queued(service.db.clone()).await, before);
}

/// A detail that fails its check refuses the enrollment by name and
/// changes nothing stored; a fresh enrollment stores its details from the
/// first insert, and the owner's list answers them.
#[tokio::test(flavor = "multi_thread")]
async fn bad_details_are_refused_by_name_and_change_nothing() {
	let service = service().await;
	let phone = apns(3);
	let endpoint = id_of(&enroll(&service, ALICE, &phone, reports("en-GB", "UTC", "2.3.0")).await);
	let kept = stored(&service.db, endpoint).await;
	for (device, named) in [
		(json!({ "locale": "not a tag" }), "locale"),
		(json!({ "locale": "x-private" }), "locale"),
		(json!({ "time_zone": "Europe/../London" }), "time zone"),
		(
			json!({ "time_zone": "Europe/".to_owned() + &"L".repeat(64) }),
			"time zone",
		),
		(json!({ "app_version": "9".repeat(65) }), "app version"),
		(json!({ "app_version": "2.1\u{7}beta" }), "app version"),
	] {
		let (status, body) = post(
			&service,
			"register_push_endpoint",
			json!({ "credential": phone, "device": device }),
			Some(&signed_in(ALICE)),
		)
		.await;
		assert_eq!(status, 400, "{device}: {body}");
		assert!(
			body.contains(named),
			"{device}: the refusal names it: {body}"
		);
		assert_eq!(stored(&service.db, endpoint).await, kept, "{device}");
	}

	// A fresh enrollment stores its details from the first insert.
	let laptop = enroll(
		&service,
		ALICE,
		&fcm(1),
		reports("de-DE", "Europe/Berlin", "2.3.0"),
	)
	.await;
	let listed = endpoints_of(&service, ALICE).await;
	assert_eq!(listed.len(), 2);
	let listed_laptop = listed
		.iter()
		.find(|item| item["id"] == laptop["id"])
		.expect("listed");
	assert_eq!(
		listed_laptop["device"],
		reports("de-DE", "Europe/Berlin", "2.3.0")
	);
}

/// Details are their owner's: the person's own list answers them, nobody
/// else's list holds the endpoint, and nothing the app reads about
/// deliveries or test sends carries them.
#[tokio::test(flavor = "multi_thread")]
async fn details_are_read_by_their_owner_alone() {
	let service = service().await;
	enroll(
		&service,
		ALICE,
		&apns(2),
		reports("en-GB", "Europe/London", "2.1.0"),
	)
	.await;

	let mine = endpoints_of(&service, ALICE).await;
	assert_eq!(mine.len(), 1);
	assert_eq!(
		mine[0]["device"],
		reports("en-GB", "Europe/London", "2.1.0")
	);
	assert!(
		endpoints_of(&service, BOB).await.is_empty(),
		"Bob sees nothing"
	);

	// The app's own reads about pushes name endpoints by id and platform,
	// and nothing a device reported.
	let (status, body) = post(
		&service,
		"send",
		json!({
			"idempotency_key": "for-alice",
			"spec": { "category": "updates", "title": "Hi" },
			"target": { "recipients": [ALICE] },
		}),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let rows = finished(&service, "for-alice").await;
	assert_eq!(rows.len(), 1);
	let (status, test) = post(
		&service,
		"send_test",
		json!({ "spec": { "category": "updates", "title": "Test" }, "person": ALICE }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{test}");

	for answer in [serde_json::to_string(&rows).unwrap(), test] {
		for detail in ["en-GB", "Europe/London", "2.1.0", "locale", "device"] {
			assert!(
				!answer.contains(detail),
				"the app's answer carries no device detail ({detail}): {answer}"
			);
		}
	}

	// A person can't read another's list by any argument: the list takes
	// none, and an anonymous caller is refused.
	let (status, _) = post(
		&service,
		"push_endpoints",
		json!({ "page": { "limit": 100 } }),
		None,
	)
	.await;
	assert_eq!(status, 403);
}

/// The per-device rule on an immediate send: Alice's iPhone matches and
/// her Android phone doesn't — one inbox row, one push, to the iPhone.
/// Carol's one iPhone matches; Bob holds only an Android phone, which
/// doesn't, so he isn't in the audience at all. `validate_only` counts
/// the same audience, and the providers see exactly the matching pushes.
#[tokio::test(flavor = "multi_thread")]
async fn a_device_condition_reaches_each_person_once_and_pushes_only_matching_endpoints() {
	let service = service().await;
	let alice_phone = id_of(
		&enroll(
			&service,
			ALICE,
			&apns(10),
			reports("en-GB", "Europe/London", "2.0.4"),
		)
		.await,
	);
	enroll(
		&service,
		ALICE,
		&fcm(10),
		reports("fr-FR", "Europe/Paris", "2.0.4"),
	)
	.await;
	enroll(
		&service,
		BOB,
		&fcm(11),
		reports("en-US", "America/New_York", "1.0.0"),
	)
	.await;
	let carol_phone = id_of(
		&enroll(
			&service,
			CAROL,
			&apns(12),
			reports("en-AU", "Australia/Sydney", "2.0"),
		)
		.await,
	);
	// Dave is current: his iPhone's version is past the bound.
	enroll(
		&service,
		DAVE,
		&apns(13),
		reports("en-GB", "Europe/London", "2.10.0"),
	)
	.await;

	let condition = "device.platform == 'apns' && device.app_version < '2.1'";

	let (status, body) =
		send_condition(&service, "update-ios-dry", condition, json!({}), true).await;
	assert_eq!(status, 200, "{body}");
	let dry: SendResult = serde_json::from_str(&body).unwrap();
	assert_eq!(
		(dry.targeted, dry.delivered),
		(2, 2),
		"Alice and Carol, counted before anything is written"
	);

	let (status, body) = send_condition(&service, "update-ios", condition, json!({}), false).await;
	assert_eq!(status, 200, "{body}");
	let progress = settled(&service, "update-ios").await;
	assert_eq!(
		(progress.state, progress.targeted, progress.delivered),
		(SendState::Complete, 2, 2),
		"{progress:?}"
	);

	assert_eq!(
		inbox_len(&service, ALICE).await,
		1,
		"one row for two devices"
	);
	assert_eq!(inbox_len(&service, CAROL).await, 1);
	assert_eq!(
		inbox_len(&service, BOB).await,
		0,
		"only non-matching devices"
	);
	assert_eq!(inbox_len(&service, DAVE).await, 0, "a current version");

	let rows = finished(&service, "update-ios").await;
	assert_eq!(
		reached(&rows),
		{
			let mut expected = vec![(ALICE, alice_phone), (CAROL, carol_phone)];
			expected.sort();
			expected
		},
		"one push each, to the matching iPhone: {rows:?}"
	);
	assert!(
		rows.iter()
			.all(|row| row["platform"] == "apns" && row["state"] == "accepted"),
		"{rows:?}"
	);
	assert_eq!(service.recorder.sent_to(APNS_HOST), 2);
	assert_eq!(
		service.recorder.sent_to(FCM_HOST),
		0,
		"no Android phone was pushed"
	);
}

/// With no device, a device predicate answers as an absent attribute does,
/// so a negated one holds: Dave, on the pro plan with no endpoints at all,
/// is in the audience and gets his inbox row with no push. Erin's only
/// device is an iPhone the condition excludes — she still matches with no
/// device, so she gets the row, and her iPhone gets no push. Alice's
/// Android phone is the one of her two devices that gets it. Frank isn't
/// on the plan.
#[tokio::test(flavor = "multi_thread")]
async fn a_negated_device_predicate_holds_with_no_device() {
	let service = service().await;
	plan(&service, "pro", &[ALICE, DAVE, ERIN]).await;
	plan(&service, "free", &[FRANK]).await;
	enroll(&service, ALICE, &apns(20), json!({})).await;
	let alice_android = id_of(&enroll(&service, ALICE, &fcm(20), json!({})).await);
	enroll(&service, ERIN, &apns(21), json!({})).await;
	enroll(&service, FRANK, &fcm(22), json!({})).await;

	let condition = "user.plan == 'pro' && !(device.platform == 'apns')";
	let (status, body) = send_condition(&service, "not-ios", condition, json!({}), false).await;
	assert_eq!(status, 200, "{body}");
	let progress = settled(&service, "not-ios").await;
	assert_eq!(
		(progress.targeted, progress.delivered),
		(3, 3),
		"{progress:?}"
	);

	for (person, rows) in [(ALICE, 1), (DAVE, 1), (ERIN, 1), (FRANK, 0)] {
		assert_eq!(inbox_len(&service, person).await, rows, "{person}");
	}

	let rows = finished(&service, "not-ios").await;
	assert_eq!(
		reached(&rows),
		vec![(ALICE, alice_android)],
		"only Alice's Android phone is pushed: {rows:?}"
	);
	assert_eq!(service.recorder.sent_to(APNS_HOST), 0);
	assert_eq!(service.recorder.sent_to(FCM_HOST), 1);
}

/// Only live endpoints count: a person whose matching device the provider
/// invalidated, and whose live device doesn't match, is not in the
/// audience — and neither is one whose only devices never matched. The
/// language predicate reads the locale's primary subtag, and a topic
/// anchors a device condition to its subscribers.
#[tokio::test(flavor = "multi_thread")]
async fn only_live_matching_endpoints_bring_a_person_in() {
	let service = service().await;
	// Bob's devices are all English; Carol's French one is invalidated.
	enroll(&service, BOB, &apns(30), reports("en-GB", "UTC", "1")).await;
	enroll(&service, BOB, &fcm(30), reports("en-US", "UTC", "1")).await;
	let dead = id_of(&enroll(&service, CAROL, &apns(31), reports("fr-FR", "UTC", "1")).await);
	enroll(&service, CAROL, &fcm(31), reports("de-DE", "UTC", "1")).await;
	service
		.db
		.execute(Statement {
			sql: format!(
				"UPDATE push_endpoints SET invalidated_at = '2026-11-01T00:00:00Z' WHERE id = {}",
				blob(dead)
			),
			args: Vec::new(),
		})
		.await
		.expect("invalidated");
	// Alice's Canadian phone matches the language.
	let alice_phone =
		id_of(&enroll(&service, ALICE, &apns(32), reports("fr-CA", "UTC", "1")).await);
	enroll(&service, ALICE, &fcm(32), reports("en-CA", "UTC", "1")).await;

	let (status, body) = send_condition(
		&service,
		"french",
		"device.language == 'fr'",
		json!({}),
		false,
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let progress = settled(&service, "french").await;
	assert_eq!(progress.targeted, 1, "{progress:?}");
	assert_eq!(inbox_len(&service, ALICE).await, 1);
	assert_eq!(
		inbox_len(&service, BOB).await,
		0,
		"no device of his matches"
	);
	assert_eq!(
		inbox_len(&service, CAROL).await,
		0,
		"her matching device is invalidated"
	);
	assert_eq!(
		reached(&finished(&service, "french").await),
		vec![(ALICE, alice_phone)]
	);

	// A topic anchors the walk; the device still picks the endpoints.
	let (status, body) = post(
		&service,
		"subscribe_to_topic",
		json!({ "topic": "news", "persons": [ALICE, BOB] }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let (status, body) = send_condition(
		&service,
		"news-en",
		"'news' in topics && device.locale in ['en-GB', 'en-CA']",
		json!({}),
		false,
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let progress = settled(&service, "news-en").await;
	assert_eq!(progress.targeted, 2, "{progress:?}");
	let rows = finished(&service, "news-en").await;
	assert_eq!(
		rows.len(),
		2,
		"Bob's en-GB phone and Alice's en-CA one: {rows:?}"
	);
	assert!(
		rows.iter()
			.any(|row| row["person"] == json!(BOB) && row["platform"] == "apns")
			&& rows
				.iter()
				.any(|row| row["person"] == json!(ALICE) && row["platform"] == "fcm"),
		"{rows:?}"
	);
}

/// A scheduled send's fanout writes its rows and decides its deliveries
/// right away — with the endpoints' details as they stand then — and both
/// wait for the send's time: the inbox row appears and the matching
/// endpoint is pushed when it's due, and a device whose details change
/// after the fanout keeps the decision the fanout made.
#[tokio::test(flavor = "multi_thread")]
async fn a_scheduled_device_send_decides_its_deliveries_at_the_fanout() {
	let service = service().await;
	let london = id_of(
		&enroll(
			&service,
			ALICE,
			&apns(40),
			reports("en-GB", "Europe/London", "3"),
		)
		.await,
	);
	enroll(
		&service,
		ALICE,
		&fcm(40),
		reports("en-US", "America/New_York", "3"),
	)
	.await;

	let due = chrono::Utc::now() + chrono::Duration::seconds(3);
	let (status, body) = send_condition(
		&service,
		"london-morning",
		"device.time_zone == 'Europe/London'",
		json!({ "visible_at": due.to_rfc3339() }),
		false,
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let progress = settled(&service, "london-morning").await;
	assert_eq!(
		(progress.targeted, progress.delivered),
		(1, 1),
		"{progress:?}"
	);

	// Before it's due: no inbox row yet, and one delivery waiting.
	assert_eq!(inbox_len(&service, ALICE).await, 0, "not yet visible");
	let waiting = deliveries(&service, "london-morning").await;
	assert_eq!(reached(&waiting), vec![(ALICE, london)], "{waiting:?}");
	assert_eq!(waiting[0]["state"], "enqueued");

	// The New York phone moves to London after the fanout: the decision
	// stands.
	enroll(
		&service,
		ALICE,
		&fcm(40),
		reports("en-US", "Europe/London", "3"),
	)
	.await;

	assert!(
		chrono::Utc::now() < due,
		"the checks above ran before the send was due"
	);
	let rows = finished(&service, "london-morning").await;
	assert!(chrono::Utc::now() >= due, "pushed no earlier than its time");
	assert_eq!(reached(&rows), vec![(ALICE, london)]);
	assert_eq!(rows[0]["state"], "accepted", "{rows:?}");
	assert_eq!(inbox_len(&service, ALICE).await, 1, "visible now");
	assert_eq!(service.recorder.sent_to(APNS_HOST), 1);
	assert_eq!(service.recorder.sent_to(FCM_HOST), 0);
}

/// A device detail nothing reports is refused at send time with the keys
/// there are, and so is a platform that isn't one — under `validate_only`
/// as well, which answers exactly what the send would.
#[tokio::test(flavor = "multi_thread")]
async fn unknown_device_keys_are_refused_by_name_even_when_validating() {
	let service = service().await;

	for validate_only in [false, true] {
		for (condition, named) in [
			("device.os == 'ios'", "`device.os` isn't a device detail"),
			("has(device.model)", "`device.model` isn't a device detail"),
			("device.platform == 'ios'", "`device.platform` is one of"),
			("device.app_version >= 'latest'", "isn't one"),
			("!(device.platform == 'fcm')", "no device"),
		] {
			let (status, body) = send_condition(
				&service,
				&format!("bad-{validate_only}"),
				condition,
				json!({}),
				validate_only,
			)
			.await;
			assert_eq!(
				status, 400,
				"{condition} (validate_only {validate_only}): {body}"
			);
			assert!(body.contains(named), "{condition}: {body}");

			if named.contains("isn't a device detail") {
				for key in [
					"device.platform",
					"device.locale",
					"device.language",
					"device.app_version",
					"device.time_zone",
				] {
					assert!(body.contains(key), "{condition}: names {key}: {body}");
				}
			}
		}
	}

	let (status, body) = post(
		&service,
		"send_status",
		json!({ "idempotency_key": "bad-false" }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 404, "a refused send records nothing: {body}");
}

/// The migration that added the details reverses and reapplies on SQLite:
/// down drops the three columns and keeps every endpoint, and up brings
/// them back empty.
#[tokio::test(flavor = "multi_thread")]
async fn the_device_details_migration_reverses_and_reapplies() {
	let db = leptoskit_db::testing::sqlite_migrated(MIGRATIONS).await;
	let last: &str = &MIGRATIONS.last().expect("migrations").name;
	assert!(last.ends_with("_device_details"), "{last}");

	db.execute(Statement {
		sql: format!(
			"INSERT INTO push_endpoints (id, person, platform, credential, generation, \
			 fingerprint, created_at, locale, time_zone, app_version) VALUES ({}, {}, 'apns', \
			 'secret', 1, '{}', '2026-11-18T00:00:00Z', 'en-GB', 'Europe/London', '2.1.0')",
			blob(Uuid::from_u128(1)),
			blob(ALICE),
			"f".repeat(64)
		),
		args: Vec::new(),
	})
	.await
	.expect("an endpoint with details");

	let columns = |db: Db| {
		async move {
			rows(
				&db,
				"SELECT name FROM pragma_table_info('push_endpoints') ORDER BY cid",
			)
			.await
			.iter()
			.map(|row| text(&row[0]))
			.collect::<Vec<_>>()
		}
	};
	let up = columns(db.clone()).await;
	println!("up: {up:?}");
	assert!(up.ends_with(&["locale".into(), "time_zone".into(), "app_version".into()]));

	let reverted = db.rollback(MIGRATIONS, 1).await.expect("down applies");
	println!("down: reverted {reverted:?}");
	assert_eq!(reverted, [last]);
	let down = columns(db.clone()).await;
	println!("down: {down:?}");
	assert!(
		!down
			.iter()
			.any(|column| column == "locale" || column == "time_zone" || column == "app_version")
	);
	assert_eq!(
		text(&rows(&db, "SELECT count(*) FROM push_endpoints").await[0][0]),
		"1",
		"the endpoint survives the reversal"
	);

	let applied = db.migrate(MIGRATIONS).await.expect("up applies again");
	println!("up again: applied {applied:?}");
	assert_eq!(applied, [last]);
	assert_eq!(columns(db.clone()).await, up);
	let row = rows(
		&db,
		"SELECT credential, locale, time_zone, app_version FROM push_endpoints",
	)
	.await;
	assert_eq!(
		row[0].iter().map(text).collect::<Vec<_>>(),
		["secret", "NULL", "NULL", "NULL"],
		"the endpoint is back, its details empty until it re-enrolls"
	);
}
