//! Message options, end to end on a real port: the shape checks the
//! service refuses, silent data messages that push durably without ever
//! surfacing in an inbox or a badge count, the idempotency that counts
//! options as intent, and the app-only test send that reaches a person's
//! live endpoints once each and writes nothing. The transports' wire
//! shapes for each option are in `push.rs`; these are the service-level
//! semantics.

#![cfg(not(target_arch = "wasm32"))]

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use leptoskit_db::Db;
use leptoskit_db::Read as _;
use leptoskit_db::Statement;
use leptoskit_notifications::MIGRATIONS;
use leptoskit_notifications::SERVICE;
use leptoskit_notifications::service::config;
use leptoskit_notifications::service::push;
use leptoskit_notifications::service::transport::Boundary;
use leptoskit_notifications::service::transport::PushRuntime;
use leptoskit_notifications::service::transport::WireError;
use leptoskit_notifications::service::transport::WireRequest;
use leptoskit_notifications::service::transport::WireResponse;
use leptoskit_rpc as rpc;
use ring::rand::SystemRandom;
use ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING;
use ring::signature::EcdsaKeyPair;
use rpc::server::Anonymous;
use rpc::server::ServiceCallers;
use serde_json::Value;
use serde_json::json;
use uuid::Uuid;

const TOKEN: &str = "app-token";
const ORIGIN: &str = "http://app.test";
const PREFIX: &str = "/_leptoskit/services/notifications";

const ALICE: Uuid = Uuid::from_u128(0xA110_0000_0000_0000_0000_0000_0000_0001);
const BOB: Uuid = Uuid::from_u128(0xB0B0_0000_0000_0000_0000_0000_0000_0002);

/// The boundary every attempt answers through, provider-shaped enough
/// for the service-level questions these tests ask: the FCM exchange
/// answers a plain access token, and everything else is a 200.
struct Provider;

impl Boundary for Provider {
	fn post(
		&self,
		request: WireRequest,
	) -> Pin<Box<dyn Future<Output = Result<WireResponse, WireError>> + Send + '_>> {
		Box::pin(async move {
			if request.url.contains("oauth2.googleapis.com") {
				return Ok(WireResponse {
					status: 200,
					headers: Vec::new(),
					body: br#"{"access_token":"ya29.test","expires_in":3600}"#.to_vec(),
				});
			}

			Ok(WireResponse {
				status: 200,
				headers: Vec::new(),
				body: Vec::new(),
			})
		})
	}
}

struct Service {
	address: std::net::SocketAddr,
	db: Db,
	push: push::Push,
	jobs: Option<rpc::server::JobsHandle>,
}

async fn service() -> Service {
	let db = leptoskit_db::testing::sqlite_migrated(MIGRATIONS).await;

	let (key_p8, _) = generated_pem();
	let (_, web) = web_config_for(&key_p8);
	let config = config::PushConfig {
		apns: Some(apns_config_for(key_p8)),
		fcm: Some(fcm_config_for(include_str!(
			"fixtures/test-only-rsa-key.pem"
		))),
		web: Some(web),
	};
	let runtime = PushRuntime::build_over(&config, Arc::new(Provider)).expect("the runtime builds");
	let push = push::Push {
		runtime: Some(Arc::new(runtime)),
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

	Service {
		address,
		db,
		push,
		jobs: None,
	}
}

impl Service {
	/// Starts the workers on a test-fast poll.
	fn resume(&mut self) {
		self.jobs = Some(
			leptoskit_notifications::service::workers_with_push(self.db.clone(), self.push.clone())
				.poll(Duration::from_millis(50))
				.start(),
		);
	}
}

impl Drop for Service {
	fn drop(&mut self) {
		if let Some(jobs) = self.jobs.take() {
			jobs.stop();
		}
	}
}

fn generated_pem() -> (String, Vec<u8>) {
	let rng = SystemRandom::new();
	let document =
		EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).expect("a key");
	let der = document.as_ref().to_vec();
	let begin = "-----BEGIN PRIVATE KEY-----"; // gitleaks:allow (a key made at test time)
	let pem = format!(
		"{begin}\n{}\n-----END PRIVATE KEY-----\n",
		base64::engine::general_purpose::STANDARD.encode(&der)
	);
	(pem, der[73..138].to_vec())
}

fn apns_config_for(key_p8: String) -> config::ApnsConfig {
	config::ApnsConfig {
		key_p8,
		key_id: "KEY123".into(),
		team_id: "TEAM456".into(),
		topic: "dev.leptoskit.app".into(),
		environment: config::ApnsEnvironment::Sandbox,
	}
}

fn fcm_config_for(private_key_pem: &str) -> config::FcmConfig {
	config::FcmConfig {
		service_account_json: json!({
			"project_id": "leptoskit-push",
			"private_key": private_key_pem,
			"client_email": "push@leptoskit-push.iam.gserviceaccount.com",
			"token_uri": "https://evil.example.test/token",
		})
		.to_string(),
	}
}

fn web_config_for(private_key_pem: &str) -> (Vec<u8>, config::WebPushConfig) {
	let pem = private_key_pem.to_owned();
	let der = base64::engine::general_purpose::STANDARD
		.decode(
			pem.lines()
				.skip(1)
				.take_while(|line| !line.starts_with("-----"))
				.collect::<String>(),
		)
		.expect("the PEM decodes");
	(
		der[73..138].to_vec(),
		config::WebPushConfig {
			vapid_private_key: URL_SAFE_NO_PAD.encode(&der[36..68]),
			vapid_public_key: URL_SAFE_NO_PAD.encode(&der[73..138]),
			subject: "mailto:push@leptoskit.dev".into(),
			extra_hosts: Vec::new(),
		},
	)
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

fn spec() -> Value {
	json!({ "category": "news", "title": "It ships", "body": "Push is here." })
}

fn data_spec() -> Value {
	json!({
		"category": "news",
		"title": "Sync",
		"body": "A silent sync nudge.",
		"options": { "kind": "data" },
	})
}

async fn enroll(service: &Service, person: Uuid, credential: Value) -> (u16, String) {
	post(
		service,
		"register_push_endpoint",
		json!({ "credential": credential }),
		Some(&signed_in(person)),
	)
	.await
}

fn apns_credential(token: &str) -> Value {
	json!({ "apns": { "device_token": token } })
}

fn device_token(n: u8) -> String {
	format!("{n:064x}")
}

async fn unread(service: &Service, person: Uuid) -> u64 {
	let (status, body) = post(service, "unread_count", json!({}), Some(&signed_in(person))).await;
	assert_eq!(status, 200, "{body}");
	serde_json::from_str(&body).unwrap_or(0)
}

async fn inbox_count(service: &Service, person: Uuid) -> usize {
	let (status, body) = post(
		service,
		"inbox",
		json!({ "page": { "limit": 100 }, "show": "all" }),
		Some(&signed_in(person)),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let page: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
	page["items"].as_array().map_or(0, Vec::len)
}

async fn count(service: &Service, sql: &str) -> i64 {
	let rows = service
		.db
		.fetch(Statement {
			sql: sql.into(),
			args: Vec::new(),
		})
		.await
		.expect("a read");

	match rows.first().and_then(|row| row.first()) {
		Some(leptoskit_db::Value::Int(count)) => *count,
		other => panic!("a count, not {other:?}"),
	}
}

/// The kitchen-sink options, as an app's server sends them.
fn kitchen_sink() -> Value {
	json!({
		"priority": "normal",
		"image": "https://cdn.example.test/paid.png",
		"sound": { "named": "chime.aiff" },
		"badge": "unread",
		"link": "/invoice/42",
		"android": {
			"channel_id": "inbox",
			"color": "#10b981",
			"icon": "ic_income",
			"tag": "billing",
			"sticky": true,
		},
		"apple": {
			"subtitle": "Invoice #42",
			"thread_id": "billing",
			"category": "BILLING",
			"interruption_level": "time_sensitive",
			"relevance_score": 0.9,
		},
		"web": {
			"icon": "https://cdn.example.test/icon.png",
			"badge_icon": "/badge.png",
			"actions": [{ "action": "open", "title": "Open" }],
			"require_interaction": true,
			"dir": "ltr",
			"lang": "en",
		},
	})
}

/// Options the shared checks refuse, each named precisely by the error.
#[tokio::test(flavor = "multi_thread")]
async fn bad_options_are_refused_with_precise_errors() {
	let service = service().await;

	let refusals = [
		(
			json!({ "image": "http://cdn.example.test/p.png" }),
			"options.image is https",
		),
		(
			json!({ "link": "//evil.test/invoice" }),
			"options.link is on your own origin",
		),
		(
			json!({ "sound": { "named": "no spaces" } }),
			"options.sound",
		),
		(
			json!({ "android": { "color": "10b981" } }),
			"options.android.color is `#RRGGBB`",
		),
		(
			json!({ "apple": { "relevance_score": 1.5 } }),
			"options.apple.relevance_score",
		),
		(
			json!({
				"web": { "actions": [
					{ "action": "a", "title": "A" },
					{ "action": "b", "title": "B" },
					{ "action": "c", "title": "C" },
				] },
			}),
			"options.web.actions carries at most 2",
		),
	];

	for (options, complaint) in refusals {
		let (status, body) = send(
			&service,
			&format!("refused-{complaint}"),
			json!({ "category": "news", "title": "T", "options": options }),
			&[ALICE],
		)
		.await;
		assert_eq!(status, 400, "{complaint}: {body}");
		assert!(body.contains(complaint), "{complaint} is named: {body}");
	}

	// The full set of valid options is accepted — the wire it produces
	// is `push.rs`'s to check.
	let (status, body) = send(
		&service,
		"kitchen-sink",
		json!({
			"category": "billing",
			"title": "Invoice paid",
			"options": kitchen_sink(),
		}),
		&[ALICE],
	)
	.await;
	assert_eq!(status, 200, "{body}");
}

/// A data message shows nothing, so the options that only shape what is
/// shown — an image, a sound, a tap link, and the per-platform
/// presentation blocks — are refused on one, each named. Its priority and
/// its badge, which rides as data for the app to apply, are accepted.
#[tokio::test(flavor = "multi_thread")]
async fn display_options_are_refused_on_a_data_send() {
	let service = service().await;

	for (option, value) in [
		("image", json!("https://cdn.example.test/p.png")),
		("sound", json!("default")),
		("link", json!("/invoice/42")),
		("android", json!({ "channel_id": "inbox" })),
		("apple", json!({ "thread_id": "billing" })),
		("web", json!({ "lang": "en", "actions": [] })),
	] {
		let mut options = json!({ "kind": "data" });
		options[option] = value;
		let (status, body) = send(
			&service,
			&format!("data-{option}"),
			json!({ "category": "news", "title": "Sync", "options": options }),
			&[ALICE],
		)
		.await;
		assert_eq!(status, 400, "{option} on a data send: {body}");
		assert!(
			body.contains(&format!("options.{option}")) && body.contains("data message"),
			"the refusal names {option} and why: {body}"
		);
	}

	let (status, body) = send(
		&service,
		"data-priority-badge",
		json!({
			"category": "news",
			"title": "Sync",
			"options": { "kind": "data", "priority": "high", "badge": { "set": 2 } },
		}),
		&[ALICE],
	)
	.await;
	assert_eq!(status, 200, "{body}");
}

/// A data send pushes durably, is counted as delivered, and never
/// surfaces: no inbox row, no unread count, and a row whose kind is the
/// only trace.
#[tokio::test(flavor = "multi_thread")]
async fn a_data_send_pushes_without_surfacing() {
	let mut service = service().await;
	assert_eq!(
		enroll(&service, ALICE, apns_credential(&device_token(1)))
			.await
			.0,
		200
	);
	service.resume();

	let (status, body) = send(&service, "silent-sync", data_spec(), &[ALICE, BOB]).await;
	assert_eq!(status, 200, "{body}");
	let result: Value = serde_json::from_str(&body).unwrap();
	assert_eq!(
		result["delivered"], 2,
		"the recipients were reached: {result}"
	);
	assert_eq!(result["state"], "complete");

	// Nothing surfaced.
	assert_eq!(inbox_count(&service, ALICE).await, 0);
	assert_eq!(unread(&service, ALICE).await, 0);
	assert_eq!(unread(&service, BOB).await, 0);

	// The durable halves exist: one row per recipient, each a data row.
	assert_eq!(
		count(
			&service,
			"SELECT count(*) FROM notifications WHERE kind = 'data'"
		)
		.await,
		2
	);
	assert_eq!(
		count(
			&service,
			"SELECT count(*) FROM notifications WHERE kind = 'notification'"
		)
		.await,
		0
	);

	// The delivery is real, accepted, and reported like any send's.
	let (status, body) = post(
		&service,
		"push_deliveries",
		json!({ "send_key": "silent-sync", "page": { "limit": 10 } }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let page: Value = serde_json::from_str(&body).unwrap();
	assert_eq!(page["items"].as_array().map(Vec::len), Some(1));
	assert_eq!(page["items"][0]["state"], "accepted", "{page}");

	// And a badge stays a question about notifications: an unread badge
	// on a data send resolves to zero here, because nothing surfaced.
	let (status, body) = send(
		&service,
		"silent-badge",
		json!({
			"category": "news",
			"title": "Sync",
			"options": { "kind": "data", "badge": "unread" },
		}),
		&[ALICE],
	)
	.await;
	assert_eq!(status, 200, "{body}");
	assert_eq!(unread(&service, ALICE).await, 0);
}

/// Data sends keep the send's whole bookkeeping: idempotency by key and
/// intent — options included — status, and a cancel that nothing ever
/// saw through.
#[tokio::test(flavor = "multi_thread")]
async fn data_sends_keep_idempotency_status_and_cancellation() {
	let service = service().await;

	let (status, body) = send(&service, "silent-kept", data_spec(), &[ALICE]).await;
	assert_eq!(status, 200, "{body}");

	// The same key, the same options: the stored answer, marked
	// duplicate.
	let (status, body) = send(&service, "silent-kept", data_spec(), &[ALICE]).await;
	assert_eq!(status, 200, "{body}");
	let repeated: Value = serde_json::from_str(&body).unwrap();
	assert_eq!(repeated["duplicate"], true, "{repeated}");

	// The same key with different options is a different intent.
	let (status, body) = send(
		&service,
		"silent-kept",
		json!({
			"category": "news",
			"title": "Sync",
			"options": { "kind": "data", "priority": "high" },
		}),
		&[ALICE],
	)
	.await;
	assert_eq!(status, 409, "changed options are a conflict: {body}");

	// Status answers the send's state and counts.
	let (status, body) = post(
		&service,
		"send_status",
		json!({ "idempotency_key": "silent-kept" }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let progress: Value = serde_json::from_str(&body).unwrap();
	assert_eq!(progress["state"], "complete");
	assert_eq!(progress["delivered"], 1);

	// A data send is cancellable however long ago it landed: nobody ever
	// saw it, so there is nothing seen to unsay.
	let (status, body) = post(
		&service,
		"cancel",
		json!({ "idempotency_key": "silent-kept" }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "a data send cancels after landing: {body}");
	assert_eq!(body, "1");

	// Cancelling again answers zero, idempotently.
	let (status, body) = post(
		&service,
		"cancel",
		json!({ "idempotency_key": "silent-kept" }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	assert_eq!(body, "0");
}

/// A topic data send fans out exactly as a visible one does, hidden rows
/// and durable deliveries included.
#[tokio::test(flavor = "multi_thread")]
async fn a_topic_data_send_fans_out_invisibly() {
	let mut service = service().await;

	for (person, n) in [(ALICE, 7_u8), (BOB, 8)] {
		assert_eq!(
			enroll(&service, person, apns_credential(&device_token(n)))
				.await
				.0,
			200
		);
	}

	let (status, body) = post(
		&service,
		"subscribe_to_topic",
		json!({ "topic": "sync.all", "persons": [ALICE, BOB] }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");

	service.resume();

	let (status, body) = post(
		&service,
		"send",
		json!({
			"idempotency_key": "topic-silent",
			"spec": data_spec(),
			"target": { "topic": "sync.all" },
		}),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let result: Value = serde_json::from_str(&body).unwrap();
	assert_eq!(result["state"], "fanning_out", "{result}");

	// The fanout walks the audience and lands hidden rows, and the
	// deliveries accept.
	for _ in 0..300 {
		if count(
			&service,
			"SELECT count(*) FROM notifications WHERE kind = 'data'",
		)
		.await
			== 2
			&& count(
				&service,
				"SELECT count(*) FROM push_deliveries WHERE state = 'accepted'",
			)
			.await
				== 2
		{
			break;
		}

		tokio::time::sleep(Duration::from_millis(50)).await;
	}

	assert_eq!(
		count(
			&service,
			"SELECT count(*) FROM notifications WHERE kind = 'data'"
		)
		.await,
		2,
		"both subscribers hold a durable half"
	);
	assert_eq!(
		count(
			&service,
			"SELECT count(*) FROM push_deliveries WHERE state = 'accepted'"
		)
		.await,
		2
	);
	assert_eq!(inbox_count(&service, ALICE).await, 0);
	assert_eq!(unread(&service, BOB).await, 0);
}

/// The test send reaches one person's live endpoints once each, answers
/// per endpoint, and writes nothing anywhere.
#[tokio::test(flavor = "multi_thread")]
async fn a_test_send_probes_once_and_writes_nothing() {
	let mut service = service().await;

	for n in 1..=3_u8 {
		assert_eq!(
			enroll(&service, ALICE, apns_credential(&device_token(n)))
				.await
				.0,
			200
		);
	}

	// Only the app tests sends.
	for token in [None, Some("not-the-token"), Some(signed_in(ALICE).as_str())] {
		let (status, body) = post(
			&service,
			"send_test",
			json!({ "spec": spec(), "person": ALICE }),
			token,
		)
		.await;
		assert_eq!(status, 403, "only the app tests: {body}");
	}

	service.resume();

	let (status, body) = post(
		&service,
		"send_test",
		json!({ "spec": spec(), "person": ALICE }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let outcomes: Vec<Value> = serde_json::from_str::<Value>(&body)
		.unwrap()
		.as_array()
		.cloned()
		.unwrap_or_default();
	assert_eq!(
		outcomes.len(),
		3,
		"one outcome per live endpoint: {outcomes:?}"
	);

	for outcome in &outcomes {
		assert_eq!(outcome["state"], "accepted", "{outcome}");
		assert_eq!(outcome["platform"], "apns");
		assert!(outcome["reason"].is_null());
		assert!(outcome["endpoint"].as_str().is_some());
	}

	// Nothing was written: no inbox row, no delivery, no send.
	assert_eq!(
		count(&service, "SELECT count(*) FROM notifications").await,
		0
	);
	assert_eq!(
		count(&service, "SELECT count(*) FROM push_deliveries").await,
		0
	);
	assert_eq!(count(&service, "SELECT count(*) FROM sends").await, 0);
	assert_eq!(unread(&service, ALICE).await, 0);

	// One endpoint probes one device.
	let (status, body) = post(
		&service,
		"push_endpoints",
		json!({ "page": { "limit": 10 } }),
		Some(&signed_in(ALICE)),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let page: Value = serde_json::from_str(&body).unwrap();
	let one = page["items"][0]["id"].clone();

	let (status, body) = post(
		&service,
		"send_test",
		json!({ "spec": spec(), "person": ALICE, "endpoint": one }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let outcomes: Vec<Value> = serde_json::from_str::<Value>(&body)
		.unwrap()
		.as_array()
		.cloned()
		.unwrap_or_default();
	assert_eq!(outcomes.len(), 1, "{outcomes:?}");

	// An id nobody holds — or another person's endpoint — is not found.
	let (status, _) = post(
		&service,
		"send_test",
		json!({
			"spec": spec(),
			"person": ALICE,
			"endpoint": Uuid::now_v7(),
		}),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 404);

	// A bad spec is refused exactly as a send's is, and a test send is
	// bounded by the person's live endpoints however it is asked.
	let (status, body) = post(
		&service,
		"send_test",
		json!({
			"spec": { "category": "news", "title": "T", "options": { "sound": { "named": "no spaces" } } },
			"person": ALICE,
		}),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 400, "{body}");
}

/// A test send on a server without push configured says so, and one to
/// a person with no endpoints answers an empty list.
#[tokio::test(flavor = "multi_thread")]
async fn a_test_send_needs_transports_and_audience() {
	let db = leptoskit_db::testing::sqlite_migrated(MIGRATIONS).await;
	let callers = ServiceCallers::new(Anonymous)
		.caller(TOKEN, rpc::server::APP_CALLER)
		.forwarded_identity(SERVICE, TOKEN);
	let (router, live) = leptoskit_notifications::service::routes_with_push(
		db.clone(),
		Some(ORIGIN),
		callers,
		push::Push { runtime: None },
	);
	live.start().await.expect("live starts");
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	tokio::spawn(async move {
		leptoskit_axum::axum::serve(listener, router).await.unwrap();
	});
	let bare = Service {
		address,
		db,
		push: push::Push { runtime: None },
		jobs: None,
	};

	let (status, body) = post(
		&bare,
		"send_test",
		json!({ "spec": spec(), "person": ALICE }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 400, "no transports, no test: {body}");
	assert!(body.contains("not configured"), "{body}");

	// A configured service, a person with nothing enrolled: the honest
	// empty answer.
	let configured = service().await;
	let (status, body) = post(
		&configured,
		"send_test",
		json!({ "spec": spec(), "person": BOB }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	assert_eq!(body, "[]", "nothing to probe");
}

/// The FCM exchange a test send drives is a real one, and its access
/// token is refused by nobody here: the probe answers through the
/// boundary and reports an operator-shaped failure as its own state.
#[tokio::test(flavor = "multi_thread")]
async fn a_test_send_reports_each_endpoint_once() {
	let service = service().await;
	assert_eq!(
		enroll(
			&service,
			ALICE,
			json!({ "fcm": { "registration_token": "probe-token" } })
		)
		.await
		.0,
		200
	);

	let (status, body) = post(
		&service,
		"send_test",
		json!({
			"spec": json!({
				"category": "news",
				"title": "Probe",
				"options": { "priority": "high" },
			}),
			"person": ALICE,
		}),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let outcomes: Vec<Value> = serde_json::from_str::<Value>(&body)
		.unwrap()
		.as_array()
		.cloned()
		.unwrap_or_default();
	assert_eq!(outcomes.len(), 1, "{outcomes:?}");
	assert_eq!(outcomes[0]["state"], "accepted", "{outcomes:?}");

	// A data test send is silent too: the same probe, shaped.
	let (status, body) = post(
		&service,
		"send_test",
		json!({
			"spec": json!({
				"category": "news",
				"title": "Probe",
				"options": { "kind": "data" },
			}),
			"person": ALICE,
		}),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let outcomes: Vec<Value> = serde_json::from_str::<Value>(&body)
		.unwrap()
		.as_array()
		.cloned()
		.unwrap_or_default();
	assert_eq!(outcomes[0]["state"], "accepted", "{outcomes:?}");
	assert_eq!(
		count(&service, "SELECT count(*) FROM notifications").await,
		0,
		"a data test writes no row either"
	);
}
