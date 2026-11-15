//! Push delivery, end to end on a real port: enrollment's authentication
//! and bounds, cross-account refusal, owner-only revocation, the fanout a
//! send makes, the wire each transport puts on the boundary — signed
//! tokens, exchanged credentials, encrypted bodies — transient retries
//! that recover, permanent failures that end and invalidate, oversized
//! payloads that end clearly without a call, schedule, cancellation,
//! expiry, and opt-out decided at the attempt, recovery across a
//! restart, idempotent duplicate sends, and where the credentials never
//! appear.
//!
//! The transports are exercised over an injected boundary: each test
//! installs a mock that receives the real provider requests and answers
//! a scripted response. The URLs, the headers, the payloads, and the
//! host policy are the production path's own; only the socket differs.
//! Tests serialize on one lock, because the installed runtime is the
//! process's.

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
use ring::signature::ECDSA_P256_SHA256_FIXED;
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

const APNS_HOST: &str = "api.sandbox.push.apple.com";
const FCM_TOKEN_HOST: &str = "oauth2.googleapis.com";
const FCM_HOST: &str = "fcm.googleapis.com";
const WEB_HOST: &str = "updates.push.services.mozilla.com";

/// The one lock every test here holds for its body: the installed
/// runtime is the process's, so the tests take turns.
static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// What the boundary received, for the tests to inspect.
#[derive(Clone, Debug)]
struct Seen {
	url: String,
	headers: Vec<(String, String)>,
	body: Vec<u8>,
}

impl Seen {
	fn header(&self, name: &str) -> Option<&str> {
		self.headers
			.iter()
			.find(|(header, _)| header == name)
			.map(|(_, value)| value.as_str())
	}

	fn json(&self) -> Value {
		serde_json::from_slice(&self.body).unwrap_or(Value::Null)
	}
}

/// A scripted answer: a status, headers, and a body — or a socket
/// failure. `Err(None)` says nothing special was scripted, and the mock
/// answers success.
type Answer = Result<(u16, Vec<(String, String)>, String), Option<WireError>>;

/// The test's script: one request and its host's request count in,
/// an answer out.
type Script = Box<dyn Fn(&WireRequest, usize) -> Answer + Send + Sync>;

/// The mock boundary: it records every request, counts requests per
/// host, and answers through the test's script.
struct Mock {
	seen: std::sync::Mutex<Vec<Seen>>,
	counts: std::sync::Mutex<std::collections::HashMap<String, usize>>,
	script: std::sync::RwLock<Script>,
}

impl Mock {
	fn new() -> Arc<Self> {
		Arc::new(Self {
			seen: std::sync::Mutex::new(Vec::new()),
			counts: std::sync::Mutex::new(std::collections::HashMap::new()),
			script: std::sync::RwLock::new(Box::new(|_, _| Err(None))),
		})
	}

	/// Answers future requests through `script`, which sees the request
	/// and how many requests its host has had before this one.
	fn scripted(&self, script: impl Fn(&WireRequest, usize) -> Answer + Send + Sync + 'static) {
		*self.script.write().unwrap() = Box::new(script);
	}

	fn requests(&self) -> Vec<Seen> {
		self.seen.lock().unwrap().clone()
	}

	fn requests_to(&self, host: &str) -> Vec<Seen> {
		self.requests()
			.into_iter()
			.filter(|seen| seen.url.contains(host))
			.collect()
	}
}

impl Boundary for Mock {
	fn post(
		&self,
		request: WireRequest,
	) -> Pin<Box<dyn Future<Output = Result<WireResponse, WireError>> + Send + '_>> {
		let host = request
			.url
			.split("//")
			.nth(1)
			.and_then(|rest| rest.split('/').next())
			.unwrap_or_default()
			.to_owned();
		let index = {
			let mut counts = self.counts.lock().unwrap();
			let count = counts.entry(host).or_insert(0);
			*count += 1;
			*count - 1
		};
		self.seen.lock().unwrap().push(Seen {
			url: request.url.clone(),
			headers: request.headers.clone(),
			body: request.body.clone(),
		});

		let scripted = (self.script.read().unwrap())(&request, index);

		Box::pin(async move {
			match scripted {
				Ok((status, headers, body)) => {
					Ok(WireResponse {
						status,
						headers,
						body: body.into_bytes(),
					})
				}
				Err(Some(error)) => Err(error),
				Err(None) => {
					Ok(WireResponse {
						status: 200,
						headers: Vec::new(),
						body: Vec::new(),
					})
				}
			}
		})
	}
}

struct Service {
	address: std::net::SocketAddr,
	db: Db,
	jobs: Option<rpc::server::JobsHandle>,
}

/// A fresh, migrated service on a free port, its push transports built
/// over `mock`, its workers not yet running: tests start them when the
/// queue should move.
async fn start(db: Db, config: &config::PushConfig, mock: &Arc<Mock>) -> Service {
	let runtime = PushRuntime::build_over(config, mock.clone()).expect("the runtime builds");
	push::install(Some(runtime));

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

	Service {
		address,
		db,
		jobs: None,
	}
}

impl Service {
	/// Starts the workers on a test-fast poll.
	fn resume(&mut self) {
		self.jobs = Some(
			leptoskit_notifications::service::workers(self.db.clone())
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

/// The lock's guard, held so the other tests here wait. The lock is a
/// static, so the guard's lifetime is the process's.
type Turn = tokio::sync::MutexGuard<'static, ()>;

async fn service(mock: &Arc<Mock>, config: &config::PushConfig) -> (Turn, Service) {
	let turn = TURN.lock().await;
	let db = leptoskit_db::testing::sqlite_migrated(MIGRATIONS).await;
	let started = start(db, config, mock).await;
	(turn, started)
}

/// All three transports, over keys nobody verifies.
fn all_transports() -> config::PushConfig {
	config::PushConfig {
		apns: Some(config::ApnsConfig {
			key_p8: generated_pem().0,
			key_id: "KEY123".into(),
			team_id: "TEAM456".into(),
			topic: "dev.leptoskit.app".into(),
			environment: config::ApnsEnvironment::Sandbox,
		}),
		fcm: Some(fcm_config_for(&generated_pem().0)),
		web: Some(web_config_for(&generated_pem().0).1),
	}
}

/// A generated `.p8` PEM and the public point it carries.
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
	// The public point sits at offset 73 in the PKCS#8 template.
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
			"type": "service_account",
			"project_id": "leptoskit-push",
			"private_key": private_key_pem,
			"client_email": "push@leptoskit-push.iam.gserviceaccount.com",
		})
		.to_string(),
	}
}

/// A VAPID pair from a PEM: the config and the public key to verify with.
fn web_config_for(private_key_pem: &str) -> (Vec<u8>, config::WebPushConfig) {
	let der = pem_der(private_key_pem);
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

/// The DER inside a PEM.
fn pem_der(pem: &str) -> Vec<u8> {
	base64::engine::general_purpose::STANDARD
		.decode(
			pem.lines()
				.skip(1)
				.take_while(|line| !line.starts_with("-----"))
				.collect::<String>(),
		)
		.expect("the PEM decodes")
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
			"recipients": recipients,
		}),
		Some(TOKEN),
	)
	.await
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

fn fcm_credential(token: &str) -> Value {
	json!({ "fcm": { "registration_token": token } })
}

fn web_credential(endpoint: &str) -> Value {
	json!({
		"web_push": {
			"endpoint": endpoint,
			"keys": {
				"p256dh": web_public_key(),
				"auth": URL_SAFE_NO_PAD.encode([7_u8; 16]),
			},
		}
	})
}

/// A browser subscription's public key: a real P-256 point, so the
/// transport's encryption runs its full ECDH.
fn web_public_key() -> String {
	let rng = SystemRandom::new();
	let private = ring::agreement::EphemeralPrivateKey::generate(&ring::agreement::ECDH_P256, &rng)
		.expect("a key");
	URL_SAFE_NO_PAD.encode(private.compute_public_key().expect("a point").as_ref())
}

async fn endpoints_of(service: &Service, person: Uuid) -> Vec<Value> {
	let (status, body) = post(
		service,
		"push_endpoints",
		json!({}),
		Some(&signed_in(person)),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	serde_json::from_str(&body).unwrap_or_default()
}

async fn delivery_status(service: &Service, send_key: &str) -> Vec<Value> {
	let (code, body) = post(
		service,
		"push_deliveries",
		json!({ "send_key": send_key, "page": { "limit": 100 } }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(code, 200, "{body}");
	let page: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
	page["items"].as_array().cloned().unwrap_or_default()
}

/// Waits until every delivery of `send_key` has left the queue, and
/// answers their states and reasons.
async fn settled(service: &Service, send_key: &str) -> Vec<(String, Option<String>)> {
	for _ in 0..300 {
		let states = delivery_status(service, send_key)
			.await
			.into_iter()
			.map(|item| {
				(
					item["state"].as_str().unwrap_or_default().to_owned(),
					item["reason"].as_str().map(str::to_owned),
				)
			})
			.collect::<Vec<_>>();

		let settled = !states.is_empty()
			&& states
				.iter()
				.all(|(state, _)| *state != "enqueued" && *state != "retrying");

		if settled {
			return states;
		}

		tokio::time::sleep(Duration::from_millis(50)).await;
	}

	let last = delivery_status(service, send_key).await;
	panic!("the deliveries of {send_key} never settled: {last:?}");
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
		leptoskit_db::Value::Int(int) => int.to_string(),
		other => format!("{other:?}"),
	}
}

/// Parses an ES256 JWT and verifies its signature against `point`,
/// answering its header and claims.
fn verify_jwt(token: &str, point: &[u8]) -> (Value, Value) {
	let parts: Vec<&str> = token.split('.').collect();
	assert_eq!(parts.len(), 3, "a JWT has three parts: {token}");
	let signing = format!("{}.{}", parts[0], parts[1]);
	let signature = URL_SAFE_NO_PAD
		.decode(parts[2])
		.expect("the signature is base64url");
	ring::signature::UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, point)
		.verify(signing.as_bytes(), &signature)
		.expect("the JWT's signature verifies");
	let header: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[0]).unwrap()).unwrap();
	let claims: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
	(header, claims)
}

/// A request header's value, from the wire shape the boundary sees.
fn wire_header<'a>(request: &'a WireRequest, name: &str) -> Option<&'a str> {
	request
		.headers
		.iter()
		.find(|(header, _)| header == name)
		.map(|(_, value)| value.as_str())
}

/// A 64-hex device token, as a device holds one.
fn device_token(n: u8) -> String {
	format!("{n:064x}")
}

fn spec() -> Value {
	json!({ "category": "news", "title": "It ships", "body": "Push is here." })
}

// Enrollment.

#[tokio::test(flavor = "multi_thread")]
async fn enrollment_is_authenticated_and_bounded() {
	let mock = Mock::new();
	let (_turn, service) = service(&mock, &all_transports()).await;

	// Nobody enrolls without a forwarded person.
	let (status, _) = post(
		&service,
		"register_push_endpoint",
		json!({ "credential": apns_credential(&device_token(1)) }),
		None,
	)
	.await;
	assert_eq!(status, 403, "an anonymous caller can't enroll");

	let (status, _) = post(
		&service,
		"register_push_endpoint",
		json!({ "credential": apns_credential(&device_token(1)) }),
		Some("not-the-token"),
	)
	.await;
	assert_eq!(status, 403, "nor a caller with the wrong token");

	// A bad credential is refused on its shape.
	for (name, credential) in [
		("a short token", apns_credential("abc")),
		(
			"a token with a non-hex character",
			apns_credential(&format!("{}g", "0".repeat(63))),
		),
		("a blank fcm token", fcm_credential("   ")),
		("a token with a space", fcm_credential("abc def")),
	] {
		let (status, _) = enroll(&service, ALICE, credential).await;
		assert_eq!(status, 400, "{name} is refused");
	}

	// The happy path answers an opaque id, with no credential in it.
	let (status, body) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200, "{body}");
	let endpoint: Value = serde_json::from_str(&body).unwrap();
	assert_eq!(endpoint["platform"], "apns");
	assert!(endpoint["id"].as_str().is_some());
	assert!(
		!serde_json::to_string(&endpoint)
			.unwrap()
			.contains(&device_token(1)),
		"the answer carries no credential"
	);

	// The same credential again answers the same id: idempotent.
	let (status, again) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200, "{again}");
	let again: Value = serde_json::from_str(&again).unwrap();
	assert_eq!(
		again["id"], endpoint["id"],
		"re-registration is the same endpoint"
	);

	// Upper-case hex is the same credential.
	let (status, upper) = enroll(
		&service,
		ALICE,
		apns_credential(&device_token(1).to_uppercase()),
	)
	.await;
	assert_eq!(status, 200, "{upper}");
	let upper: Value = serde_json::from_str(&upper).unwrap();
	assert_eq!(
		upper["id"], endpoint["id"],
		"hex case doesn't make a second endpoint"
	);

	// The bound: ten live endpoints, the eleventh refused.
	for n in 2..=10_u8 {
		let (status, body) = enroll(&service, ALICE, apns_credential(&device_token(n))).await;
		assert_eq!(status, 200, "{body}");
	}

	let (status, _) = enroll(&service, ALICE, apns_credential(&device_token(11))).await;
	assert_eq!(status, 400, "the eleventh endpoint is refused");

	// Lists are personal and credential-free.
	let alice = endpoints_of(&service, ALICE).await;
	assert_eq!(alice.len(), 10);
	let bob = endpoints_of(&service, BOB).await;
	assert_eq!(bob.len(), 0, "nobody else's endpoints are in the list");

	let listed = serde_json::to_string(&alice).unwrap();
	assert!(
		!listed.contains(&device_token(1)),
		"the list carries no credential: {listed}"
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_platform_without_a_transport_is_refused_loudly() {
	let mock = Mock::new();
	let config = config::PushConfig {
		apns: Some(apns_config_for(generated_pem().0)),
		fcm: None,
		web: None,
	};
	let (_turn, service) = service(&mock, &config).await;

	let (status, body) = enroll(&service, ALICE, fcm_credential("a-token")).await;
	assert_eq!(status, 400, "{body}");
	assert!(
		body.contains("not configured"),
		"the refusal says the transport is missing: {body}"
	);

	// And with nothing installed at all, so is the configured one.
	push::install(None);
	let (status, _) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 400, "with push disabled, nothing enrolls");
	push::install(None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_credential_of_another_account_is_refused_not_reassigned() {
	let mock = Mock::new();
	let (_turn, service) = service(&mock, &all_transports()).await;

	let (status, body) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200, "{body}");
	let alice_endpoint: Value = serde_json::from_str(&body).unwrap();

	// Bob presents the same credential: refused, with no reassignment.
	let (status, body) = enroll(&service, BOB, apns_credential(&device_token(1))).await;
	assert_eq!(status, 409, "{body}");
	assert!(
		!body.contains(&device_token(1)),
		"the refusal carries no credential: {body}"
	);

	let alice_now = endpoints_of(&service, ALICE).await;
	assert_eq!(alice_now.len(), 1, "nothing moved");
	assert_eq!(alice_now[0]["id"], alice_endpoint["id"]);

	// Alice revokes — what signing out of the app does — and then the
	// credential is free for bob's device.
	let (status, _) = post(
		&service,
		"revoke_push_endpoint",
		json!({ "endpoint": alice_endpoint["id"] }),
		Some(&signed_in(ALICE)),
	)
	.await;
	assert_eq!(status, 200);

	let (status, body) = enroll(&service, BOB, apns_credential(&device_token(1))).await;
	assert_eq!(
		status, 200,
		"after revocation the credential enrolls: {body}"
	);

	let bob_now = endpoints_of(&service, BOB).await;
	assert_eq!(bob_now.len(), 1);
	assert_ne!(
		bob_now[0]["id"], alice_endpoint["id"],
		"a new endpoint, not the old one"
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn revocation_is_owner_only_and_ends_queued_deliveries() {
	let mock = Mock::new();
	let (_turn, service) = service(&mock, &all_transports()).await;

	let (status, body) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200, "{body}");
	let endpoint: Value = serde_json::from_str(&body).unwrap();

	let (status, _) = send(&service, "queued", spec(), &[ALICE]).await;
	assert_eq!(status, 200);

	// Bob can't revoke alice's endpoint; her id is not found for him.
	let (status, _) = post(
		&service,
		"revoke_push_endpoint",
		json!({ "endpoint": endpoint["id"] }),
		Some(&signed_in(BOB)),
	)
	.await;
	assert_eq!(status, 404, "bob sees no endpoint of alice's");

	let (status, body) = post(
		&service,
		"revoke_push_endpoint",
		json!({ "endpoint": endpoint["id"] }),
		Some(&signed_in(ALICE)),
	)
	.await;
	assert_eq!(status, 200, "{body}");

	let alice = endpoints_of(&service, ALICE).await;
	assert_eq!(alice.len(), 0, "revocation deletes the endpoint");

	// The queued delivery ended as revoked without an attempt.
	let outcomes = settled(&service, "queued").await;
	assert_eq!(
		outcomes,
		vec![("skipped".to_owned(), Some("endpoint_revoked".to_owned()))],
		"{outcomes:?}"
	);
	assert!(
		mock.requests_to(APNS_HOST).is_empty(),
		"nothing went to the provider"
	);
}

// The app's status query.

#[tokio::test(flavor = "multi_thread")]
async fn only_the_app_reads_delivery_status() {
	let mock = Mock::new();
	let (_turn, mut service) = service(&mock, &all_transports()).await;

	let (status, _) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200);
	service.resume();
	let (status, _) = send(&service, "readable", spec(), &[ALICE]).await;
	assert_eq!(status, 200);
	settled(&service, "readable").await;

	for (token, name) in [
		(None, "an anonymous call"),
		(Some("not-the-token"), "a wrong token"),
		(Some(signed_in(ALICE).as_str()), "the recipient's own token"),
	] {
		let (status, _) = post(
			&service,
			"push_deliveries",
			json!({ "send_key": "readable", "page": { "limit": 10 } }),
			token,
		)
		.await;
		assert_eq!(status, 403, "{name} reads nobody's status");
	}

	let items = delivery_status(&service, "readable").await;
	assert_eq!(items.len(), 1);
	let item = &items[0];
	assert_eq!(item["state"], "accepted");
	assert_eq!(item["platform"], "apns");
	assert_eq!(item["person"], ALICE.to_string());
	assert!(item["endpoint"].as_str().is_some());
	let shown = serde_json::to_string(&items).unwrap();
	assert!(
		!shown.contains(&device_token(1)),
		"the status carries no credential: {shown}"
	);

	let (code, body) = post(
		&service,
		"push_deliveries",
		json!({ "send_key": "unknown-send", "page": { "limit": 10 } }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(code, 200, "{body}");
	let page: Value = serde_json::from_str(&body).unwrap();
	assert_eq!(page["items"].as_array().map(Vec::len), Some(0));
}

// The fanout and the wire.

#[tokio::test(flavor = "multi_thread")]
async fn a_send_fans_out_one_delivery_per_endpoint() {
	let mock = Mock::new();
	let (_turn, mut service) = service(&mock, &all_transports()).await;

	for (person, base) in [(ALICE, 1_u8), (BOB, 3)] {
		for n in base..base + 2 {
			let (status, body) = enroll(&service, person, apns_credential(&device_token(n))).await;
			assert_eq!(status, 200, "{body}");
		}
	}

	service.resume();

	let (status, body) = send(&service, "fanout", spec(), &[ALICE, BOB]).await;
	assert_eq!(status, 200, "{body}");

	let outcomes = settled(&service, "fanout").await;
	assert_eq!(outcomes.len(), 4, "one delivery per endpoint: {outcomes:?}");
	assert!(
		outcomes
			.iter()
			.all(|(state, reason)| state == "accepted" && reason.is_none()),
		"{outcomes:?}"
	);

	let stored = rows(&service, "SELECT count(*) FROM push_deliveries").await;
	assert_eq!(text(&stored[0][0]), "4", "and no more than one per pair");

	// The wire: one POST per endpoint to APNs, with the provider's shape.
	let sent = mock.requests_to(APNS_HOST);
	assert_eq!(sent.len(), 4, "one request per endpoint");

	for seen in &sent {
		assert!(
			seen.url
				.starts_with(&format!("https://{APNS_HOST}/3/device/")),
			"{}",
			seen.url
		);
		assert_eq!(seen.header("apns-topic"), Some("dev.leptoskit.app"));
		assert_eq!(seen.header("apns-push-type"), Some("alert"));
		assert!(
			seen.header("authorization")
				.is_some_and(|value| value.starts_with("Bearer "))
		);
		let payload = seen.json();
		assert_eq!(payload["aps"]["alert"]["title"], "It ships");
		assert_eq!(payload["aps"]["alert"]["body"], "Push is here.");
		assert!(
			payload["lkn"].as_str().is_some(),
			"the id rides in the payload"
		);
	}

	// Collapse ids match the notifications they carry.
	let notifications: Vec<String> = rows(
		&service,
		"SELECT lower(hex(id)) FROM notifications WHERE send_key = 'fanout'",
	)
	.await
	.into_iter()
	.map(|row| text(&row[0]))
	.collect();

	for seen in &sent {
		let id = seen.header("apns-collapse-id").expect("a collapse id");
		let id = Uuid::parse_str(id).expect("the collapse id is a UUID");
		assert!(
			notifications.contains(&id.simple().to_string()),
			"{id} is one of the send's notifications"
		);
	}
}

#[tokio::test(flavor = "multi_thread")]
async fn the_apns_wire_carries_a_verified_cached_provider_token() {
	let mock = Mock::new();
	let (key_p8, point) = generated_pem();
	let config = config::PushConfig {
		apns: Some(apns_config_for(key_p8)),
		fcm: None,
		web: None,
	};
	let (_turn, mut service) = service(&mock, &config).await;

	let (status, _) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200);
	service.resume();

	let (status, _) = send(&service, "signed", spec(), &[ALICE]).await;
	assert_eq!(status, 200);
	settled(&service, "signed").await;
	let (status, _) = send(&service, "signed-again", spec(), &[ALICE]).await;
	assert_eq!(status, 200);
	settled(&service, "signed-again").await;

	let sent = mock.requests_to(APNS_HOST);
	assert_eq!(sent.len(), 2, "two pushes, two requests");

	for seen in &sent {
		let bearer = seen
			.header("authorization")
			.expect("a bearer token")
			.strip_prefix("Bearer ")
			.expect("Bearer");
		let (header, claims) = verify_jwt(bearer, &point);
		assert_eq!(header["alg"], "ES256");
		assert_eq!(header["kid"], "KEY123");
		assert_eq!(claims["iss"], "TEAM456");
		assert!(claims["iat"].as_i64().is_some());
	}

	assert_eq!(
		sent[0].header("authorization"),
		sent[1].header("authorization"),
		"the cached provider token serves both pushes"
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_fcm_wire_exchanges_a_signed_account_jwt_for_a_token() {
	let mock = Mock::new();
	let (key_pem, point) = generated_pem();
	let config = config::PushConfig {
		apns: None,
		fcm: Some(fcm_config_for(&key_pem)),
		web: None,
	};
	let (_turn, mut service) = service(&mock, &config).await;

	let (status, _) = enroll(&service, ALICE, fcm_credential("fcm-token-1")).await;
	assert_eq!(status, 200);
	service.resume();

	// The token endpoint answers a short-lived access token.
	let verifying = point.clone();
	mock.scripted(move |request, _| {
		if !request.url.contains(FCM_TOKEN_HOST) {
			return Err(None);
		}

		assert!(
			wire_header(request, "content-type")
				.is_some_and(|value| value.contains("application/x-www-form-urlencoded"))
		);
		let form = String::from_utf8(request.body.clone()).unwrap();
		assert!(
			form.contains("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer"),
			"the JWT-bearer grant: {form}"
		);
		let assertion = form
			.split("assertion=")
			.nth(1)
			.expect("an assertion")
			.split('&')
			.next()
			.expect("the assertion value");
		let (header, claims) = verify_jwt(assertion, &verifying);
		assert_eq!(header["alg"], "ES256");
		assert_eq!(claims["iss"], "push@leptoskit-push.iam.gserviceaccount.com");
		assert_eq!(claims["aud"], "https://oauth2.googleapis.com/token");
		assert_eq!(
			claims["scope"],
			"https://www.googleapis.com/auth/firebase.messaging"
		);

		Ok((
			200,
			vec![("content-type".to_owned(), "application/json".to_owned())],
			json!({ "access_token": "ya29.test-token", "expires_in": 3600 }).to_string(),
		))
	});

	let (status, _) = send(&service, "fcm-sent", spec(), &[ALICE]).await;
	assert_eq!(status, 200);
	settled(&service, "fcm-sent").await;

	let sent = mock.requests_to(FCM_HOST);
	assert_eq!(sent.len(), 1, "one send request");
	assert_eq!(
		sent[0].url,
		"https://fcm.googleapis.com/v1/projects/leptoskit-push/messages:send"
	);
	assert_eq!(
		sent[0].header("authorization"),
		Some("Bearer ya29.test-token")
	);
	let message = sent[0].json();
	assert_eq!(message["message"]["token"], "fcm-token-1");
	assert_eq!(message["message"]["notification"]["title"], "It ships");
	assert_eq!(message["message"]["notification"]["body"], "Push is here.");
	assert_eq!(
		message["message"]["android"]["collapse_key"], message["message"]["data"]["lkn"],
		"the collapse key is the notification's id"
	);

	// The access token is cached: a second send doesn't re-exchange.
	let exchanges_before = mock.requests_to(FCM_TOKEN_HOST).len();
	let (status, _) = send(&service, "fcm-again", spec(), &[ALICE]).await;
	assert_eq!(status, 200);
	settled(&service, "fcm-again").await;
	assert_eq!(
		mock.requests_to(FCM_TOKEN_HOST).len(),
		exchanges_before,
		"the cached access token serves the second send"
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_web_push_wire_carries_vapid_and_an_encrypted_body() {
	let mock = Mock::new();
	let (key_pem, _) = generated_pem();
	let (vapids, web) = web_config_for(&key_pem);
	let config = config::PushConfig {
		apns: None,
		fcm: None,
		web: Some(web),
	};
	let (_turn, mut service) = service(&mock, &config).await;

	let endpoint = format!("https://{WEB_HOST}/push/abcdef123456");
	let (status, body) = enroll(&service, ALICE, web_credential(&endpoint)).await;
	assert_eq!(status, 200, "{body}");
	service.resume();

	let (status, _) = send(&service, "encrypted", spec(), &[ALICE]).await;
	assert_eq!(status, 200);
	settled(&service, "encrypted").await;

	let sent = mock.requests_to(WEB_HOST);
	assert_eq!(sent.len(), 1);
	assert_eq!(
		sent[0].url, endpoint,
		"the endpoint is pushed to, not rewritten"
	);
	assert_eq!(sent[0].header("content-encoding"), Some("aes128gcm"));
	assert_eq!(
		sent[0].header("content-type"),
		Some("application/octet-stream")
	);
	assert!(sent[0].header("ttl").is_some());
	assert_eq!(
		sent[0].header("topic").expect("a topic").len(),
		32,
		"a topic is a UUID without its dashes"
	);

	// The VAPID header: `vapid t=<JWT>, k=<public key>`, aud the
	// endpoint's origin, sub the operator's contact, exp ahead, and the
	// k half matching the configured public key.
	let vapid = sent[0]
		.header("authorization")
		.expect("vapid")
		.strip_prefix("vapid ")
		.expect("the scheme");
	let (token, public) = vapid.split_once(", k=").expect("t= and k= halves");
	let token = token.strip_prefix("t=").expect("the token half");
	let (_, claims) = verify_jwt(token, &vapids);
	assert_eq!(claims["aud"], format!("https://{WEB_HOST}"));
	assert_eq!(claims["sub"], "mailto:push@leptoskit.dev");
	assert!(claims["exp"].as_i64().unwrap() > leptoskit_db::now().timestamp());
	assert_eq!(
		URL_SAFE_NO_PAD.decode(public).unwrap(),
		vapids,
		"k= is the configured public key"
	);

	// The body: an aes128gcm header with the sender's 65-byte point as
	// its key id, and one record that fits the service's limit.
	let body = &sent[0].body;
	assert!(body.len() <= 4_096, "{} bytes", body.len());
	assert_eq!(&body[16..20], &4096_u32.to_be_bytes(), "one 4 KiB record");
	assert_eq!(body[21], 0x04, "the key id is an uncompressed point");
}

#[tokio::test(flavor = "multi_thread")]
async fn every_transport_on_one_send_reaches_its_own_provider() {
	let mock = Mock::new();
	let (_turn, mut service) = service(&mock, &all_transports()).await;

	let (status, _) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200);
	let (status, _) = enroll(&service, ALICE, fcm_credential("fcm-token-1")).await;
	assert_eq!(status, 200);
	let endpoint = format!("https://{WEB_HOST}/push/abcdef123456");
	let (status, _) = enroll(&service, ALICE, web_credential(&endpoint)).await;
	assert_eq!(status, 200);

	// The FCM transport exchanges its access token on the way through.
	let (account_pem, account_point) = generated_pem();
	let _ = account_pem;
	mock.scripted(|request, _| {
		if request.url.contains(FCM_TOKEN_HOST) {
			return Ok((
				200,
				vec![("content-type".to_owned(), "application/json".to_owned())],
				json!({ "access_token": "ya29.everywhere", "expires_in": 3600 }).to_string(),
			));
		}

		Err(None)
	});
	let _ = account_point;

	service.resume();
	let (status, _) = send(&service, "everywhere", spec(), &[ALICE]).await;
	assert_eq!(status, 200);

	let outcomes = settled(&service, "everywhere").await;
	assert_eq!(outcomes.len(), 3, "{outcomes:?}");
	assert!(
		outcomes.iter().all(|(state, _)| state == "accepted"),
		"{outcomes:?}"
	);
	assert_eq!(mock.requests_to(APNS_HOST).len(), 1);
	assert_eq!(mock.requests_to(FCM_HOST).len(), 1);
	assert_eq!(mock.requests_to(WEB_HOST).len(), 1);
}

// Transient failures and retries.

#[tokio::test(flavor = "multi_thread")]
async fn transient_failures_retry_and_recover() {
	let mock = Mock::new();
	let (_turn, mut service) = service(&mock, &all_transports()).await;

	let (status, _) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200);
	service.resume();
	let (status, _) = send(&service, "flaky", spec(), &[ALICE]).await;
	assert_eq!(status, 200);

	// A rate limit with a Retry-After, honored over the engine's own
	// backoff; then a server error; then success.
	mock.scripted(|request, index| {
		if !request.url.contains(APNS_HOST) {
			return Err(None);
		}

		match index {
			0 => {
				Ok((
					429,
					vec![("retry-after".to_owned(), "1".to_owned())],
					json!({ "reason": "TooManyRequests" }).to_string(),
				))
			}
			1 => Ok((503, Vec::new(), String::new())),
			_ => Err(None),
		}
	});

	let outcomes = settled(&service, "flaky").await;
	assert_eq!(
		outcomes,
		vec![("accepted".to_owned(), None)],
		"the delivery recovers: {outcomes:?}"
	);
	assert_eq!(mock.requests_to(APNS_HOST).len(), 3, "three attempts");

	let items = delivery_status(&service, "flaky").await;
	assert_eq!(items[0]["attempts"], 3, "every attempt is counted");
	assert!(items[0]["attempted_at"].as_str().is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_network_failure_retries_like_any_transient() {
	let mock = Mock::new();
	let (_turn, mut service) = service(&mock, &all_transports()).await;

	let (status, _) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200);
	service.resume();
	let (status, _) = send(&service, "unplugged", spec(), &[ALICE]).await;
	assert_eq!(status, 200);

	mock.scripted(|request, index| {
		if request.url.contains(APNS_HOST) {
			if index == 0 {
				return Err(Some(WireError::Unreachable));
			}

			return Err(None);
		}

		Err(None)
	});

	let outcomes = settled(&service, "unplugged").await;
	assert_eq!(
		outcomes,
		vec![("accepted".to_owned(), None)],
		"{outcomes:?}"
	);
	assert_eq!(mock.requests_to(APNS_HOST).len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn exhausted_attempts_end_rejected() {
	let mock = Mock::new();
	let (_turn, mut service) = service(&mock, &all_transports()).await;

	let (status, _) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200);
	service.resume();
	let (status, _) = send(&service, "doomed", spec(), &[ALICE]).await;
	assert_eq!(status, 200);

	// Every attempt answers a rate limit that asks for one second, until
	// the bound spends itself.
	mock.scripted(|request, _| {
		if request.url.contains(APNS_HOST) {
			return Ok((
				429,
				vec![("retry-after".to_owned(), "1".to_owned())],
				json!({ "reason": "TooManyRequests" }).to_string(),
			));
		}

		Err(None)
	});

	let outcomes = settled(&service, "doomed").await;
	assert_eq!(
		outcomes,
		vec![("rejected".to_owned(), Some("attempts_exhausted".to_owned()))],
		"{outcomes:?}"
	);
}

// Permanent failures and invalidation.

#[tokio::test(flavor = "multi_thread")]
async fn an_unregistered_credential_ends_and_invalidates() {
	let mock = Mock::new();
	let (_turn, mut service) = service(&mock, &all_transports()).await;

	// Two endpoints, the second of which the provider no longer knows.
	let (status, _) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200);
	let (status, _) = enroll(&service, ALICE, apns_credential(&device_token(2))).await;
	assert_eq!(status, 200);
	service.resume();

	// Two sends, so the unregistered endpoint holds two deliveries.
	let (status, _) = send(&service, "first", spec(), &[ALICE]).await;
	assert_eq!(status, 200);
	let (status, _) = send(&service, "second", spec(), &[ALICE]).await;
	assert_eq!(status, 200);

	mock.scripted(|request, _| {
		if !request.url.contains(APNS_HOST) {
			return Err(None);
		}

		if request.url.ends_with(&device_token(2)) {
			return Ok((
				410,
				Vec::new(),
				json!({ "reason": "Unregistered" }).to_string(),
			));
		}

		Err(None)
	});

	let first = settled(&service, "first").await;
	let second = settled(&service, "second").await;

	for outcomes in [&first, &second] {
		assert!(
			outcomes
				.iter()
				.any(|(state, reason)| state == "accepted" && reason.is_none()),
			"the live endpoint delivered: {outcomes:?}"
		);
		assert!(
			outcomes.contains(&("rejected".to_owned(), Some("unregistered".to_owned())))
				|| outcomes.contains(&(
					"skipped".to_owned(),
					Some("endpoint_unavailable".to_owned())
				)),
			"the dead endpoint's delivery ended clearly: {outcomes:?}"
		);
	}

	// The endpoint is invalidated, once, and nothing further is pushed
	// to it.
	let invalidated = rows(
		&service,
		&format!(
			"SELECT count(*) FROM push_endpoints WHERE credential = '{}'",
			device_token(2)
		),
	)
	.await;
	assert_eq!(text(&invalidated[0][0]), "1", "the endpoint is invalidated");

	let attempts_to_dead = mock
		.requests_to(APNS_HOST)
		.iter()
		.filter(|seen| seen.url.ends_with(&device_token(2)))
		.count();
	assert_eq!(attempts_to_dead, 1, "only the first delivery reached it");

	// Its status rows end as skipped, not rejected: they never tried.
	let items = delivery_status(&service, "second").await;
	assert!(
		items
			.iter()
			.any(|item| { item["state"] == "skipped" && item["reason"] == "endpoint_unavailable" }),
		"{items:?}"
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn provider_rejections_end_without_invalidating() {
	let mock = Mock::new();
	let (_turn, mut service) = service(&mock, &all_transports()).await;

	let (status, _) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200);
	service.resume();
	let (status, _) = send(&service, "refused", spec(), &[ALICE]).await;
	assert_eq!(status, 200);

	// A topic mismatch is the operator's configuration: the delivery
	// ends, the endpoint stays.
	mock.scripted(|request, _| {
		if request.url.contains(APNS_HOST) {
			return Ok((
				403,
				Vec::new(),
				json!({ "reason": "InvalidProviderToken" }).to_string(),
			));
		}

		Err(None)
	});

	let outcomes = settled(&service, "refused").await;
	assert_eq!(
		outcomes,
		vec![("rejected".to_owned(), Some("provider_rejected".to_owned()))],
		"{outcomes:?}"
	);

	let invalidated = rows(
		&service,
		"SELECT count(*) FROM push_endpoints WHERE invalidated_at IS NOT NULL",
	)
	.await;
	assert_eq!(
		text(&invalidated[0][0]),
		"0",
		"the endpoint is not invalidated"
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_web_push_gone_answers_invalidate_too() {
	let mock = Mock::new();
	let config = config::PushConfig {
		apns: None,
		fcm: None,
		web: Some(web_config_for(&generated_pem().0).1),
	};
	let (_turn, mut service) = service(&mock, &config).await;

	let endpoint = format!("https://{WEB_HOST}/push/abcdef123456");
	let (status, _) = enroll(&service, ALICE, web_credential(&endpoint)).await;
	assert_eq!(status, 200);
	service.resume();
	let (status, _) = send(&service, "gone", spec(), &[ALICE]).await;
	assert_eq!(status, 200);

	mock.scripted(|request, _| {
		if request.url.contains(WEB_HOST) {
			return Ok((410, Vec::new(), String::new()));
		}

		Err(None)
	});

	let outcomes = settled(&service, "gone").await;
	assert_eq!(
		outcomes,
		vec![("rejected".to_owned(), Some("unregistered".to_owned()))],
		"{outcomes:?}"
	);
	let invalidated = rows(
		&service,
		"SELECT count(*) FROM push_endpoints WHERE invalidated_at IS NOT NULL",
	)
	.await;
	assert_eq!(text(&invalidated[0][0]), "1");
}

// Payload limits, on the encoded bytes.

#[tokio::test(flavor = "multi_thread")]
async fn oversized_payloads_drop_data_first_and_end_clearly() {
	let _turn = TURN.lock().await;

	let mock = Mock::new();
	let mut service = start(
		leptoskit_db::testing::sqlite_migrated(MIGRATIONS).await,
		&all_transports(),
		&mock,
	)
	.await;

	let (status, _) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200);
	service.resume();

	// A send whose full payload is past 4 KiB — a big body and data to
	// match — but whose minimal envelope fits: the data drops, the push
	// still goes.
	let big = json!({
		"category": "news",
		"title": "It ships",
		"body": "x".repeat(1_900),
		"data": json!({ "x": "x".repeat(4_000) }).to_string(),
	});
	let (status, body) = send(&service, "big-but-fits", big, &[ALICE]).await;
	assert_eq!(status, 200, "{body}");

	let outcomes = settled(&service, "big-but-fits").await;
	assert_eq!(
		outcomes,
		vec![("accepted".to_owned(), None)],
		"{outcomes:?}"
	);

	let sent = mock.requests_to(APNS_HOST);
	assert_eq!(sent.len(), 1);
	let payload = sent[0].json();
	assert!(
		payload["lk"].is_null(),
		"the data dropped: {}",
		sent[0].body.len()
	);
	assert!(payload["lkn"].as_str().is_some(), "the id stayed");
	assert_eq!(payload["aps"]["alert"]["body"], "x".repeat(1_900));

	// A send too big even for the minimal envelope — at FCM the visible
	// notification rides twice, in `notification` and again inside the
	// `apns` payload, so a full-size body pushes the encoded message past
	// 4 KiB whatever is dropped. It ends oversized, with no request at
	// all.
	let mock2 = Mock::new();
	let mut service2 = start(
		leptoskit_db::testing::sqlite_migrated(MIGRATIONS).await,
		&all_transports(),
		&mock2,
	)
	.await;
	let (status, _) = enroll(&service2, ALICE, fcm_credential("fcm-token-2")).await;
	assert_eq!(status, 200);
	// The exchange would answer normally; the delivery never gets there.
	mock2.scripted(|request, _| {
		if request.url.contains(FCM_TOKEN_HOST) {
			return Ok((
				200,
				vec![("content-type".to_owned(), "application/json".to_owned())],
				json!({ "access_token": "ya29.too-big", "expires_in": 3600 }).to_string(),
			));
		}

		Err(None)
	});
	service2.resume();

	let too_big = json!({
		"category": "news",
		"title": "It ships",
		"body": "y".repeat(2_000),
	});
	let (status, body) = send(&service2, "too-big", too_big, &[ALICE]).await;
	assert_eq!(status, 200, "{body}");

	let outcomes = settled(&service2, "too-big").await;
	assert_eq!(
		outcomes,
		vec![("rejected".to_owned(), Some("oversized".to_owned()))],
		"{outcomes:?}"
	);
	assert!(
		mock2.requests_to(FCM_HOST).is_empty(),
		"an oversized payload never reaches the provider"
	);
}

// The schedule, cancellation, expiry, and the latest opt-out, at the
// attempt.

#[tokio::test(flavor = "multi_thread")]
async fn cancelled_sends_never_push() {
	let mock = Mock::new();
	let (_turn, mut service) = service(&mock, &all_transports()).await;

	let (status, _) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200);

	let due = (chrono::Utc::now() + chrono::Duration::seconds(2)).to_rfc3339();
	let (status, body) = send(
		&service,
		"scheduled-cancelled",
		json!({ "category": "news", "title": "Later", "visible_at": due }),
		&[ALICE],
	)
	.await;
	assert_eq!(status, 200, "{body}");

	let (status, body) = post(
		&service,
		"cancel",
		json!({ "idempotency_key": "scheduled-cancelled" }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");

	service.resume();
	let outcomes = settled(&service, "scheduled-cancelled").await;
	assert_eq!(
		outcomes,
		vec![("skipped".to_owned(), Some("cancelled".to_owned()))],
		"{outcomes:?}"
	);
	assert!(mock.requests_to(APNS_HOST).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scheduled_send_pushes_when_it_becomes_visible() {
	let mock = Mock::new();
	let (_turn, mut service) = service(&mock, &all_transports()).await;

	let (status, _) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200);

	let due = (chrono::Utc::now() + chrono::Duration::seconds(1)).to_rfc3339();
	let (status, body) = send(
		&service,
		"due-soon",
		json!({ "category": "news", "title": "Soon", "visible_at": due }),
		&[ALICE],
	)
	.await;
	assert_eq!(status, 200, "{body}");

	service.resume();
	let outcomes = settled(&service, "due-soon").await;
	assert_eq!(
		outcomes,
		vec![("accepted".to_owned(), None)],
		"{outcomes:?}"
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn expired_notifications_are_skipped_at_the_attempt() {
	let mock = Mock::new();
	let (_turn, mut service) = service(&mock, &all_transports()).await;

	let (status, _) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200);

	let expires = (chrono::Utc::now() + chrono::Duration::seconds(1)).to_rfc3339();
	let (status, body) = send(
		&service,
		"fleeting",
		json!({ "category": "news", "title": "Briefly", "expires_at": expires }),
		&[ALICE],
	)
	.await;
	assert_eq!(status, 200, "{body}");

	// No workers until it has expired.
	tokio::time::sleep(Duration::from_millis(1_300)).await;
	service.resume();

	let outcomes = settled(&service, "fleeting").await;
	assert_eq!(
		outcomes,
		vec![("skipped".to_owned(), Some("expired".to_owned()))],
		"{outcomes:?}"
	);
	assert!(mock.requests_to(APNS_HOST).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_opt_out_made_in_the_queue_decides_at_the_attempt() {
	let mock = Mock::new();
	let (_turn, mut service) = service(&mock, &all_transports()).await;

	let (status, _) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200);
	let (status, _) = send(&service, "maybe", spec(), &[ALICE]).await;
	assert_eq!(status, 200);

	// Alice opts out before any attempt runs.
	let (status, body) = post(
		&service,
		"set_category_preference",
		json!({ "category": "news", "opted_out": true }),
		Some(&signed_in(ALICE)),
	)
	.await;
	assert_eq!(status, 200, "{body}");

	service.resume();
	let outcomes = settled(&service, "maybe").await;
	assert_eq!(
		outcomes,
		vec![("skipped".to_owned(), Some("opted_out".to_owned()))],
		"{outcomes:?}"
	);
	assert!(mock.requests_to(APNS_HOST).is_empty());
}

// Durability.

#[tokio::test(flavor = "multi_thread")]
async fn a_restart_recovers_pending_work() {
	let _turn = TURN.lock().await;
	let file = tempfile::Builder::new()
		.prefix("notifications-push-restart-")
		.suffix(".sqlite3")
		.tempfile()
		.expect("a database file");
	let url = format!("sqlite://{}", file.path().display());
	let mock = Arc::new(Mock::new());

	// The first process: enroll, send, and stop before any attempt runs.
	let db = Db::connect(&url).await.expect("the database opens");
	db.migrate(MIGRATIONS).await.expect("migrations apply");
	{
		let service = start(db, &all_transports(), &mock).await;
		let (status, _) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
		assert_eq!(status, 200);
		let (status, _) = send(&service, "across-restart", spec(), &[ALICE]).await;
		assert_eq!(status, 200);

		let enqueued = delivery_status(&service, "across-restart").await;
		assert_eq!(enqueued[0]["state"], "enqueued", "{enqueued:?}");
		assert!(
			mock.requests_to(APNS_HOST).is_empty(),
			"nothing attempted yet"
		);
		// Dropping the service stops its workers and its live socket:
		// the process is gone, its rows stay.
	}

	// The second process, over the same database: the delivery row and
	// its round-zero job are there, and the push completes.
	let db = Db::connect(&url).await.expect("the database opens again");
	let mut restarted = start(db, &all_transports(), &mock).await;
	restarted.resume();
	let outcomes = settled(&restarted, "across-restart").await;
	assert_eq!(
		outcomes,
		vec![("accepted".to_owned(), None)],
		"{outcomes:?}"
	);
	assert_eq!(mock.requests_to(APNS_HOST).len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_duplicate_send_enrolls_each_endpoint_once() {
	let mock = Mock::new();
	let (_turn, mut service) = service(&mock, &all_transports()).await;

	let (status, _) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200);
	service.resume();

	let (status, first) = send(&service, "once", spec(), &[ALICE]).await;
	assert_eq!(status, 200, "{first}");
	let (status, again) = send(&service, "once", spec(), &[ALICE]).await;
	assert_eq!(status, 200, "{again}");
	assert!(again.contains("\"duplicate\":true"), "{again}");

	let outcomes = settled(&service, "once").await;
	assert_eq!(outcomes.len(), 1, "{outcomes:?}");
	let stored = rows(&service, "SELECT count(*) FROM push_deliveries").await;
	assert_eq!(text(&stored[0][0]), "1", "one delivery row, once");
	assert_eq!(mock.requests_to(APNS_HOST).len(), 1, "and one push");
}

#[tokio::test(flavor = "multi_thread")]
async fn one_failing_endpoint_never_holds_its_peers() {
	let mock = Mock::new();
	let (_turn, mut service) = service(&mock, &all_transports()).await;

	let (status, _) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200);
	let (status, _) = enroll(&service, BOB, apns_credential(&device_token(3))).await;
	assert_eq!(status, 200);
	service.resume();
	let (status, _) = send(&service, "mixed", spec(), &[ALICE, BOB]).await;
	assert_eq!(status, 200);

	// Alice's endpoint is rate limited forever, asking for one second
	// each time; bob's answers success. Alice's attempts spend the bound
	// while bob's delivery completes untouched.
	mock.scripted(|request, _| {
		if !request.url.contains(APNS_HOST) {
			return Err(None);
		}

		if request.url.ends_with(&device_token(1)) {
			return Ok((
				429,
				vec![("retry-after".to_owned(), "1".to_owned())],
				json!({ "reason": "TooManyRequests" }).to_string(),
			));
		}

		Err(None)
	});

	let outcomes = settled(&service, "mixed").await;
	assert!(
		outcomes.contains(&("accepted".to_owned(), None)),
		"bob's delivery completed: {outcomes:?}"
	);
	assert!(
		outcomes.contains(&("rejected".to_owned(), Some("attempts_exhausted".to_owned()))),
		"alice's ended on its own: {outcomes:?}"
	);
}

// Where credentials never appear.

#[tokio::test(flavor = "multi_thread")]
async fn credentials_never_appear_in_answers_or_debugs() {
	let mock = Mock::new();
	let (_turn, mut service) = service(&mock, &all_transports()).await;

	let token = device_token(1);
	let (status, register) = enroll(&service, ALICE, apns_credential(&token)).await;
	assert_eq!(status, 200, "{register}");

	service.resume();
	let (status, _) = send(&service, "quiet", spec(), &[ALICE]).await;
	assert_eq!(status, 200);
	settled(&service, "quiet").await;

	let (_, listed) = post(
		&service,
		"push_endpoints",
		json!({}),
		Some(&signed_in(ALICE)),
	)
	.await;
	let (_, reported) = post(
		&service,
		"push_deliveries",
		json!({ "send_key": "quiet", "page": { "limit": 100 } }),
		Some(TOKEN),
	)
	.await;
	let (_, conflict) = enroll(&service, BOB, apns_credential(&token)).await;

	// The stored row's Debug, redacted by the table macro: the row type
	// formats its secret column as `<secret>`.
	let stored_rows = leptoskit_notifications::push::PushEndpoint::query()
		.filter(|endpoint| endpoint.person.eq(ALICE))
		.all(&service.db)
		.await
		.expect("a read");
	let stored = format!("{stored_rows:?}");

	for (name, answer) in [
		("the register answer", register),
		("the endpoint list", listed),
		("the delivery status", reported),
		("the cross-account refusal", conflict),
		("the stored row's debug", stored),
	] {
		assert!(
			!answer.contains(&token),
			"{name} carries the credential: {answer}"
		);
	}
}

// The push-host policy at enrollment.

#[tokio::test(flavor = "multi_thread")]
async fn web_push_enrollment_refuses_hosts_the_policy_wont_push_to() {
	let mock = Mock::new();
	let (_turn, service) = service(&mock, &all_transports()).await;

	let refused = [
		(
			"a plain http endpoint",
			"http://fcm.googleapis.com/fcm/send/abc",
		),
		("loopback", "https://localhost/push/1"),
		("loopback by address", "https://127.0.0.1/push/1"),
		("metadata", "https://169.254.169.254/latest/meta-data"),
		("a private range", "https://10.0.0.1/push/1"),
		("an arbitrary host", "https://attacker.test/push/1"),
		(
			"a suffix trick",
			"https://fcm.googleapis.com.attacker.test/push/1",
		),
		("a wrong port", "https://fcm.googleapis.com:8443/push/1"),
		(
			"credentials in the URL",
			"https://user:pass@fcm.googleapis.com/fcm/send/abc",
		),
	];

	for (name, endpoint) in refused {
		let (status, body) = enroll(&service, ALICE, web_credential(endpoint)).await;
		assert_eq!(status, 400, "{name} is refused: {body}");
		assert!(
			!body.contains(endpoint),
			"the refusal doesn't echo the attacker's URL: {body}"
		);
	}

	// The shipped host enrolls, and the mock receives exactly it.
	let shipped = format!("https://{WEB_HOST}/push/abcdef123456");
	let (status, body) = enroll(&service, ALICE, web_credential(&shipped)).await;
	assert_eq!(status, 200, "{body}");
}

// Operator configuration.

#[test]
fn partial_provider_config_is_named_and_refused() {
	let vars = [
		(
			"LEPTOSKIT_NOTIFICATIONS_APNS_KEY_P8".to_owned(),
			"-----BEGIN PRIVATE KEY-----".to_owned(),
		),
		(
			"LEPTOSKIT_NOTIFICATIONS_APNS_KEY_ID".to_owned(),
			"KEY123".to_owned(),
		),
	];
	let error = config::PushConfig::from_vars(vars).expect_err("half of APNs is an error");
	assert!(
		error.contains("APNS_TEAM_ID")
			&& error.contains("APNS_TOPIC")
			&& error.contains("APNS_ENVIRONMENT"),
		"every missing piece is named: {error}"
	);
}

#[test]
fn an_unusable_key_fails_the_build_not_the_first_send() {
	let config = config::PushConfig {
		apns: Some(config::ApnsConfig {
			key_p8: "not a key".into(),
			key_id: "KEY123".into(),
			team_id: "TEAM456".into(),
			topic: "dev.leptoskit.app".into(),
			environment: config::ApnsEnvironment::Production,
		}),
		fcm: None,
		web: None,
	};
	let Err(error) = PushRuntime::build(&config) else {
		panic!("a broken key refuses to build");
	};
	assert!(error.contains("APNs"), "{error}");
}

#[test]
fn a_mismatched_vapid_pair_is_refused_at_the_build() {
	let (key_pem, point) = generated_pem();
	let other = generated_pem();
	let der = pem_der(&key_pem);
	let config = config::PushConfig {
		apns: None,
		fcm: None,
		web: Some(config::WebPushConfig {
			vapid_private_key: URL_SAFE_NO_PAD.encode(&der[36..68]),
			vapid_public_key: URL_SAFE_NO_PAD.encode(&other.1),
			subject: "mailto:push@leptoskit.dev".into(),
			extra_hosts: Vec::new(),
		}),
	};
	let Err(error) = PushRuntime::build(&config) else {
		panic!("a mismatched pair refuses to build");
	};
	assert!(error.contains("VAPID"), "{error}");
	let _ = point;
}
