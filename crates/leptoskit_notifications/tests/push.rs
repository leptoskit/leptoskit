//! Push delivery, end to end on a real port: enrollment's authentication
//! and bounds, cross-account refusal, owner-only revocation, the fanout a
//! send makes, the wire each transport puts on the boundary — signed
//! tokens, exchanged credentials, encrypted bodies — transient retries
//! that recover, permanent failures that end and invalidate, oversized
//! payloads that end clearly without a call, schedule, cancellation,
//! expiry, and opt-out decided at the attempt, recovery across a
//! restart, idempotent duplicate sends, the endpoint identity and
//! rotation semantics, the fenced outcome writes that make revocations
//! and replayed rounds safe, and where the credentials never appear.
//!
//! The transports are exercised over an injected boundary: each test
//! installs a mock that receives the real provider requests and answers
//! a scripted response — holding one in flight, for the race tests,
//! until the test lets it go. The URLs, the headers, the payloads, and
//! the host policy are the production path's own; only the socket
//! differs. Every test serves its own database and its own push
//! instance, so they run in parallel without sharing credentials.

#![cfg(not(target_arch = "wasm32"))]

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
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
use ring::aead::Aad;
use ring::aead::LessSafeKey;
use ring::aead::Nonce;
use ring::agreement::EphemeralPrivateKey;
use ring::agreement::UnparsedPublicKey;
use ring::agreement::agree_ephemeral;
use ring::hkdf::HKDF_SHA256;
use ring::hkdf::Salt;
use ring::rand::SecureRandom as _;
use ring::rand::SystemRandom;
use ring::signature::ECDSA_P256_SHA256_FIXED;
use ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING;
use ring::signature::EcdsaKeyPair;
use ring::signature::RSA_PKCS1_2048_8192_SHA256;
use ring::signature::RsaPublicKeyComponents;
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

/// A fake access token the script answers and the send endpoint refuses:
/// a made-up value, like every token in this file.
const REFUSED_ACCESS_TOKEN: &str = "ya29.forever-bad"; // gitleaks:allow (a fake token a test answers)

/// What the boundary received, for the tests to inspect.
#[derive(Clone)]
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

/// One scripted answer: a status, headers, and a body — a socket
/// failure — or the default success. `Hold` is the same answer, but the
/// request is held in flight until the test releases it, so a
/// revocation or a second round can race it deterministically.
enum Scripted {
	Reply(u16, Vec<(String, String)>, String),
	Wire(WireError),
	Hold(u16, Vec<(String, String)>, String),
	Default,
}

type Answer = Scripted;

/// The test's script: one request and its host's request count in, an
/// answer out.
type Script = Box<dyn Fn(&WireRequest, usize) -> Answer + Send + Sync>;

/// The mock boundary: it records every request, counts requests per
/// host, answers through the test's script, and can hold requests in
/// flight for the race tests.
struct Mock {
	seen: std::sync::Mutex<Vec<Seen>>,
	counts: std::sync::Mutex<std::collections::HashMap<String, usize>>,
	script: std::sync::RwLock<Script>,
	/// Held requests wait on this; the test fires it when they may go.
	gate: tokio::sync::Notify,
	/// How many requests are waiting on the gate right now, so a test
	/// can know its hold has truly landed.
	holding: AtomicUsize,
}

impl Mock {
	fn new() -> Arc<Self> {
		Arc::new(Self {
			seen: std::sync::Mutex::new(Vec::new()),
			counts: std::sync::Mutex::new(std::collections::HashMap::new()),
			script: std::sync::RwLock::new(Box::new(|_, _| Scripted::Default)),
			gate: tokio::sync::Notify::new(),
			holding: AtomicUsize::new(0),
		})
	}

	/// Answers future requests through `script`, which sees the request
	/// and how many requests its host has had before this one.
	fn scripted(&self, script: impl Fn(&WireRequest, usize) -> Answer + Send + Sync + 'static) {
		*self.script.write().unwrap() = Box::new(script);
	}

	/// Lets every held request go.
	fn release(&self) {
		self.gate.notify_waiters();
	}

	/// Waits until `count` requests are held in flight.
	async fn wait_held(&self, count: usize) {
		for _ in 0..600 {
			if self.holding.load(Ordering::SeqCst) >= count {
				return;
			}

			tokio::time::sleep(Duration::from_millis(25)).await;
		}

		panic!(
			"{} requests never went in flight",
			count - self.holding.load(Ordering::SeqCst)
		);
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
				Scripted::Reply(status, headers, body) => {
					Ok(WireResponse {
						status,
						headers,
						body: body.into_bytes(),
					})
				}
				Scripted::Wire(error) => Err(error),
				Scripted::Hold(status, headers, body) => {
					// Subscribe before counting ourselves held, so a
					// test that waits for the count can always release
					// us.
					let released = self.gate.notified();
					tokio::pin!(released);
					released.as_mut().enable();
					self.holding.fetch_add(1, Ordering::SeqCst);
					released.await;
					self.holding.fetch_sub(1, Ordering::SeqCst);

					Ok(WireResponse {
						status,
						headers,
						body: body.into_bytes(),
					})
				}
				Scripted::Default => {
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
	push: push::Push,
	jobs: Option<rpc::server::JobsHandle>,
}

/// A fresh, migrated service on a free port, its push transports built
/// over `mock` and carried by its routes, its workers not yet running:
/// tests start them when the queue should move. Each service owns its runtime.
async fn start(db: Db, config: &config::PushConfig, mock: &Arc<Mock>) -> Service {
	let runtime = if config.apns.is_some() || config.fcm.is_some() || config.web.is_some() {
		Some(PushRuntime::build_over(config, mock.clone()).expect("the runtime builds"))
	} else {
		None
	};

	start_push(db, runtime).await
}

/// [`start`], over an already-built (or absent) runtime.
async fn start_push(db: Db, runtime: Option<PushRuntime>) -> Service {
	let push = push::Push {
		runtime: runtime.map(Arc::new),
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

/// Runs one push round directly, outside any worker, as the race tests
/// do: the deliveries' own writes decide everything. Owned, so it can be
/// spawned beside whatever it races.
fn spawn_round(service: &Service, send_key: &str, round: u32) -> tokio::task::JoinHandle<()> {
	let db = service.db.clone();
	let push = service.push.clone();
	let send_key = send_key.to_owned();

	tokio::spawn(async move {
		let cx = rpc::JobCx::new(
			db,
			rpc::server::Identity::anonymous(),
			format!("test:{send_key}:{round}"),
			1,
		);
		push::round(&cx, &send_key, round, &push)
			.await
			.expect("the round runs");
	})
}

impl Drop for Service {
	fn drop(&mut self) {
		if let Some(jobs) = self.jobs.take() {
			jobs.stop();
		}
	}
}

/// A fresh, migrated database.
async fn db() -> Db {
	leptoskit_db::testing::sqlite_migrated(MIGRATIONS).await
}

async fn service(mock: &Arc<Mock>, config: &config::PushConfig) -> Service {
	start(db().await, config, mock).await
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

/// A test-only 2048-bit RSA private key, in the PKCS#8 PEM form Google's
/// service-account JSON carries. ring signs RSA but generates no RSA
/// keys, so this key was made once, out of band, for these tests alone —
/// the same fixture pattern the `IdP` suite uses; it signs nothing outside
/// the tests, whose provider sockets are all mocks.
const TEST_RSA_KEY: &str = include_str!("fixtures/test-only-rsa-key.pem");

/// The test RSA key, as the service-account JSON carries it, next to the
/// modulus and exponent its signatures must verify against.
fn test_rsa() -> (String, (Vec<u8>, Vec<u8>)) {
	(
		TEST_RSA_KEY.to_owned(),
		rsa_public_half(&pem_der(TEST_RSA_KEY)),
	)
}

/// One DER tag-length-value out of `input`: the tag, its contents, and
/// what follows. Enough for the fixed shape a PKCS#8 RSA key has.
fn der_tlv(input: &[u8]) -> Option<(u8, &[u8], &[u8])> {
	let (&tag, rest) = input.split_first()?;
	let (&first, rest) = rest.split_first()?;
	let (length, rest) = if first & 0x80 == 0 {
		(usize::from(first), rest)
	} else {
		let count = usize::from(first & 0x7f);
		let mut length = 0_usize;

		for byte in rest.get(..count)? {
			length = (length << 8) | usize::from(*byte);
		}

		(length, rest.get(count..)?)
	};
	let contents = rest.get(..length)?;

	Some((tag, contents, rest.get(length..)?))
}

/// The public half of a PKCS#8 RSA private key: the modulus and the
/// exponent, big-endian without leading zeros, as ring's verifier takes
/// them. A small walk over the DER shape — a `PrivateKeyInfo` around an
/// `RSAPrivateKey`, whose first two integers are the public half.
fn rsa_public_half(pkcs8_der: &[u8]) -> (Vec<u8>, Vec<u8>) {
	let (_, info, _) = der_tlv(pkcs8_der).expect("a PrivateKeyInfo sequence");
	let (_, version, rest) = der_tlv(info).expect("a version integer");
	assert_eq!(version, [0], "a PKCS#8 v1 key");
	let (_, _algorithm, rest) = der_tlv(rest).expect("an algorithm identifier");
	let (_, key, _) = der_tlv(rest).expect("the private key octets");
	let (_, key_sequence, _) = der_tlv(key).expect("an RSAPrivateKey sequence");
	let (_, _version, rest) = der_tlv(key_sequence).expect("an RSA version");
	let (_, modulus, rest) = der_tlv(rest).expect("the modulus");
	let (_, exponent, _) = der_tlv(rest).expect("the exponent");

	let magnitude = |bytes: &[u8]| -> Vec<u8> {
		let start = bytes
			.iter()
			.position(|byte| *byte != 0)
			.unwrap_or(bytes.len().saturating_sub(1));
		bytes[start..].to_vec()
	};

	(magnitude(modulus), magnitude(exponent))
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

/// A service-account JSON around `private_key_pem`, in Google's own
/// shape — including a `token_uri`, which the transport must ignore in
/// favor of its fixed first-party endpoint.
fn fcm_config_for(private_key_pem: &str) -> config::FcmConfig {
	config::FcmConfig {
		service_account_json: json!({
			"type": "service_account",
			"project_id": "leptoskit-push",
			"private_key_id": "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
			"private_key": private_key_pem,
			"client_email": "push@leptoskit-push.iam.gserviceaccount.com",
			"client_id": "123456789000000000000",
			"token_uri": "https://evil.example.test/token",
		})
		.to_string(),
	}
}

/// A VAPID pair from a PEM: the config and the public point to verify
/// with.
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
			"target": { "recipients": recipients },
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
	let (_, credential, ..) = web_subscription(endpoint);
	credential
}

/// A browser subscription whose receiver side the test keeps: the
/// private key that made the public point (consumed by the decrypt), the
/// auth secret, and the enrollment JSON.
fn web_subscription(endpoint: &str) -> (EphemeralPrivateKey, Value, Vec<u8>, String) {
	let rng = SystemRandom::new();
	let private = EphemeralPrivateKey::generate(&ring::agreement::ECDH_P256, &rng).expect("a key");
	let point = private
		.compute_public_key()
		.expect("a point")
		.as_ref()
		.to_vec();
	let mut auth = [0_u8; 16];
	rng.fill(&mut auth).expect("an auth secret");
	let public = URL_SAFE_NO_PAD.encode(&point);

	(
		private,
		json!({
			"web_push": {
				"endpoint": endpoint,
				"keys": {
					"p256dh": public,
					"auth": URL_SAFE_NO_PAD.encode(auth),
				},
			}
		}),
		auth.to_vec(),
		public,
	)
}

async fn endpoints_of(service: &Service, person: Uuid) -> Vec<Value> {
	let (status, body) = post(
		service,
		"push_endpoints",
		json!({ "page": { "limit": 100 } }),
		Some(&signed_in(person)),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let page: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
	page["items"].as_array().cloned().unwrap_or_default()
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

/// Waits until the deliveries of `send_key` have left their initial
/// enqueue — long enough for a bounded DNS resolution to give up — and
/// answers their last-read rows.
async fn wait_not_enqueued(service: &Service, send_key: &str) -> Vec<Value> {
	for _ in 0..400 {
		let items = delivery_status(service, send_key).await;

		if !items.is_empty() && items.iter().all(|item| item["state"] != "enqueued") {
			return items;
		}

		tokio::time::sleep(Duration::from_millis(50)).await;
	}

	delivery_status(service, send_key).await
}

/// Waits until every delivery of `send_key` has left the queue, and
/// answers their states and reasons.
async fn settled(service: &Service, send_key: &str) -> Vec<(String, Option<String>)> {
	settle(service, send_key, false).await
}

/// [`settled`], moving the queue's clock past every retry's wait as it
/// waits: retries wait minutes to hours, which a test that follows a
/// delivery through them takes as read.
async fn settled_rewinding(service: &Service, send_key: &str) -> Vec<(String, Option<String>)> {
	settle(service, send_key, true).await
}

async fn settle(
	service: &Service,
	send_key: &str,
	rewinding: bool,
) -> Vec<(String, Option<String>)> {
	for _ in 0..300 {
		if rewinding {
			rewind(service, send_key).await;
		}

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

/// Makes every retrying delivery of `send_key` due now, and every push
/// round waiting to run them.
async fn rewind(service: &Service, send_key: &str) {
	use leptoskit_notifications::push::PushDelivery;

	let now = leptoskit_db::now();
	PushDelivery::update()
		.filter(|row| row.send_key.eq(send_key.to_owned()) & row.state.eq("retrying"))
		.set(|row| row.next_attempt_at, Some(now))
		.run(&service.db)
		.await
		.expect("the deliveries rewind");
	service
		.db
		.fetch(Statement {
			sql: "UPDATE \"rpc_jobs\" SET \"run_at\" = ?1 WHERE \"name\" = 'push_send' AND \
			      \"state\" = 'pending'"
				.into(),
			args: vec![leptoskit_db::Value::Timestamp(now)],
		})
		.await
		.expect("the rounds rewind");
}

/// How many of `send_key`'s deliveries are in `state`.
async fn in_state(service: &Service, send_key: &str, state: &str) -> usize {
	let stored = rows(
		service,
		&format!(
			"SELECT count(*) FROM push_deliveries WHERE send_key = '{send_key}' AND state = \
			 '{state}'"
		),
	)
	.await;
	text(&stored[0][0]).parse().expect("a count")
}

/// Waits until `count` of `send_key`'s deliveries are accepted.
async fn all_accepted(service: &Service, send_key: &str, count: usize) {
	for _ in 0..600 {
		if in_state(service, send_key, "accepted").await >= count {
			return;
		}

		tokio::time::sleep(Duration::from_millis(50)).await;
	}

	panic!(
		"{} of {count} deliveries of {send_key} were accepted; {} still enqueued",
		in_state(service, send_key, "accepted").await,
		in_state(service, send_key, "enqueued").await,
	);
}

/// Enrolls one APNs endpoint per entry of `people`, by writing the rows a
/// registration writes: thousands of HTTP registrations would only slow
/// these tests down.
async fn enroll_directly(service: &Service, people: &[Uuid]) {
	let tx = service.db.begin().await.expect("a transaction");

	for (n, person) in people.iter().enumerate() {
		leptoskit_notifications::push::NewPushEndpoint {
			id: Uuid::now_v7(),
			person: *person,
			platform: "apns".to_owned(),
			credential: format!("{n:064x}"),
			generation: 1,
			fingerprint: format!("{n:064x}"),
			invalidated_at: None,
			locale: None,
			time_zone: None,
			app_version: None,
			seen_at: None,
			imported_at: None,
		}
		.insert(&tx)
		.await
		.expect("an endpoint");
	}

	tx.commit().await.expect("the endpoints commit");
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

/// Verifies an RS256 JWT against the public half of the key that signed
/// it — ring's own RSA verifier, an independent check of this file's
/// signer — answering its header and claims.
fn verify_rs256(token: &str, public: &(Vec<u8>, Vec<u8>)) -> (Value, Value) {
	let parts: Vec<&str> = token.split('.').collect();
	assert_eq!(parts.len(), 3, "a JWT has three parts: {token}");
	let signing = format!("{}.{}", parts[0], parts[1]);
	let signature = URL_SAFE_NO_PAD
		.decode(parts[2])
		.expect("the signature is base64url");

	let (modulus, exponent) = public;
	RsaPublicKeyComponents {
		n: modulus,
		e: exponent,
	}
	.verify(&RSA_PKCS1_2048_8192_SHA256, signing.as_bytes(), &signature)
	.expect("the RS256 signature verifies");

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

/// The FCM token-exchange script that verifies each assertion
/// independently and answers a plain access token, for tests that only
/// care to get past the exchange.
fn passing_exchange() -> impl Fn(&WireRequest, usize) -> Answer + Send + Sync + 'static {
	|request, _| {
		if !request.url.contains(FCM_TOKEN_HOST) {
			return Scripted::Default;
		}

		assert!(
			wire_header(request, "content-type")
				.is_some_and(|value| value.contains("application/x-www-form-urlencoded"))
		);
		Scripted::Reply(
			200,
			vec![("content-type".to_owned(), "application/json".to_owned())],
			json!({ "access_token": "ya29.test-token", "expires_in": 3600 }).to_string(),
		)
	}
}

/// The independent receiver side of RFC 8291: opens an `aes128gcm` body
/// with the receiver's own key material, derived by this test's own
/// ring-based HKDF rather than anything the sender's library shares.
/// The receiver's `private` is consumed, exactly as an ephemeral
/// agreement is. Answers the decrypted payload.
fn open_aes128gcm(
	body: &[u8],
	receiver_private: EphemeralPrivateKey,
	receiver_point: &[u8],
	auth_secret: &[u8],
) -> Vec<u8> {
	assert!(body.len() >= 21, "an aes128gcm header");
	let mut salt = [0_u8; 16];
	salt.copy_from_slice(&body[0..16]);
	let record_size = u32::from_be_bytes([body[16], body[17], body[18], body[19]]);
	let key_id_len = body[20] as usize;
	let end = 21 + key_id_len;
	assert!(body.len() >= end, "the header's key id fits");
	let sender_point = &body[21..end];
	assert_eq!(sender_point.len(), 65, "the key id is the sender's point");
	assert_eq!(sender_point[0], 0x04, "an uncompressed point");
	let record = &body[end..];
	assert_eq!(record_size, 4_096, "one 4 KiB record");

	// The receiver derives from their own view: their public point and
	// the sender's from the header in the key info, and the ECDH secret
	// theirs against the sender's.
	let shared = agree_ephemeral(
		receiver_private,
		&UnparsedPublicKey::new(&ring::agreement::ECDH_P256, sender_point.to_vec()),
		<[u8]>::to_vec,
	)
	.expect("the sender's point is on the curve");

	let mut key_info = Vec::with_capacity(14 + 65 + 65);
	key_info.extend_from_slice(b"WebPush: info\0");
	key_info.extend_from_slice(receiver_point);
	key_info.extend_from_slice(sender_point);
	let ikm = derive(auth_secret, &shared, &[&key_info], 32);
	let cek = derive(&salt, &ikm, &[b"Content-Encoding: aes128gcm\0"], 16);
	let nonce = derive(&salt, &ikm, &[b"Content-Encoding: nonce\0"], 12);

	let key = LessSafeKey::new(
		ring::aead::UnboundKey::new(&ring::aead::AES_128_GCM, &cek).expect("the content key"),
	);
	let mut record = record.to_vec();
	let opened = key
		.open_in_place(
			Nonce::assume_unique_for_key(nonce.try_into().expect("the nonce is 12 bytes")),
			Aad::empty(),
			&mut record,
		)
		.expect("the record opens");

	// The record's plaintext is the payload, the delimiter octet, and
	// zero padding (RFC 8188): strip the zeros, then the delimiter.
	let unpadded = opened
		.iter()
		.rposition(|byte| *byte != 0)
		.expect("a record is never empty");
	assert_eq!(
		opened[unpadded], 0x02,
		"the last-record delimiter, after the padding"
	);

	opened[..unpadded].to_vec()
}

/// HKDF-SHA256, the test's own: the lengths asked for here are fixed and
/// valid.
fn derive(salt: &[u8], secret: &[u8], infos: &[&[u8]], length: usize) -> Vec<u8> {
	struct Length(usize);

	impl ring::hkdf::KeyType for Length {
		fn len(&self) -> usize {
			self.0
		}
	}

	let prk = Salt::new(HKDF_SHA256, salt).extract(secret);
	let expansion = prk.expand(infos, Length(length)).expect("a valid length");
	let mut okm = vec![0_u8; length];
	expansion.fill(&mut okm).expect("a valid fill");
	okm
}

/// The stored credential of one endpoint, by its opaque id.
async fn credential_of(service: &Service, endpoint: &str) -> String {
	leptoskit_notifications::push::PushEndpoint::query()
		.filter(|row| row.id.eq(Uuid::parse_str(endpoint).unwrap()))
		.one(&service.db)
		.await
		.expect("stored endpoint")
		.credential
}

/// A 64-hex device token, as a device holds one.
fn device_token(n: u8) -> String {
	format!("{n:064x}")
}

fn spec() -> Value {
	json!({ "category": "news", "title": "It ships", "body": "Push is here." })
}

fn spec_expiring() -> Value {
	json!({
		"category": "news",
		"title": "It ships",
		"body": "Push is here.",
		"expires_at": (chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339(),
	})
}

/// All three transports, over keys nobody verifies: an EC `.p8` for
/// APNs, an RSA service account for FCM, and a VAPID pair for Web Push.
fn all_transports() -> config::PushConfig {
	let (rsa_pem, _) = test_rsa();
	config::PushConfig {
		apns: Some(apns_config_for(generated_pem().0)),
		fcm: Some(fcm_config_for(&rsa_pem)),
		web: Some(web_config_for(&generated_pem().0).1),
	}
}

// Enrollment.

#[tokio::test(flavor = "multi_thread")]
async fn enrollment_is_authenticated_and_bounded() {
	let mock = Mock::new();
	let service = service(&mock, &all_transports()).await;

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
	assert!(endpoint["invalidated_at"].is_null(), "it starts live");
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
	let service = service(&mock, &config).await;

	let (status, body) = enroll(&service, ALICE, fcm_credential("a-token")).await;
	assert_eq!(status, 400, "{body}");
	assert!(
		body.contains("not configured"),
		"the refusal says the transport is missing: {body}"
	);

	// And with no transports configured at all, so is the configured
	// one: a different service instance in the same process, its own
	// runtime absent, answering for itself — never the other service's
	// credentials.
	let bare = Mock::new();
	let unconfigured = start_push(db().await, None).await;

	let (status, body) = enroll(&unconfigured, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 400, "with push disabled, nothing enrolls: {body}");
	assert!(body.contains("not configured"), "{body}");
	assert!(
		bare.requests().is_empty(),
		"nothing reached any provider through any instance"
	);
}

/// A destination changes hands only to someone who presents the
/// byte-identical credential — the device itself. The same Web Push URL
/// under keys of the caller's own proves nothing and is refused.
#[tokio::test(flavor = "multi_thread")]
async fn a_destination_changes_hands_only_for_the_identical_credential() {
	let mock = Mock::new();
	let service = service(&mock, &all_transports()).await;

	let (status, body) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200, "{body}");
	let alice_endpoint: Value = serde_json::from_str(&body).unwrap();

	// Bob presents the same token: he holds the device now, so it is his,
	// under a fresh endpoint, and alice's is revoked.
	let (status, body) = enroll(&service, BOB, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200, "{body}");
	let bob_endpoint: Value = serde_json::from_str(&body).unwrap();
	assert_ne!(
		bob_endpoint["id"], alice_endpoint["id"],
		"a new endpoint, not the old one"
	);
	assert!(
		endpoints_of(&service, ALICE).await.is_empty(),
		"alice no longer holds the device"
	);
	assert_eq!(endpoints_of(&service, BOB).await.len(), 1);

	// A Web Push subscription changes hands the same way: the same URL
	// and the same keys.
	let endpoint = format!("https://{WEB_HOST}/push/abcdef123456");
	let subscription = web_credential(&endpoint);
	let (status, _) = enroll(&service, ALICE, subscription.clone()).await;
	assert_eq!(status, 200);
	let alice_now = endpoints_of(&service, ALICE).await;
	assert_eq!(alice_now.len(), 1);

	// But where only the keys differ, nothing moves: bob can't take
	// alice's Web Push URL by presenting his own keys for it.
	let (status, body) = enroll(&service, BOB, web_credential(&endpoint)).await;
	assert_eq!(status, 409, "a URL without its keys proves nothing: {body}");
	assert!(
		!body.contains(WEB_HOST) && !body.contains("abcdef"),
		"the refusal carries no endpoint: {body}"
	);
	assert_eq!(
		endpoints_of(&service, ALICE).await,
		alice_now,
		"nothing moved"
	);

	let (status, body) = enroll(&service, BOB, subscription).await;
	assert_eq!(status, 200, "the identical subscription does: {body}");
	assert!(
		endpoints_of(&service, ALICE).await.is_empty(),
		"alice's endpoint was revoked with the handover"
	);
	assert_eq!(endpoints_of(&service, BOB).await.len(), 2);

	// A takeover is an enrollment like any other: it counts against the
	// new owner's bound, and at the bound nothing moves.
	for n in 2..=9_u8 {
		let (status, body) = enroll(&service, BOB, apns_credential(&device_token(n))).await;
		assert_eq!(status, 200, "{body}");
	}

	let (status, _) = enroll(&service, ALICE, apns_credential(&device_token(0xA1))).await;
	assert_eq!(status, 200);
	let (status, body) = enroll(&service, BOB, apns_credential(&device_token(0xA1))).await;
	assert_eq!(status, 400, "bob is at the bound: {body}");
	assert_eq!(
		endpoints_of(&service, ALICE).await.len(),
		1,
		"a refused takeover leaves the device with its owner"
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn revocation_is_owner_only_and_ends_queued_deliveries() {
	let mock = Mock::new();
	let service = service(&mock, &all_transports()).await;

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
	let items = delivery_status(&service, "queued").await;
	assert_eq!(items[0]["attempts"], 0, "a skip spends no attempt");
	assert!(
		mock.requests_to(APNS_HOST).is_empty(),
		"nothing went to the provider"
	);
}

// Endpoint identity, rotation, and re-registration.

#[tokio::test(flavor = "multi_thread")]
async fn a_browser_that_rotates_its_keys_keeps_its_endpoint() {
	let mock = Mock::new();
	let service = service(&mock, &all_transports()).await;

	let endpoint = format!("https://{WEB_HOST}/push/abcdef123456");
	let (_, first, ..) = web_subscription(&endpoint);
	let (status, body) = enroll(&service, ALICE, first).await;
	assert_eq!(status, 200, "{body}");
	let enrolled: Value = serde_json::from_str(&body).unwrap();

	// The same browser through a rotation — same URL, a genuinely new
	// key pair, handed over padded the way other SDKs do — is the same
	// endpoint, and the stored credential rotates in place.
	let (private, _rotated, auth, rotated_public) = web_subscription(&endpoint);
	let padded_form = |unpadded: &str| {
		base64::engine::general_purpose::URL_SAFE.encode(
			URL_SAFE_NO_PAD
				.decode(unpadded)
				.expect("the unpadded form decodes"),
		)
	};
	let rotated_enrollment = json!({
		"web_push": {
			"endpoint": endpoint,
			"keys": {
				"p256dh": padded_form(&rotated_public),
				"auth": padded_form(&URL_SAFE_NO_PAD.encode(auth)),
			},
		}
	});
	let (status, body) = enroll(&service, ALICE, rotated_enrollment).await;
	assert_eq!(status, 200, "{body}");
	let again: Value = serde_json::from_str(&body).unwrap();
	assert_eq!(again["id"], enrolled["id"], "rotation keeps the endpoint");

	let listed = endpoints_of(&service, ALICE).await;
	assert_eq!(listed.len(), 1, "no second endpoint appeared");

	// The stored credential carries the rotated keys, in the service's
	// one canonical unpadded form — so the transport can encrypt for
	// the browser's new keys without ever seeing padding.
	let credential_rows = rows(&service, "SELECT credential FROM push_endpoints").await;
	let stored = text(&credential_rows[0][0]);
	assert!(
		stored.contains(&rotated_public),
		"the stored credential is the rotated one"
	);
	assert!(!stored.contains("=="), "stored keys are unpadded: {stored}");

	// A push now goes to the rotated keys: the wire test's independent
	// receiver opens it with the rotated receiver material.
	let mut delivered = service;
	delivered.resume();
	let (status, _) = send(&delivered, "rotated", spec(), &[ALICE]).await;
	assert_eq!(status, 200);
	let outcomes = settled(&delivered, "rotated").await;
	assert_eq!(outcomes, vec![("accepted".to_owned(), None)]);

	let sent = mock.requests_to(WEB_HOST);
	assert_eq!(sent.len(), 1, "one push, on the rotated credential");
	let _ = private;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_same_token_under_two_platforms_is_two_endpoints() {
	let mock = Mock::new();
	let service = service(&mock, &all_transports()).await;

	// An all-hex string is a legal APNs device token and a legal FCM
	// registration token: the platform is part of the identity, or the
	// second enrollment would look like a takeover of the first.
	let token = format!("{:064x}", 0xAB);
	let (status, _) = enroll(&service, ALICE, apns_credential(&token)).await;
	assert_eq!(status, 200);
	let (status, _) = enroll(&service, ALICE, fcm_credential(&token)).await;
	assert_eq!(status, 200, "a different platform is a different endpoint");

	let listed = endpoints_of(&service, ALICE).await;
	assert_eq!(listed.len(), 2, "{listed:?}");
	assert_ne!(listed[0]["platform"], listed[1]["platform"]);

	// A changed device token is a new destination: a new endpoint, and
	// the old one stays until it is revoked or invalidated.
	let (status, _) = enroll(&service, ALICE, apns_credential(&format!("{:064x}", 0xCD))).await;
	assert_eq!(status, 200);
	assert_eq!(endpoints_of(&service, ALICE).await.len(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_invalidated_destination_requires_its_owner_to_revoke() {
	let mock = Mock::new();
	let mut service = service(&mock, &all_transports()).await;

	let endpoint = format!("https://{WEB_HOST}/push/abcdef123456");
	let (status, body) = enroll(&service, ALICE, web_credential(&endpoint)).await;
	assert_eq!(status, 200, "{body}");
	let enrolled: Value = serde_json::from_str(&body).unwrap();

	// The provider invalidates alice's endpoint through one send.
	mock.scripted(|request, _| {
		if request.url.contains(WEB_HOST) {
			return Scripted::Reply(410, Vec::new(), String::new());
		}

		Scripted::Default
	});
	service.resume();
	let (status, _) = send(&service, "gone", spec(), &[ALICE]).await;
	assert_eq!(status, 200);
	settled(&service, "gone").await;

	let listed = endpoints_of(&service, ALICE).await;
	assert_eq!(listed.len(), 1, "the invalidated endpoint stays listed");
	assert!(
		listed[0]["invalidated_at"].is_string(),
		"with the invalidation visible: {listed:?}"
	);

	// A re-enrollment of the same destination reactivates it: the answer
	// is success and the endpoint is live again, not a silent nothing.
	mock.scripted(|_, _| Scripted::Default);
	let (status, body) = enroll(&service, ALICE, web_credential(&endpoint)).await;
	assert_eq!(status, 200, "{body}");
	let again: Value = serde_json::from_str(&body).unwrap();
	assert_eq!(again["id"], enrolled["id"]);

	let listed = endpoints_of(&service, ALICE).await;
	assert_eq!(listed.len(), 1);
	assert!(listed[0]["invalidated_at"].is_null(), "reactivated");

	// And a delivery goes out on it again.
	let (status, _) = send(&service, "revived", spec(), &[ALICE]).await;
	assert_eq!(status, 200);
	let outcomes = settled(&service, "revived").await;
	assert_eq!(outcomes, vec![("accepted".to_owned(), None)]);

	// Invalidation preserves ownership until explicit revocation.
	let (status, _) = post(
		&service,
		"revoke_push_endpoint",
		json!({ "endpoint": enrolled["id"] }),
		Some(&signed_in(ALICE)),
	)
	.await;
	assert_eq!(status, 200);
	let (status, _) = enroll(&service, ALICE, web_credential(&endpoint)).await;
	assert_eq!(status, 200);
	mock.scripted(|request, _| {
		if request.url.contains(WEB_HOST) {
			return Scripted::Reply(410, Vec::new(), String::new());
		}

		Scripted::Default
	});
	let (status, _) = send(&service, "gone-again", spec(), &[ALICE]).await;
	assert_eq!(status, 200);
	settled(&service, "gone-again").await;

	mock.scripted(|_, _| Scripted::Default);
	let current = endpoints_of(&service, ALICE)
		.await
		.pop()
		.expect("the invalidated endpoint");
	let (status, body) = enroll(&service, BOB, web_credential(&endpoint)).await;
	assert_eq!(
		status, 409,
		"invalidated destinations still belong to their owner: {body}"
	);
	let (status, body) = post(
		&service,
		"revoke_push_endpoint",
		json!({"endpoint": current["id"]}),
		Some(&signed_in(ALICE)),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let (status, body) = enroll(&service, BOB, web_credential(&endpoint)).await;
	assert_eq!(status, 200, "{body}");
	let taken: Value = serde_json::from_str(&body).unwrap();
	assert_ne!(
		taken["id"], current["id"],
		"revocation followed by enrollment creates a fresh ID"
	);

	let bob = endpoints_of(&service, BOB).await;
	assert_eq!(bob.len(), 1);
	assert!(bob[0]["invalidated_at"].is_null(), "live for bob now");
	assert!(
		endpoints_of(&service, ALICE).await.is_empty(),
		"and no longer alice's"
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_endpoint_list_is_bounded_and_paginated() {
	let mock = Mock::new();
	let mut service = service(&mock, &all_transports()).await;

	// Ten live endpoints, the bound.
	for n in 1..=10_u8 {
		let (status, body) = enroll(&service, ALICE, apns_credential(&device_token(n))).await;
		assert_eq!(status, 200, "{body}");
	}

	// The live bound is on live endpoints: invalidating some leaves room
	// for more, and the list keeps showing everything, page by page.
	mock.scripted(|request, _| {
		if request.url.ends_with(&device_token(1))
			|| request.url.ends_with(&device_token(2))
			|| request.url.ends_with(&device_token(3))
		{
			return Scripted::Reply(410, Vec::new(), String::new());
		}

		Scripted::Default
	});
	service.resume();

	for send_key in ["fill-1", "fill-2", "fill-3"] {
		let (status, _) = send(&service, send_key, spec(), &[ALICE]).await;
		assert_eq!(status, 200);
		settled(&service, send_key).await;
	}

	// Three endpoints invalidated, three more enroll within the live
	// bound — invisible to a list that only counted or showed ten.
	mock.scripted(|_, _| Scripted::Default);
	for n in 11..=13_u8 {
		let (status, body) = enroll(&service, ALICE, apns_credential(&device_token(n))).await;
		assert_eq!(
			status, 200,
			"invalidation frees room under the live bound: {body}"
		);
	}

	let listed = endpoints_of(&service, ALICE).await;
	assert_eq!(listed.len(), 13, "invalidated ones included: {listed:?}");
	assert_eq!(
		listed
			.iter()
			.filter(|endpoint| endpoint["invalidated_at"].is_string())
			.count(),
		3
	);

	// The page is bounded: a small page returns a cursor and the rest.
	let (status, body) = post(
		&service,
		"push_endpoints",
		json!({ "page": { "limit": 5 } }),
		Some(&signed_in(ALICE)),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let page: Value = serde_json::from_str(&body).unwrap();
	assert_eq!(page["items"].as_array().map(Vec::len), Some(5));
	assert!(page["has_more"].as_bool().unwrap_or(false), "{page:?}");

	let (status, body) = post(
		&service,
		"push_endpoints",
		json!({
			"page": { "limit": 5, "after": page["end"] }
		}),
		Some(&signed_in(ALICE)),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let rest: Value = serde_json::from_str(&body).unwrap();
	assert_eq!(rest["items"].as_array().map(Vec::len), Some(5));

	// Twelve more endpoints churn: each wave invalidates one live endpoint
	// and enrolls a new one.
	for wave in 0..12_u8 {
		let listed = endpoints_of(&service, ALICE).await;
		let victim_now = listed
			.iter()
			.find(|endpoint| endpoint["invalidated_at"].is_null())
			.expect("a live endpoint to invalidate");
		let token = credential_of(&service, victim_now["id"].as_str().unwrap()).await;
		mock.scripted(move |request, _| {
			if request.url.ends_with(&token) {
				return Scripted::Reply(410, Vec::new(), String::new());
			}

			Scripted::Default
		});
		let (status, _) = send(&service, &format!("prune-{wave}"), spec(), &[ALICE]).await;
		assert_eq!(status, 200);
		settled(&service, &format!("prune-{wave}")).await;

		let victim = 20 + wave;
		let (status, body) = enroll(&service, ALICE, apns_credential(&device_token(victim))).await;
		assert_eq!(status, 200, "{body}");
	}

	// Twenty-five endpoints were enrolled and fifteen invalidated, and the
	// table holds twenty: the ten live ones, and the ten most recently
	// enrolled of the invalidated. Enrolling past the history bound drops
	// the oldest invalidated endpoints, so churn can't grow a person's
	// rows without limit.
	let count = rows(&service, "SELECT count(*) FROM push_endpoints").await;
	let total: usize = text(&count[0][0]).parse().unwrap();
	assert_eq!(total, 20, "ten live, and ten of history");
	let listed = endpoints_of(&service, ALICE).await;
	assert_eq!(listed.len(), 20, "and the whole list is one page");
	assert_eq!(
		listed
			.iter()
			.filter(|endpoint| endpoint["invalidated_at"].is_null())
			.count(),
		10
	);
}

// The app's status query.

#[tokio::test(flavor = "multi_thread")]
async fn only_the_app_reads_delivery_status() {
	let mock = Mock::new();
	let mut service = service(&mock, &all_transports()).await;

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
	let mut service = service(&mock, &all_transports()).await;

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

/// A topic send's fanout enqueues the same per-endpoint deliveries a
/// recipients send does: one per enrolled endpoint of everyone the
/// fanout matched, attempted when the notification is visible.
#[tokio::test(flavor = "multi_thread")]
async fn a_topic_send_fans_out_one_delivery_per_endpoint() {
	let mock = Mock::new();
	let mut service = service(&mock, &all_transports()).await;

	for n in 1..3_u8 {
		let (status, body) = enroll(&service, ALICE, apns_credential(&device_token(n))).await;
		assert_eq!(status, 200, "{body}");
	}

	let (status, body) = post(
		&service,
		"subscribe_to_topic",
		json!({ "topic": "news.push", "persons": [ALICE, BOB] }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");

	service.resume();

	let (status, body) = post(
		&service,
		"send",
		json!({
			"idempotency_key": "topic-fanout",
			"spec": spec(),
			"target": { "topic": "news.push" },
		}),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");

	// Bob enrolled nothing, so only alice's two endpoints carry it.
	let outcomes = settled(&service, "topic-fanout").await;
	assert_eq!(outcomes.len(), 2, "{outcomes:?}");
	assert!(
		outcomes
			.iter()
			.all(|(state, reason)| state == "accepted" && reason.is_none()),
		"{outcomes:?}"
	);

	let sent = mock.requests_to(APNS_HOST);
	assert_eq!(sent.len(), 2, "one request per endpoint");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_apns_wire_carries_a_verified_cached_provider_token_and_the_expiry() {
	let mock = Mock::new();
	let (key_p8, point) = generated_pem();
	let config = config::PushConfig {
		apns: Some(apns_config_for(key_p8)),
		fcm: None,
		web: None,
	};
	let mut service = service(&mock, &config).await;

	let (status, _) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200);
	service.resume();

	let (status, _) = send(&service, "signed", spec_expiring(), &[ALICE]).await;
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

	// The expiry rides the wire: a provider holding the message past the
	// send's life would deliver something already gone.
	let expiration = sent[0]
		.header("apns-expiration")
		.expect("an apns-expiration for an expiring send")
		.parse::<i64>()
		.unwrap();
	let expires_at = (chrono::Utc::now() + chrono::Duration::hours(2)).timestamp();
	assert!(
		(expiration - expires_at).abs() < 300,
		"the expiration is the send's: {expiration} vs {expires_at}"
	);
	assert!(
		sent[1].header("apns-expiration").is_none(),
		"a send without an expiry carries none"
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_fcm_wire_exchanges_a_verified_rs256_account_jwt() {
	let mock = Mock::new();
	let (rsa_pem, rsa_public) = test_rsa();
	let config = config::PushConfig {
		apns: None,
		fcm: Some(fcm_config_for(&rsa_pem)),
		web: None,
	};
	let mut service = service(&mock, &config).await;

	let (status, _) = enroll(&service, ALICE, fcm_credential("fcm-token-1")).await;
	assert_eq!(status, 200);
	service.resume();

	// The token endpoint answers a short-lived access token. The script
	// verifies each assertion independently — RS256, ring's own RSA
	// verifier against the key's public half — and checks the exact
	// claims the grant needs. The account's JSON names a token_uri of
	// its own; the exchange must go to the fixed first-party endpoint
	// regardless.
	let exchanges = Arc::new(AtomicUsize::new(0));
	let counting = Arc::clone(&exchanges);
	mock.scripted(move |request, _| {
		if !request.url.contains(FCM_TOKEN_HOST) {
			return Scripted::Default;
		}

		assert_eq!(
			request.url, "https://oauth2.googleapis.com/token",
			"the exchange is fixed first party, never the JSON's token_uri: {}",
			request.url
		);
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

		let now = leptoskit_db::now().timestamp();
		let (header, claims) = verify_rs256(assertion, &rsa_public);
		assert_eq!(header["alg"], "RS256", "Google's OAuth needs RS256");
		assert_eq!(header["typ"], "JWT");
		assert_eq!(claims["iss"], "push@leptoskit-push.iam.gserviceaccount.com");
		assert_eq!(claims["aud"], "https://oauth2.googleapis.com/token");
		assert_eq!(
			claims["scope"],
			"https://www.googleapis.com/auth/firebase.messaging"
		);
		assert!((claims["iat"].as_i64().unwrap() - now).abs() < 60);
		assert_eq!(
			claims["exp"].as_i64().unwrap() - claims["iat"].as_i64().unwrap(),
			3_600,
			"the assertion lives an hour"
		);

		counting.fetch_add(1, Ordering::SeqCst);
		Scripted::Reply(
			200,
			vec![("content-type".to_owned(), "application/json".to_owned())],
			json!({ "access_token": "ya29.test-token", "expires_in": 3600 }).to_string(),
		)
	});

	let (status, _) = send(&service, "fcm-sent", spec_expiring(), &[ALICE]).await;
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

	// The send's expiry is the platforms' TTL and the iOS expiry, so a
	// provider can't deliver past it.
	let ttl = message["message"]["android"]["ttl"]
		.as_str()
		.expect("an android ttl for an expiring send");
	let secs: i64 = ttl.trim_end_matches('s').parse().expect("proto seconds");
	assert!(
		(secs - 2 * 3_600).abs() < 300,
		"the ttl is the send's remaining life: {ttl}"
	);
	let apns_expiration = message["message"]["apns"]["headers"]["apns-expiration"]
		.as_str()
		.expect("an apns-expiration for an expiring send")
		.parse::<i64>()
		.unwrap();
	assert!(
		(apns_expiration - (chrono::Utc::now() + chrono::Duration::hours(2)).timestamp()).abs()
			< 300,
		"the apns expiry is the send's"
	);

	// The access token is cached: a second send doesn't re-exchange.
	let before = exchanges.load(Ordering::SeqCst);
	assert_eq!(before, 1, "one exchange so far");
	let (status, _) = send(&service, "fcm-again", spec(), &[ALICE]).await;
	assert_eq!(status, 200);
	settled(&service, "fcm-again").await;
	assert_eq!(
		exchanges.load(Ordering::SeqCst),
		before,
		"the cached access token serves the second send"
	);
}

/// A send without an expiry carries no TTLs, and the caller's data drops
/// the envelope's own key and the platform's reserved words rather than
/// sending them or being overwritten by them.
#[tokio::test(flavor = "multi_thread")]
async fn the_fcm_data_map_drops_reserved_keys_and_unneeded_ttls() {
	let mock = Mock::new();
	let (rsa_pem, _) = test_rsa();
	let config = config::PushConfig {
		apns: None,
		fcm: Some(fcm_config_for(&rsa_pem)),
		web: None,
	};
	let mut service = service(&mock, &config).await;

	let (status, _) = enroll(&service, ALICE, fcm_credential("fcm-token-1")).await;
	assert_eq!(status, 200);
	mock.scripted(|request, _| {
		if request.url.contains(FCM_TOKEN_HOST) {
			return Scripted::Reply(
				200,
				Vec::new(),
				json!({ "access_token": "ya29.test-token", "expires_in": 3600 }).to_string(),
			);
		}

		Scripted::Default
	});
	service.resume();

	let (status, _) = send(
		&service,
		"fcm-reserved",
		json!({
			"category": "news",
			"title": "It ships",
			"body": "Clean.",
			"data": json!({
				"lkn": "the attacker's id",
				"from": "spoofed",
				"message_type": "spoofed",
				"google.sneaky": "no",
				"gcm.sneaky": "no",
				"gcm.notification.sneaky": "no",
				"real": "kept"
			})
			.to_string(),
		}),
		&[ALICE],
	)
	.await;
	assert_eq!(status, 200);
	settled(&service, "fcm-reserved").await;

	let reserved = mock
		.requests_to(FCM_HOST)
		.into_iter()
		.find(|seen| seen.body.windows(4).any(|w| w == b"real"))
		.expect("the reserved-key send went out");
	let message = reserved.json();
	assert_eq!(message["message"]["data"]["real"], "kept");
	assert_eq!(
		message["message"]["data"]["lkn"], message["message"]["android"]["collapse_key"],
		"the id key is the service's own, never the caller's"
	);
	assert!(
		message["message"]["data"].get("from").is_none()
			&& message["message"]["data"].get("message_type").is_none()
			&& message["message"]["data"].get("google.sneaky").is_none()
			&& message["message"]["data"].get("gcm.sneaky").is_none()
			&& message["message"]["data"]
				.get("gcm.notification.sneaky")
				.is_none(),
		"the platform's reserved words don't ride: {}",
		message["message"]["data"]
	);
	assert!(
		message["message"]["android"].get("ttl").is_none()
			&& message["message"]["apns"]["headers"]
				.get("apns-expiration")
				.is_none(),
		"a send without an expiry carries no TTLs"
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_broken_fcm_account_stalls_the_delivery_without_failing_it() {
	let mock = Mock::new();
	let (rsa_pem, _) = test_rsa();
	let config = config::PushConfig {
		apns: None,
		fcm: Some(fcm_config_for(&rsa_pem)),
		web: None,
	};
	let mut service = service(&mock, &config).await;

	let (status, _) = enroll(&service, ALICE, fcm_credential("fcm-token-1")).await;
	assert_eq!(status, 200);
	service.resume();

	// Every malformed answer an exchange can give — a refused exchange,
	// a body with no access_token, a nonsense lifetime — stalls the
	// delivery: no panic, no rejection, no invalidation, no attempt
	// spent, retrying at its calm pace.
	for (name, answer) in [
		(
			"refused",
			json!({ "error": "unauthorized_client" }).to_string(),
		),
		("no access_token", json!({ "expires_in": 3600 }).to_string()),
		(
			"absurd expires_in",
			json!({ "access_token": "ya29.lies", "expires_in": -1 }).to_string(),
		),
	] {
		mock.scripted(move |request, _| {
			if request.url.contains(FCM_TOKEN_HOST) {
				return Scripted::Reply(
					if name == "refused" { 400 } else { 200 },
					Vec::new(),
					answer.clone(),
				);
			}

			Scripted::Default
		});

		let (status, _) = send(&service, &format!("stall-{name}"), spec(), &[ALICE]).await;
		assert_eq!(status, 200);

		let items = wait_not_enqueued(&service, &format!("stall-{name}")).await;
		assert_eq!(
			items[0]["state"], "retrying",
			"{name}: the delivery waits: {items:?}"
		);
		assert_eq!(items[0]["attempts"], 0, "{name}: no attempt spent");
		assert!(
			mock.requests_to(FCM_HOST).is_empty(),
			"{name}: nothing reached the send endpoint"
		);

		let invalidated = rows(
			&service,
			"SELECT count(*) FROM push_endpoints WHERE invalidated_at IS NOT NULL",
		)
		.await;
		assert_eq!(text(&invalidated[0][0]), "0", "{name}: nothing invalidated");
	}
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_authorization_refreshes_once_and_then_stalls() {
	let mock = Mock::new();
	let (rsa_pem, _) = test_rsa();
	let config = config::PushConfig {
		apns: None,
		fcm: Some(fcm_config_for(&rsa_pem)),
		web: None,
	};
	let mut service = service(&mock, &config).await;

	let (status, _) = enroll(&service, ALICE, fcm_credential("fcm-token-1")).await;
	assert_eq!(status, 200);
	service.resume();

	// A stale cached token is refused with 401: the transport drops the
	// cache, exchanges once more, and the retried send goes through. The
	// first exchange answers yesterday's token; only the second answers
	// one the provider still knows.
	let exchanges = Arc::new(AtomicUsize::new(0));
	let counting = Arc::clone(&exchanges);
	mock.scripted(move |request, _| {
		if request.url.contains(FCM_TOKEN_HOST) {
			let count = counting.fetch_add(1, Ordering::SeqCst) + 1;
			let token = if count == 1 {
				"ya29.stale"
			} else {
				"ya29.fresh"
			};

			return Scripted::Reply(
				200,
				Vec::new(),
				json!({ "access_token": token, "expires_in": 3600 }).to_string(),
			);
		}

		let authorization = wire_header(request, "authorization").unwrap_or("");

		if authorization == "Bearer ya29.fresh" {
			return Scripted::Default;
		}

		Scripted::Reply(
			401,
			Vec::new(),
			json!({ "error": { "status": "UNAUTHENTICATED" } }).to_string(),
		)
	});

	let (status, _) = send(&service, "stale", spec(), &[ALICE]).await;
	assert_eq!(status, 200);
	let outcomes = settled_rewinding(&service, "stale").await;
	assert_eq!(
		outcomes,
		vec![("accepted".to_owned(), None)],
		"a refreshed token gets the send through"
	);
	assert_eq!(
		exchanges.load(Ordering::SeqCst),
		2,
		"the stale token's exchange, and one refresh: bounded"
	);

	// A token refused when still fresh is the operator's account: the
	// delivery stalls — retrying, attempts unspent — rather than ending
	// or invalidating the endpoint.
	let fresh_refusals = Arc::new(AtomicUsize::new(0));
	let refusing = Arc::clone(&fresh_refusals);
	mock.scripted(move |request, _| {
		if request.url.contains(FCM_TOKEN_HOST) {
			return Scripted::Reply(
				200,
				Vec::new(),
				json!({ "access_token": REFUSED_ACCESS_TOKEN, "expires_in": 3600 }).to_string(),
			);
		}

		refusing.fetch_add(1, Ordering::SeqCst);
		Scripted::Reply(
			401,
			Vec::new(),
			json!({ "error": { "status": "UNAUTHENTICATED" } }).to_string(),
		)
	});

	let (status, _) = send(&service, "unfixable", spec(), &[ALICE]).await;
	assert_eq!(status, 200);

	let items = wait_not_enqueued(&service, "unfixable").await;
	assert_eq!(
		items[0]["state"], "retrying",
		"a fresh 401 is counted: {items:?}"
	);
	assert_eq!(items[0]["attempts"], 1, "{items:?}");
	assert_eq!(
		fresh_refusals.load(Ordering::SeqCst),
		1,
		"one push per durable attempt; refresh does not spend a push"
	);

	let invalidated = rows(
		&service,
		"SELECT count(*) FROM push_endpoints WHERE invalidated_at IS NOT NULL",
	)
	.await;
	assert_eq!(
		text(&invalidated[0][0]),
		"0",
		"the endpoint is not at fault"
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_generic_fcm_404_does_not_invalidate_the_device() {
	let mock = Mock::new();
	let (rsa_pem, _) = test_rsa();
	let config = config::PushConfig {
		apns: None,
		fcm: Some(fcm_config_for(&rsa_pem)),
		web: None,
	};
	let mut service = service(&mock, &config).await;

	let (status, _) = enroll(&service, ALICE, fcm_credential("fcm-token-1")).await;
	assert_eq!(status, 200);
	service.resume();
	mock.scripted(passing_exchange());

	// A 404 whose error detail says nothing about the token is the
	// operator's URL naming, not a dead device: the endpoint stays.
	mock.scripted(|request, _| {
		if request.url.contains(FCM_TOKEN_HOST) {
			return Scripted::Reply(
				200,
				Vec::new(),
				json!({ "access_token": "ya29.lost", "expires_in": 3600 }).to_string(),
			);
		}

		Scripted::Reply(
			404,
			Vec::new(),
			json!({ "error": { "status": "NOT_FOUND", "message": "project not found" } })
				.to_string(),
		)
	});

	let (status, _) = send(&service, "lost-project", spec(), &[ALICE]).await;
	assert_eq!(status, 200);

	let items = wait_not_enqueued(&service, "lost-project").await;
	assert_eq!(
		items[0]["state"], "retrying",
		"an operator's 404 stalls rather than ending: {items:?}"
	);

	let endpoints = endpoints_of(&service, ALICE).await;
	assert_eq!(endpoints.len(), 1);
	assert!(
		endpoints[0]["invalidated_at"].is_null(),
		"a generic 404 invalidates nothing"
	);

	// The error detail that says UNREGISTERED — whatever the status — is
	// the one that invalidates.
	mock.scripted(|request, _| {
		if request.url.contains(FCM_TOKEN_HOST) {
			return Scripted::Reply(
				200,
				Vec::new(),
				json!({ "access_token": "ya29.dead", "expires_in": 3600 }).to_string(),
			);
		}

		Scripted::Reply(
			404,
			Vec::new(),
			json!({ "error": { "code": 404, "status": "NOT_FOUND", "details": [{
				"@type": "type.googleapis.com/google.firebase.fcm.v1.FcmError",
				"errorCode": "UNREGISTERED"
			}] } })
			.to_string(),
		)
	});

	let (status, _) = send(&service, "dead-token", spec(), &[ALICE]).await;
	assert_eq!(status, 200);
	let outcomes = settled(&service, "dead-token").await;
	assert_eq!(
		outcomes,
		vec![("rejected".to_owned(), Some("unregistered".to_owned()))],
		"{outcomes:?}"
	);

	let endpoints = endpoints_of(&service, ALICE).await;
	assert!(
		endpoints[0]["invalidated_at"].is_string(),
		"the UNREGISTERED detail invalidates"
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_web_push_wire_carries_vapid_and_an_independently_opened_body() {
	let mock = Mock::new();
	let (key_pem, _) = generated_pem();
	let (vapids, web) = web_config_for(&key_pem);
	let config = config::PushConfig {
		apns: None,
		fcm: None,
		web: Some(web),
	};
	let mut service = service(&mock, &config).await;

	let endpoint = format!("https://{WEB_HOST}/push/abcdef123456");
	let (receiver, credential, auth_secret, _) = web_subscription(&endpoint);
	let (status, body) = enroll(&service, ALICE, credential).await;
	assert_eq!(status, 200, "{body}");
	service.resume();

	let (status, _) = send(&service, "encrypted", spec_expiring(), &[ALICE]).await;
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

	// The TTL is the send's remaining life.
	let ttl = sent[0]
		.header("ttl")
		.expect("a ttl")
		.parse::<i64>()
		.unwrap();
	assert!(
		(ttl - 2 * 3_600).abs() < 300,
		"the ttl is the send's remaining life: {ttl}"
	);
	assert_eq!(
		sent[0].header("topic").expect("a topic").len(),
		32,
		"a topic is a UUID without its dashes"
	);

	// The VAPID header: `vapid t=<JWT>, k=<public key>`, aud the
	// endpoint's origin, sub the operator's contact, exp ahead, and the
	// k half matching the configured public key. Verified with this
	// test's own ES256 verifier, independent of the crate that signed.
	let vapid = sent[0]
		.header("authorization")
		.expect("vapid")
		.strip_prefix("vapid ")
		.expect("the scheme");
	let (token, public) = vapid.split_once(", k=").expect("t= and k= halves");
	let token = token.strip_prefix("t=").expect("the token half");
	let (header, claims) = verify_jwt(token, &vapids);
	assert_eq!(header["typ"], "JWT");
	assert_eq!(header["alg"], "ES256", "VAPID is ES256 only");
	assert_eq!(claims["aud"], format!("https://{WEB_HOST}"));
	assert_eq!(claims["sub"], "mailto:push@leptoskit.dev");
	assert!(claims["exp"].as_i64().unwrap() > leptoskit_db::now().timestamp());
	assert!(
		claims["exp"].as_i64().unwrap() - leptoskit_db::now().timestamp() <= 24 * 3_600,
		"RFC 8292 caps a token's life at a day"
	);
	assert_eq!(
		URL_SAFE_NO_PAD.decode(public).unwrap(),
		vapids,
		"k= is the configured public key"
	);

	// The body: an aes128gcm envelope this test opens on its own
	// receiver side — its own ring-based derivation, not the sender
	// library's — to the payload a service worker would read.
	let body = sent[0].body.clone();
	assert!(body.len() <= 4_096, "{} bytes", body.len());
	let receiver_point = {
		// The receiver's public point is the subscription's p256dh; take
		// it from what was enrolled.
		let stored = rows(&service, "SELECT credential FROM push_endpoints").await;
		let subscription: Value = serde_json::from_str(&text(&stored[0][0])).unwrap();
		URL_SAFE_NO_PAD
			.decode(subscription["keys"]["p256dh"].as_str().unwrap())
			.unwrap()
	};
	let opened = open_aes128gcm(&body, receiver, &receiver_point, &auth_secret);
	let payload: Value = serde_json::from_slice(&opened).unwrap();
	assert_eq!(payload["title"], "It ships");
	assert_eq!(payload["body"], "Push is here.");
	assert!(payload["lkn"].as_str().is_some(), "the id rides");
	assert!(
		payload["lk"].is_null()
			|| payload["lk"]
				.as_object()
				.is_some_and(serde_json::Map::is_empty),
		"a send with no data carries none"
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn every_transport_on_one_send_reaches_its_own_provider() {
	let mock = Mock::new();
	let mut service = service(&mock, &all_transports()).await;

	let (status, _) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200);
	let (status, _) = enroll(&service, ALICE, fcm_credential("fcm-token-1")).await;
	assert_eq!(status, 200);
	let endpoint = format!("https://{WEB_HOST}/push/abcdef123456");
	let (status, _) = enroll(&service, ALICE, web_credential(&endpoint)).await;
	assert_eq!(status, 200);

	// The FCM transport exchanges its access token on the way through.
	mock.scripted(|request, _| {
		if request.url.contains(FCM_TOKEN_HOST) {
			return Scripted::Reply(
				200,
				vec![("content-type".to_owned(), "application/json".to_owned())],
				json!({ "access_token": "ya29.everywhere", "expires_in": 3600 }).to_string(),
			);
		}

		Scripted::Default
	});

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

#[tokio::test(flavor = "multi_thread")]
async fn two_services_in_one_process_keep_their_own_credentials() {
	// The process-global hazard, answered: two notifications services
	// embedded in one process, each with its own FCM account, each send
	// signed by its own account only.
	let one_mock = Mock::new();
	let (one_pem, one_public) = test_rsa();
	let one = start(
		db().await,
		&config::PushConfig {
			apns: None,
			fcm: Some(fcm_config_for(&one_pem)),
			web: None,
		},
		&one_mock,
	)
	.await;

	let two_mock = Mock::new();
	let (two_pem, two_public) = test_rsa();
	let mut two = start(
		db().await,
		&config::PushConfig {
			apns: None,
			fcm: Some(config::FcmConfig {
				service_account_json: json!({
					"type": "service_account",
					"project_id": "leptoskit-other",
					"private_key": two_pem,
					"client_email": "push@leptoskit-other.iam.gserviceaccount.com",
					"token_uri": "https://evil.example.test/token",
				})
				.to_string(),
			}),
			web: None,
		},
		&two_mock,
	)
	.await;

	let (status, _) = enroll(&one, ALICE, fcm_credential("token-one")).await;
	assert_eq!(status, 200);
	let (status, _) = enroll(&two, BOB, fcm_credential("token-two")).await;
	assert_eq!(status, 200);

	one_mock.scripted(|request, _| {
		if request.url.contains(FCM_TOKEN_HOST) {
			return Scripted::Reply(
				200,
				Vec::new(),
				json!({ "access_token": "ya29.one", "expires_in": 3600 }).to_string(),
			);
		}

		Scripted::Default
	});
	two_mock.scripted(|request, _| {
		if request.url.contains(FCM_TOKEN_HOST) {
			return Scripted::Reply(
				200,
				Vec::new(),
				json!({ "access_token": "ya29.two", "expires_in": 3600 }).to_string(),
			);
		}

		Scripted::Default
	});

	// Both workers and sends run concurrently, each with its own runtime.
	let mut one = one;
	one.resume();
	two.resume();
	let (first, second) = tokio::join!(
		send(&one, "mine", spec(), &[ALICE]),
		send(&two, "theirs", spec(), &[BOB])
	);
	assert_eq!(first.0, 200);
	assert_eq!(second.0, 200);
	tokio::join!(settled(&one, "mine"), settled(&two, "theirs"));

	// Each service's assertion is its own account's, and each send
	// carries its own access token.
	let one_assertion =
		String::from_utf8(one_mock.requests_to(FCM_TOKEN_HOST)[0].body.clone()).unwrap();
	let assertion = one_assertion.split("assertion=").nth(1).unwrap().to_owned();
	let (_, claims) = verify_rs256(&assertion, &one_public);
	assert_eq!(
		claims["iss"], "push@leptoskit-push.iam.gserviceaccount.com",
		"service one signs as itself"
	);

	let two_assertion =
		String::from_utf8(two_mock.requests_to(FCM_TOKEN_HOST)[0].body.clone()).unwrap();
	let assertion = two_assertion.split("assertion=").nth(1).unwrap().to_owned();
	let (_, claims) = verify_rs256(&assertion, &two_public);
	assert_eq!(
		claims["iss"], "push@leptoskit-other.iam.gserviceaccount.com",
		"service two signs as itself, never with one's account"
	);
	assert_eq!(
		two_mock.requests_to(FCM_HOST)[0].url,
		"https://fcm.googleapis.com/v1/projects/leptoskit-other/messages:send",
		"service two sends under its own project"
	);

	assert_eq!(
		one_mock.requests_to(FCM_HOST)[0].header("authorization"),
		Some("Bearer ya29.one")
	);
	assert_eq!(
		two_mock.requests_to(FCM_HOST)[0].header("authorization"),
		Some("Bearer ya29.two")
	);
}

// Transient failures and retries.

#[tokio::test(flavor = "multi_thread")]
async fn transient_failures_retry_and_recover() {
	let mock = Mock::new();
	let mut service = service(&mock, &all_transports()).await;

	let (status, _) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200);

	// A rate limit with a Retry-After, honored over the engine's own
	// backoff; then a server error; then success.
	mock.scripted(|request, index| {
		if !request.url.contains(APNS_HOST) {
			return Scripted::Default;
		}

		match index {
			0 => {
				Scripted::Reply(
					429,
					vec![("retry-after".to_owned(), "1".to_owned())],
					json!({ "reason": "TooManyRequests" }).to_string(),
				)
			}
			1 => Scripted::Reply(503, Vec::new(), String::new()),
			_ => Scripted::Default,
		}
	});
	service.resume();
	let (status, _) = send(&service, "flaky", spec(), &[ALICE]).await;
	assert_eq!(status, 200);

	let outcomes = settled_rewinding(&service, "flaky").await;
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
	let mut service = service(&mock, &all_transports()).await;

	let (status, _) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200);

	mock.scripted(|request, index| {
		if request.url.contains(APNS_HOST) {
			if index == 0 {
				return Scripted::Wire(WireError::Unreachable);
			}

			return Scripted::Default;
		}

		Scripted::Default
	});
	service.resume();
	let (status, _) = send(&service, "unplugged", spec(), &[ALICE]).await;
	assert_eq!(status, 200);

	let outcomes = settled_rewinding(&service, "unplugged").await;
	assert_eq!(
		outcomes,
		vec![("accepted".to_owned(), None)],
		"{outcomes:?}"
	);
	assert_eq!(mock.requests_to(APNS_HOST).len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn eight_attempts_are_eight_not_nine() {
	let mock = Mock::new();
	let mut service = service(&mock, &all_transports()).await;

	let (status, _) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200);

	// Every attempt answers a rate limit that asks for one second, until
	// the bound spends itself: eight sends on the wire, not nine.
	mock.scripted(|request, _| {
		if request.url.contains(APNS_HOST) {
			return Scripted::Reply(
				429,
				vec![("retry-after".to_owned(), "1".to_owned())],
				json!({ "reason": "TooManyRequests" }).to_string(),
			);
		}

		Scripted::Default
	});
	service.resume();
	let (status, _) = send(&service, "doomed", spec(), &[ALICE]).await;
	assert_eq!(status, 200);

	let outcomes = settled_rewinding(&service, "doomed").await;
	assert_eq!(
		outcomes,
		vec![("rejected".to_owned(), Some("attempts_exhausted".to_owned()))],
		"{outcomes:?}"
	);
	assert_eq!(
		mock.requests_to(APNS_HOST).len(),
		8,
		"the bound is eight actual attempts"
	);

	let items = delivery_status(&service, "doomed").await;
	assert_eq!(items[0]["attempts"], 8);
}

// Permanent failures and invalidation.

#[tokio::test(flavor = "multi_thread")]
async fn an_unregistered_credential_ends_and_invalidates() {
	let mock = Mock::new();
	let mut service = service(&mock, &all_transports()).await;

	// Two endpoints, the second of which the provider no longer knows.
	let (status, _) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200);
	let (status, _) = enroll(&service, ALICE, apns_credential(&device_token(2))).await;
	assert_eq!(status, 200);

	// Two sends, so the unregistered endpoint holds two deliveries. The
	// workers start after both sends commit, and run one round at a time,
	// so the second delivery witnesses the first one's invalidation.
	mock.scripted(|request, _| {
		if !request.url.contains(APNS_HOST) {
			return Scripted::Default;
		}

		if request.url.ends_with(&device_token(2)) {
			return Scripted::Reply(
				410,
				Vec::new(),
				json!({ "reason": "Unregistered" }).to_string(),
			);
		}

		Scripted::Default
	});

	let (status, _) = send(&service, "first", spec(), &[ALICE]).await;
	assert_eq!(status, 200);
	let (status, _) = send(&service, "second", spec(), &[ALICE]).await;
	assert_eq!(status, 200);
	service.jobs = Some(
		leptoskit_notifications::service::workers_with_push(
			service.db.clone(),
			service.push.clone(),
		)
		.queue("push", 1)
		.poll(Duration::from_millis(50))
		.start(),
	);

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
	assert!(
		items
			.iter()
			.all(|item| item["attempts"] == 0 || item["state"] != "skipped"),
		"skips spend no attempts: {items:?}"
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn provider_rejections_end_without_invalidating() {
	let mock = Mock::new();
	let mut service = service(&mock, &all_transports()).await;

	let (status, _) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200);

	// A request APNs calls malformed is the send's own: the delivery
	// ends, the endpoint stays.
	mock.scripted(|request, _| {
		if request.url.contains(APNS_HOST) {
			return Scripted::Reply(
				400,
				Vec::new(),
				json!({ "reason": "BadPriority" }).to_string(),
			);
		}

		Scripted::Default
	});
	service.resume();
	let (status, _) = send(&service, "refused", spec(), &[ALICE]).await;
	assert_eq!(status, 200);

	let outcomes = settled(&service, "refused").await;
	assert_eq!(
		outcomes,
		vec![("rejected".to_owned(), Some("provider_rejected".to_owned()))],
		"{outcomes:?}"
	);

	let endpoints = endpoints_of(&service, ALICE).await;
	assert_eq!(endpoints.len(), 1);
	assert!(
		endpoints[0]["invalidated_at"].is_null(),
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
	let mut service = service(&mock, &config).await;

	let endpoint = format!("https://{WEB_HOST}/push/abcdef123456");
	let (status, _) = enroll(&service, ALICE, web_credential(&endpoint)).await;
	assert_eq!(status, 200);

	mock.scripted(|request, _| {
		if request.url.contains(WEB_HOST) {
			return Scripted::Reply(410, Vec::new(), String::new());
		}

		Scripted::Default
	});
	service.resume();
	let (status, _) = send(&service, "gone", spec(), &[ALICE]).await;
	assert_eq!(status, 200);

	let outcomes = settled(&service, "gone").await;
	assert_eq!(
		outcomes,
		vec![("rejected".to_owned(), Some("unregistered".to_owned()))],
		"{outcomes:?}"
	);
	let endpoints = endpoints_of(&service, ALICE).await;
	assert!(endpoints[0]["invalidated_at"].is_string());
}

// Payload limits, on the encoded bytes.

#[tokio::test(flavor = "multi_thread")]
async fn oversized_payloads_drop_data_first_and_end_clearly() {
	let mock = Mock::new();
	let mut service = service(&mock, &all_transports()).await;

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

	// A send too big even for the minimal envelope — a title and body
	// whose quotes JSON escapes to past Apple's 4 KiB — ends oversized,
	// with no request at all.
	let too_big = json!({
		"category": "news",
		"title": "\"".repeat(200),
		"body": "\"".repeat(2_000),
	});
	let (status, body) = send(&service, "too-big", too_big, &[ALICE]).await;
	assert_eq!(status, 200, "{body}");

	let outcomes = settled(&service, "too-big").await;
	assert_eq!(
		outcomes,
		vec![("rejected".to_owned(), Some("oversized".to_owned()))],
		"{outcomes:?}"
	);
	assert_eq!(
		mock.requests_to(APNS_HOST).len(),
		1,
		"an oversized payload never reaches the provider"
	);

	// The minimal envelope, when it does fit, keeps the collapse key and
	// the apns headers: a fallback that dropped them would let a repeat
	// stack banners the full payload would have replaced.
	let mock3 = Mock::new();
	let (rsa_pem3, _) = test_rsa();
	let mut service3 = start(
		db().await,
		&config::PushConfig {
			apns: None,
			fcm: Some(fcm_config_for(&rsa_pem3)),
			web: None,
		},
		&mock3,
	)
	.await;
	let (status, _) = enroll(&service3, ALICE, fcm_credential("fcm-token-3")).await;
	assert_eq!(status, 200);
	mock3.scripted(|request, _| {
		if request.url.contains(FCM_TOKEN_HOST) {
			return Scripted::Reply(
				200,
				Vec::new(),
				json!({ "access_token": "ya29.fallback", "expires_in": 3600 }).to_string(),
			);
		}

		Scripted::Default
	});
	service3.resume();

	let (status, _) = send(
		&service3,
		"fallback-fits",
		json!({
			"category": "news",
			"title": "It ships",
			"body": "z".repeat(1_500),
			"data": json!({ "x": "x".repeat(4_000) }).to_string(),
		}),
		&[ALICE],
	)
	.await;
	assert_eq!(status, 200);
	let outcomes = settled(&service3, "fallback-fits").await;
	assert_eq!(outcomes, vec![("accepted".to_owned(), None)]);

	let sent = mock3.requests_to(FCM_HOST);
	assert_eq!(sent.len(), 1);
	let message = sent[0].json();
	assert!(
		message["message"]["data"].get("x").is_none(),
		"the data dropped"
	);
	assert!(
		message["message"]["android"]["collapse_key"]
			.as_str()
			.is_some_and(|key| !key.is_empty()),
		"the fallback keeps the collapse key"
	);
	assert!(
		message["message"]["apns"]["headers"]["apns-collapse-id"]
			.as_str()
			.is_some_and(|id| !id.is_empty()),
		"the fallback keeps the apns collapse header"
	);
}

// The schedule, cancellation, expiry, and the latest opt-out, at the
// attempt.

#[tokio::test(flavor = "multi_thread")]
async fn cancelled_sends_never_push() {
	let mock = Mock::new();
	let mut service = service(&mock, &all_transports()).await;

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
	let mut service = service(&mock, &all_transports()).await;

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
	let mut service = service(&mock, &all_transports()).await;

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
	let items = delivery_status(&service, "fleeting").await;
	assert_eq!(items[0]["attempts"], 0, "an expired skip spends no attempt");
	assert!(items[0]["attempted_at"].is_null(), "nothing was attempted");
	assert!(mock.requests_to(APNS_HOST).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_opt_out_made_in_the_queue_decides_at_the_attempt() {
	let mock = Mock::new();
	let mut service = service(&mock, &all_transports()).await;

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

#[test]
fn push_job_arguments_ignore_the_old_instance_field() {
	let job: leptoskit_notifications::push::PushSend = serde_json::from_value(json!({
		"send_key": "legacy-send",
		"round": 2,
		"instance": "obsolete-instance"
	}))
	.unwrap();
	assert_eq!(
		serde_json::to_value(job).unwrap(),
		json!({"send_key": "legacy-send", "round": 2})
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restart_recovers_pending_work() {
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

		let jobs = rpc::server::jobs::JobRow::query()
			.filter(|row| row.name.eq("push_send"))
			.all(&service.db.system("inspect durable push arguments"))
			.await
			.unwrap();
		assert_eq!(jobs.len(), 1);
		assert_eq!(
			jobs[0].arguments.0,
			json!({"send_key": "across-restart", "round": 0})
		);
		assert!(!format!("{:?}", jobs[0].arguments).contains(&url));

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
	// its round-zero job are there. Only a freshly allocated runtime's
	// provider receives the recovered work.
	let fresh_mock = Arc::new(Mock::new());
	let db = Db::connect(&url).await.expect("the database opens again");
	let mut restarted = start(db, &all_transports(), &fresh_mock).await;
	restarted.resume();
	let outcomes = settled(&restarted, "across-restart").await;
	assert_eq!(
		outcomes,
		vec![("accepted".to_owned(), None)],
		"{outcomes:?}"
	);
	assert_eq!(fresh_mock.requests_to(APNS_HOST).len(), 1);
	assert!(mock.requests_to(APNS_HOST).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_duplicate_send_enrolls_each_endpoint_once() {
	let mock = Mock::new();
	let mut service = service(&mock, &all_transports()).await;

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
	let mut service = service(&mock, &all_transports()).await;

	let (status, _) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200);
	let (status, _) = enroll(&service, BOB, apns_credential(&device_token(3))).await;
	assert_eq!(status, 200);

	// Alice's endpoint is rate limited forever, asking for one second
	// each time; bob's answers success. Alice's attempts spend the bound
	// while bob's delivery completes untouched.
	mock.scripted(|request, _| {
		if !request.url.contains(APNS_HOST) {
			return Scripted::Default;
		}

		if request.url.ends_with(&device_token(1)) {
			return Scripted::Reply(
				429,
				vec![("retry-after".to_owned(), "1".to_owned())],
				json!({ "reason": "TooManyRequests" }).to_string(),
			);
		}

		Scripted::Default
	});
	service.resume();
	let (status, _) = send(&service, "mixed", spec(), &[ALICE, BOB]).await;
	assert_eq!(status, 200);

	let outcomes = settled_rewinding(&service, "mixed").await;
	assert!(
		outcomes.contains(&("accepted".to_owned(), None)),
		"bob's delivery completed: {outcomes:?}"
	);
	assert!(
		outcomes.contains(&("rejected".to_owned(), Some("attempts_exhausted".to_owned()))),
		"alice's ended on its own: {outcomes:?}"
	);
}

// The fenced outcome writes: races between attempts, revocations, and
// replayed rounds. The held boundary makes each one deterministic.

#[tokio::test(flavor = "multi_thread")]
async fn a_revocation_during_an_in_flight_attempt_never_resurrects() {
	let mock = Mock::new();
	let service = service(&mock, &all_transports()).await;

	let (status, body) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200, "{body}");
	let endpoint: Value = serde_json::from_str(&body).unwrap();
	let (status, _) = send(&service, "racy", spec(), &[ALICE]).await;
	assert_eq!(status, 200);

	// The attempt goes out and is held in flight; the revocation lands
	// while it is; then the provider's answer arrives. The answer must
	// not bring the delivery back.
	mock.scripted(|request, _| {
		if request.url.contains(APNS_HOST) {
			return Scripted::Hold(
				429,
				vec![("retry-after".to_owned(), "1".to_owned())],
				String::new(),
			);
		}

		Scripted::Default
	});

	let racing = spawn_round(&service, "racy", 0);

	mock.wait_held(1).await;

	let (status, _) = post(
		&service,
		"revoke_push_endpoint",
		json!({ "endpoint": endpoint["id"] }),
		Some(&signed_in(ALICE)),
	)
	.await;
	assert_eq!(status, 200, "the revocation lands mid-flight");

	mock.release();
	racing.await.expect("the round finishes");

	let items = delivery_status(&service, "racy").await;
	assert_eq!(items[0]["state"], "skipped", "{items:?}");
	assert_eq!(items[0]["reason"], "endpoint_revoked", "{items:?}");
	assert_eq!(
		items[0]["attempts"], 0,
		"the late transient answer spends no attempt"
	);
	assert!(
		items[0]["next_attempt_at"].is_null(),
		"and schedules nothing"
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_late_accepted_answer_cannot_resurrect_a_revoked_delivery() {
	let mock = Mock::new();
	let service = service(&mock, &all_transports()).await;

	let (status, body) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200, "{body}");
	let endpoint: Value = serde_json::from_str(&body).unwrap();
	let (status, _) = send(&service, "late-accept", spec(), &[ALICE]).await;
	assert_eq!(status, 200);

	// The provider answers success, but only after the revocation
	// committed: the acceptance is recorded nowhere, because the row it
	// would write to has left the queue.
	mock.scripted(|request, _| {
		if request.url.contains(APNS_HOST) {
			return Scripted::Hold(200, Vec::new(), String::new());
		}

		Scripted::Default
	});

	let racing = spawn_round(&service, "late-accept", 0);

	mock.wait_held(1).await;

	let (status, _) = post(
		&service,
		"revoke_push_endpoint",
		json!({ "endpoint": endpoint["id"] }),
		Some(&signed_in(ALICE)),
	)
	.await;
	assert_eq!(status, 200);

	mock.release();
	racing.await.expect("the round finishes");

	let items = delivery_status(&service, "late-accept").await;
	assert_eq!(items[0]["state"], "skipped", "{items:?}");
	assert_eq!(items[0]["reason"], "endpoint_revoked", "{items:?}");
	assert_eq!(items[0]["attempts"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn replayed_rounds_send_at_most_twice_and_write_at_most_once() {
	let mock = Mock::new();
	let service = service(&mock, &all_transports()).await;

	let (status, _) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200);
	let (status, _) = send(&service, "replayed", spec(), &[ALICE]).await;
	assert_eq!(status, 200);

	// Two rounds run at once, as a lease expiry replays a round while
	// its first run is still in flight. Both attempts may reach the
	// provider — at-least-once allows it — but the outcome is written
	// once, by whoever wins the fence.
	mock.scripted(|request, _| {
		if request.url.contains(APNS_HOST) {
			return Scripted::Hold(200, Vec::new(), String::new());
		}

		Scripted::Default
	});

	let one = spawn_round(&service, "replayed", 0);
	let two = spawn_round(&service, "replayed", 0);

	mock.wait_held(2).await;
	mock.release();
	one.await.expect("the first round finishes");
	two.await.expect("the replayed round finishes");

	assert_eq!(
		mock.requests_to(APNS_HOST).len(),
		2,
		"at-least-once: both rounds reached the provider"
	);

	let items = delivery_status(&service, "replayed").await;
	assert_eq!(
		items[0]["state"], "accepted",
		"the delivery ends accepted: {items:?}"
	);
	assert_eq!(
		items[0]["attempts"], 1,
		"one attempt is counted, not stomped by the replay: {items:?}"
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_siblings_invalidations_do_not_stomp_an_answered_peer() {
	let mock = Mock::new();
	let service = service(&mock, &all_transports()).await;

	// One endpoint, two queued deliveries: the first send's answer will
	// invalidate the endpoint, and the second's succeeds first, while
	// the invalidating answer is still held in flight.
	let (status, _) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200);
	let (status, _) = send(
		&service,
		"sibling-gone",
		json!({ "category": "news", "title": "Dead", "body": "Gone." }),
		&[ALICE],
	)
	.await;
	assert_eq!(status, 200);
	let (status, _) = send(
		&service,
		"sibling-ok",
		json!({ "category": "news", "title": "Fine", "body": "Delivered." }),
		&[ALICE],
	)
	.await;
	assert_eq!(status, 200);

	mock.scripted(|request, _| {
		if !request.url.contains(APNS_HOST) {
			return Scripted::Default;
		}

		// The fine one answers success at once; the dead one is held
		// until its peer's acceptance has committed.
		if request.body.windows(6).any(|window| window == b"\"Fine\"") {
			return Scripted::Reply(200, Vec::new(), String::new());
		}

		Scripted::Hold(
			410,
			Vec::new(),
			json!({ "reason": "Unregistered" }).to_string(),
		)
	});

	// The fine delivery's own round answers it at once; the dead one's
	// round is held in flight.
	let ok_round = spawn_round(&service, "sibling-ok", 0);
	ok_round.await.expect("the fine round finishes");

	let racing = spawn_round(&service, "sibling-gone", 0);
	mock.wait_held(1).await;

	// The acceptance has landed; give the held write its turn: it must
	// not sweep its answered peer.
	let ok = delivery_status(&service, "sibling-ok").await;
	assert_eq!(
		ok[0]["state"], "accepted",
		"the answered peer committed first: {ok:?}"
	);

	mock.release();
	racing.await.expect("the dead round finishes");

	// The dead credential's delivery ended and invalidated the endpoint;
	// the sibling that was already answered stays accepted — the
	// invalidation's sweep of queued siblings can only take rows still
	// in the queue.
	let gone = delivery_status(&service, "sibling-gone").await;
	assert_eq!(gone[0]["state"], "rejected", "{gone:?}");
	assert_eq!(gone[0]["reason"], "unregistered", "{gone:?}");

	let ok = delivery_status(&service, "sibling-ok").await;
	assert_eq!(
		ok[0]["state"], "accepted",
		"an answered peer is never stomped: {ok:?}"
	);

	let endpoints = endpoints_of(&service, ALICE).await;
	assert!(endpoints[0]["invalidated_at"].is_string());
}

// Where credentials never appear.

#[tokio::test(flavor = "multi_thread")]
async fn credentials_never_appear_in_answers_or_debugs() {
	let mock = Mock::new();
	let mut service = service(&mock, &all_transports()).await;

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
		json!({ "page": { "limit": 100 } }),
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

	// The shared types' own Debug, redacted in the crate.
	let typed = format!(
		"{:?}",
		leptoskit_notifications::push::PushCredential::Apns {
			device_token: token.clone()
		}
	);

	for (name, answer) in [
		("the register answer", register),
		("the endpoint list", listed),
		("the delivery status", reported),
		("the cross-account refusal", conflict),
		("the stored row's debug", stored),
		("the credential type's debug", typed),
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
	let service = service(&mock, &all_transports()).await;

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
		assert!(
			!body.contains("attacker.test") && !body.contains("169.254"),
			"or any of it: {body}"
		);
	}

	// The shipped host enrolls, and the mock receives exactly it.
	let shipped = format!("https://{WEB_HOST}/push/abcdef123456");
	let (status, body) = enroll(&service, ALICE, web_credential(&shipped)).await;
	assert_eq!(status, 200, "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stale_extra_host_resolution_is_a_transient_retry() {
	// An operator-approved extra host whose resolution fails: the
	// delivery retries — DNS is a network dependency — rather than
	// ending or invalidating the endpoint.
	let mock = Mock::new();
	let (key_pem, _) = generated_pem();
	let (_, web) = web_config_for(&key_pem);
	let config = config::PushConfig {
		apns: None,
		fcm: None,
		web: Some(config::WebPushConfig {
			vapid_private_key: web.vapid_private_key,
			vapid_public_key: web.vapid_public_key,
			subject: web.subject,
			extra_hosts: vec!["push.unresolvable.invalid".to_owned()],
		}),
	};
	let mut service = service(&mock, &config).await;

	let endpoint = "https://push.unresolvable.invalid/send/1";
	let (status, body) = enroll(&service, ALICE, web_credential(endpoint)).await;
	assert_eq!(status, 200, "an approved host enrolls: {body}");

	service.resume();
	let (status, _) = send(&service, "no-dns", spec(), &[ALICE]).await;
	assert_eq!(status, 200);

	// The bounded resolution may take its full five seconds before the
	// retrying write lands.
	let items = wait_not_enqueued(&service, "no-dns").await;
	assert_eq!(
		items[0]["state"], "retrying",
		"a DNS failure retries, it doesn't end: {items:?}"
	);
	assert!(items[0]["reason"].is_null(), "{items:?}");

	let endpoints = endpoints_of(&service, ALICE).await;
	assert!(endpoints[0]["invalidated_at"].is_null());
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
fn an_ec_key_cannot_be_an_fcm_service_account_key() {
	// Google issues RSA keys and its OAuth takes RS256: an EC key in the
	// JSON — exactly the previous defect — refuses to start.
	let (ec_pem, _) = generated_pem();
	let config = config::PushConfig {
		apns: None,
		fcm: Some(fcm_config_for(&ec_pem)),
		web: None,
	};
	let Err(error) = PushRuntime::build(&config) else {
		panic!("an EC service-account key refuses to build");
	};
	assert!(error.contains("FCM"), "{error}");
	assert!(
		error.contains("RSA"),
		"the refusal names what it needs: {error}"
	);
}

#[test]
fn a_mismatched_vapid_pair_is_refused_at_the_build() {
	let (key_pem, _) = generated_pem();
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
}

#[test]
fn a_malformed_extra_host_is_refused_by_name_at_the_build() {
	let (key_pem, _) = generated_pem();
	let (_, web) = web_config_for(&key_pem);
	let config = config::PushConfig {
		apns: None,
		fcm: None,
		web: Some(config::WebPushConfig {
			vapid_private_key: web.vapid_private_key,
			vapid_public_key: web.vapid_public_key,
			subject: web.subject,
			extra_hosts: vec!["https://push.example.net".to_owned()],
		}),
	};
	let Err(error) = PushRuntime::build(&config) else {
		panic!("a malformed extra host refuses to build");
	};
	assert!(
		error.contains("push.example.net"),
		"the refusal names the entry: {error}"
	);
}

// Message options on the wire.

/// The APNs wire carries every option where Apple reads it: priority in
/// the header, sound, badge, and link in the payload, the Apple
/// overrides in `aps`, and an image under the mutable-content flag.
#[tokio::test(flavor = "multi_thread")]
async fn the_apns_wire_carries_the_send_options() {
	let mock = Mock::new();
	let mut service = service(&mock, &all_transports()).await;

	assert_eq!(
		enroll(&service, ALICE, apns_credential(&device_token(1)))
			.await
			.0,
		200
	);
	service.resume();

	let (status, body) = send(
		&service,
		"apns-options",
		json!({
			"category": "billing",
			"title": "Invoice paid",
			"body": "See the attachment.",
			"options": {
				"priority": "normal",
				"image": "https://cdn.example.test/paid.png",
				"sound": { "named": "chime.aiff" },
				"badge": { "set": 7 },
				"link": "/invoice/42",
				"apple": {
					"subtitle": "Invoice #42",
					"thread_id": "billing",
					"category": "BILLING",
					"interruption_level": "time_sensitive",
					"relevance_score": 0.9,
				},
			},
		}),
		&[ALICE],
	)
	.await;
	assert_eq!(status, 200, "{body}");
	settled(&service, "apns-options").await;

	let sent = mock.requests_to(APNS_HOST);
	assert_eq!(sent.len(), 1);
	assert_eq!(sent[0].header("apns-priority"), Some("5"), "normal is 5");
	assert_eq!(sent[0].header("apns-push-type"), Some("alert"));

	let payload = sent[0].json();
	assert_eq!(payload["aps"]["alert"]["title"], "Invoice paid");
	assert_eq!(payload["aps"]["alert"]["subtitle"], "Invoice #42");
	assert_eq!(payload["aps"]["sound"], "chime.aiff");
	assert_eq!(payload["aps"]["badge"], 7);
	assert_eq!(payload["aps"]["thread-id"], "billing");
	assert_eq!(payload["aps"]["category"], "BILLING");
	assert_eq!(payload["aps"]["interruption-level"], "time-sensitive");
	assert_eq!(payload["aps"]["relevance-score"], 0.9);
	assert_eq!(payload["aps"]["mutable-content"], 1);
	assert_eq!(payload["lkimg"], "https://cdn.example.test/paid.png");
	assert_eq!(payload["lklink"], "/invoice/42");
	assert!(payload["lkn"].as_str().is_some());

	// A silent sound leaves the key out — Apple platforms are silent
	// without one — and the default sound names it.
	for (sound, _key) in [("silent", false), ("default", true)] {
		let (status, body) = send(
			&service,
			&format!("apns-sound-{sound}"),
			json!({
				"category": "billing",
				"title": "T",
				"options": { "sound": sound },
			}),
			&[ALICE],
		)
		.await;
		assert_eq!(status, 200, "{body}");
		settled(&service, &format!("apns-sound-{sound}")).await;
	}

	let sounds = mock.requests_to(APNS_HOST);
	assert!(
		sounds[1].json()["aps"].get("sound").is_none(),
		"silent sends no sound key"
	);
	assert_eq!(sounds[2].json()["aps"]["sound"], "default");
}

/// A data send's APNs wire is a background push: the background type,
/// priority 5 as Apple requires with it, `content-available`, and no
/// alert — with the data and the id still riding.
#[tokio::test(flavor = "multi_thread")]
async fn the_apns_data_wire_is_a_background_push() {
	let mock = Mock::new();
	let mut service = service(&mock, &all_transports()).await;

	assert_eq!(
		enroll(&service, ALICE, apns_credential(&device_token(1)))
			.await
			.0,
		200
	);
	service.resume();

	let (status, body) = send(
		&service,
		"apns-data",
		json!({
			"category": "sync",
			"title": "Sync",
			"body": "A silent nudge.",
			"data": json!({ "since": "42" }).to_string(),
			"options": {
				"kind": "data",
				"priority": "high",
				"badge": { "set": 3 },
			},
		}),
		&[ALICE],
	)
	.await;
	assert_eq!(status, 200, "{body}");
	settled(&service, "apns-data").await;

	let sent = mock.requests_to(APNS_HOST);
	assert_eq!(sent.len(), 1);
	assert_eq!(sent[0].header("apns-push-type"), Some("background"));
	assert_eq!(
		sent[0].header("apns-priority"),
		Some("5"),
		"a background push is priority 5, whatever the send asked"
	);

	// Apple's background push takes `content-available` alone in `aps`:
	// a badge there would be a user-visible change, so it rides as data
	// for the app to apply.
	let payload = sent[0].json();
	assert_eq!(
		payload["aps"],
		json!({ "content-available": 1 }),
		"nothing but the background flag: {payload}"
	);
	assert_eq!(payload["lkbadge"], 3, "the badge rides as data");
	assert_eq!(payload["lk"]["since"], "42");
	assert!(payload["lkn"].as_str().is_some());
}

/// The FCM wire carries the options where the HTTP v1 API reads them:
/// the Android notification and priority, the image in both the
/// notification and the iOS payload, the link in the data, and the
/// Apple overrides in the APNs block the project also serves.
#[tokio::test(flavor = "multi_thread")]
async fn the_fcm_wire_carries_the_send_options() {
	let mock = Mock::new();
	let (rsa_pem, _) = test_rsa();
	let config = config::PushConfig {
		apns: None,
		fcm: Some(fcm_config_for(&rsa_pem)),
		web: None,
	};
	let mut service = service(&mock, &config).await;

	assert_eq!(
		enroll(&service, ALICE, fcm_credential("fcm-token-1"))
			.await
			.0,
		200
	);
	mock.scripted(passing_exchange());
	service.resume();

	let (status, body) = send(
		&service,
		"fcm-options",
		json!({
			"category": "billing",
			"title": "Invoice paid",
			"body": "See the attachment.",
			"options": {
				"priority": "high",
				"image": "https://cdn.example.test/paid.png",
				"sound": { "named": "chime" },
				"badge": { "set": 7 },
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
					"interruption_level": "passive",
					"relevance_score": 0.5,
				},
			},
		}),
		&[ALICE],
	)
	.await;
	assert_eq!(status, 200, "{body}");
	settled(&service, "fcm-options").await;

	let sent = mock.requests_to(FCM_HOST);
	assert_eq!(sent.len(), 1);
	let message = sent[0].json()["message"].clone();

	let android = &message["android"];
	assert_eq!(android["priority"], "HIGH");
	// The channel a send names is the one the local-notifications plugin
	// made under that id, and the plugin namespaces every channel it
	// creates: the id Android knows is the prefixed one. The raw id names
	// no channel, and Firebase would post through its fallback instead.
	assert_eq!(
		android["notification"]["channel_id"],
		"leptoskit.notifications.inbox"
	);
	assert_eq!(android["notification"]["color"], "#10b981");
	assert_eq!(android["notification"]["icon"], "ic_income");
	assert_eq!(android["notification"]["tag"], "billing");
	assert_eq!(android["notification"]["sticky"], true);
	assert_eq!(android["notification"]["sound"], "chime");
	assert_eq!(android["notification"]["notification_count"], 7);

	assert_eq!(
		message["notification"]["image"],
		"https://cdn.example.test/paid.png"
	);
	assert_eq!(message["data"]["lklink"], "/invoice/42");

	let aps = &message["apns"]["payload"]["aps"];
	assert_eq!(aps["alert"]["subtitle"], "Invoice #42");
	assert_eq!(aps["thread-id"], "billing");
	assert_eq!(aps["interruption-level"], "passive");
	assert_eq!(aps["relevance-score"], 0.5);
	assert_eq!(aps["mutable-content"], 1);
	assert_eq!(
		message["apns"]["payload"]["fcm_options"]["image"],
		"https://cdn.example.test/paid.png"
	);
	assert_eq!(message["apns"]["headers"]["apns-priority"], "10");

	// A default send adds none of the option headers it never had: the
	// bytes an option-less send produces are the bytes it always did.
	let (status, body) = send(&service, "fcm-plain-after-options", spec(), &[ALICE]).await;
	assert_eq!(status, 200, "{body}");
	settled(&service, "fcm-plain-after-options").await;
	let plain = mock.requests_to(FCM_HOST)[1].json()["message"].clone();
	assert!(plain["android"].get("priority").is_none(), "{plain}");
	assert!(plain["android"].get("notification").is_none(), "{plain}");
	assert!(
		plain["apns"]["headers"].get("apns-priority").is_none(),
		"{plain}"
	);
	assert!(
		plain["notification"].get("image").is_none() && plain["data"].get("lklink").is_none(),
		"{plain}"
	);
}

/// A data send's FCM wire has no notification block at all — Android
/// hands the data to the app — and its iOS half is the same background
/// push a direct APNs send would be.
#[tokio::test(flavor = "multi_thread")]
async fn the_fcm_data_wire_has_no_notification_block() {
	let mock = Mock::new();
	let (rsa_pem, _) = test_rsa();
	let config = config::PushConfig {
		apns: None,
		fcm: Some(fcm_config_for(&rsa_pem)),
		web: None,
	};
	let mut service = service(&mock, &config).await;

	assert_eq!(
		enroll(&service, ALICE, fcm_credential("fcm-token-1"))
			.await
			.0,
		200
	);
	mock.scripted(passing_exchange());
	service.resume();

	let (status, body) = send(
		&service,
		"fcm-data",
		json!({
			"category": "sync",
			"title": "Sync",
			"body": "A silent nudge.",
			"data": json!({ "since": "42" }).to_string(),
			"options": { "kind": "data", "priority": "normal", "badge": { "set": 3 } },
		}),
		&[ALICE],
	)
	.await;
	assert_eq!(status, 200, "{body}");
	settled(&service, "fcm-data").await;

	let message = mock.requests_to(FCM_HOST)[0].json()["message"].clone();
	assert!(
		message.get("notification").is_none(),
		"a data message shows nothing: {message}"
	);
	// An Android notification block would make FCM show the message in
	// the tray and never hand it to the app: the badge rides as data.
	assert!(
		message["android"].get("notification").is_none(),
		"no Android notification block either: {message}"
	);
	assert_eq!(message["data"]["lkbadge"], "3", "the badge rides as data");
	assert_eq!(message["data"]["since"], "42");
	assert_eq!(message["data"]["lkn"], message["android"]["collapse_key"]);
	assert_eq!(message["android"]["priority"], "NORMAL");
	assert_eq!(message["apns"]["headers"]["apns-push-type"], "background");
	assert_eq!(message["apns"]["headers"]["apns-priority"], "5");
	assert_eq!(
		message["apns"]["payload"]["aps"],
		json!({ "content-available": 1 }),
		"{message}"
	);
}

/// The Web Push wire carries the options the receiver reads: urgency in
/// the header, and the image, badge, link, silent flag, and web
/// presentation inside the independently opened envelope.
#[tokio::test(flavor = "multi_thread")]
async fn the_web_push_wire_carries_the_send_options() {
	let mock = Mock::new();
	let (key_pem, _) = generated_pem();
	let (_, web) = web_config_for(&key_pem);
	let config = config::PushConfig {
		apns: None,
		fcm: None,
		web: Some(web),
	};
	let mut service = service(&mock, &config).await;

	let endpoint = format!("https://{WEB_HOST}/push/abcdef123456");
	let (receiver, credential, auth_secret, _) = web_subscription(&endpoint);
	let (status, body) = enroll(&service, ALICE, credential).await;
	assert_eq!(status, 200, "{body}");
	service.resume();

	let (status, body) = send(
		&service,
		"web-options",
		json!({
			"category": "billing",
			"title": "Invoice paid",
			"body": "See the attachment.",
			"options": {
				"priority": "high",
				"image": "https://cdn.example.test/paid.png",
				"sound": "silent",
				"badge": { "set": 7 },
				"link": "/invoice/42",
				"web": {
					"icon": "https://cdn.example.test/icon.png",
					"badge_icon": "/badge.png",
					"actions": [{ "action": "open", "title": "Open" }],
					"require_interaction": true,
					"dir": "rtl",
					"lang": "de",
				},
			},
		}),
		&[ALICE],
	)
	.await;
	assert_eq!(status, 200, "{body}");
	settled(&service, "web-options").await;

	let sent = mock.requests_to(WEB_HOST);
	assert_eq!(sent.len(), 1);
	assert_eq!(sent[0].header("urgency"), Some("high"));

	let receiver_point = {
		let stored = rows(&service, "SELECT credential FROM push_endpoints").await;
		let subscription: Value = serde_json::from_str(&text(&stored[0][0])).unwrap();
		URL_SAFE_NO_PAD
			.decode(subscription["keys"]["p256dh"].as_str().unwrap())
			.unwrap()
	};
	let opened = open_aes128gcm(&sent[0].body, receiver, &receiver_point, &auth_secret);
	let payload: Value = serde_json::from_slice(&opened).unwrap();
	assert_eq!(payload["image"], "https://cdn.example.test/paid.png");
	assert_eq!(payload["lkbadge"], 7);
	assert_eq!(payload["lklink"], "/invoice/42");
	assert_eq!(payload["silent"], true);
	assert_eq!(payload["lkw"]["icon"], "https://cdn.example.test/icon.png");
	assert_eq!(payload["lkw"]["badge"], "/badge.png");
	assert_eq!(payload["lkw"]["actions"][0]["action"], "open");
	assert_eq!(payload["lkw"]["actions"][0]["title"], "Open");
	assert_eq!(payload["lkw"]["require_interaction"], true);
	assert_eq!(payload["lkw"]["dir"], "rtl");
	assert_eq!(payload["lkw"]["lang"], "de");
	assert!(payload["lkdata"].is_null(), "a visible send is not flagged");

	// A default send keeps today's urgency and today's envelope keys.
	let (status, body) = send(&service, "web-plain", spec(), &[ALICE]).await;
	assert_eq!(status, 200, "{body}");
	settled(&service, "web-plain").await;
	let plain = mock.requests_to(WEB_HOST)[1].clone();
	assert_eq!(plain.header("urgency"), Some("normal"));
}

/// A data message is never pushed to a browser: browsers require every
/// push to show a notification — Chrome shows its own generic one, Safari
/// revokes the subscription after silent pushes, Edge doesn't take them
/// — and the worker has no page to hand data to. The delivery is skipped
/// with a reason that says the platform doesn't support it; the same send
/// still reaches the person's native devices, and a test send says the
/// same per endpoint.
#[tokio::test(flavor = "multi_thread")]
async fn a_data_message_is_never_pushed_to_a_browser() {
	let mock = Mock::new();
	let mut service = service(&mock, &all_transports()).await;

	let endpoint = format!("https://{WEB_HOST}/push/abcdef123456");
	let (status, body) = enroll(&service, ALICE, web_credential(&endpoint)).await;
	assert_eq!(status, 200, "{body}");
	let (status, body) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200, "{body}");
	service.resume();

	let data = json!({
		"category": "sync",
		"title": "Sync",
		"body": "A silent nudge.",
		"data": json!({ "since": "42" }).to_string(),
		"options": { "kind": "data", "badge": { "set": 2 } },
	});
	let (status, body) = send(&service, "web-data", data.clone(), &[ALICE]).await;
	assert_eq!(status, 200, "{body}");

	let mut outcomes = settled(&service, "web-data").await;
	outcomes.sort();
	assert_eq!(
		outcomes,
		vec![
			("accepted".to_owned(), None),
			(
				"skipped".to_owned(),
				Some("platform_unsupported".to_owned())
			),
		],
		"the browser's delivery is skipped, the device's goes: {outcomes:?}"
	);
	assert!(
		mock.requests_to(WEB_HOST).is_empty(),
		"nothing reached the push service"
	);
	assert_eq!(mock.requests_to(APNS_HOST).len(), 1);
	assert!(
		endpoints_of(&service, ALICE)
			.await
			.iter()
			.all(|endpoint| endpoint["invalidated_at"].is_null()),
		"the subscription stays enrolled"
	);

	let (status, body) = post(
		&service,
		"send_test",
		json!({ "spec": data, "person": ALICE }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let tested: Value = serde_json::from_str(&body).unwrap();
	let web = tested
		.as_array()
		.unwrap()
		.iter()
		.find(|outcome| outcome["platform"] == "web_push")
		.cloned()
		.expect("the browser's outcome");
	assert_eq!(web["state"], "skipped", "{tested}");
	assert_eq!(web["reason"], "platform_unsupported", "{tested}");
	assert!(mock.requests_to(WEB_HOST).is_empty());
}

/// An unread badge resolves per recipient at push time: two people, two
/// different unread counts, one send — each payload badges its own.
#[tokio::test(flavor = "multi_thread")]
async fn an_unread_badge_resolves_per_recipient_at_push_time() {
	let mock = Mock::new();
	let mut service = service(&mock, &all_transports()).await;

	assert_eq!(
		enroll(&service, ALICE, apns_credential(&device_token(1)))
			.await
			.0,
		200
	);
	assert_eq!(
		enroll(&service, BOB, apns_credential(&device_token(2)))
			.await
			.0,
		200
	);
	service.resume();

	// Alice holds one unread, bob three.
	for key in ["alice-prior"] {
		let (status, body) = send(&service, key, spec(), &[ALICE]).await;
		assert_eq!(status, 200, "{body}");
	}

	for key in ["bob-prior-1", "bob-prior-2", "bob-prior-3"] {
		let (status, body) = send(&service, key, spec(), &[BOB]).await;
		assert_eq!(status, 200, "{body}");
	}

	settled(&service, "bob-prior-3").await;

	let (status, body) = send(
		&service,
		"badge-unread",
		json!({
			"category": "news",
			"title": "Two badges",
			"body": "One each.",
			"options": { "badge": "unread" },
		}),
		&[ALICE, BOB],
	)
	.await;
	assert_eq!(status, 200, "{body}");
	settled(&service, "badge-unread").await;

	// The badge-unread send is itself unread, so each count includes it:
	// alice's 1 prior plus this one, bob's 3 plus this one.
	let mut badges = mock
		.requests_to(APNS_HOST)
		.iter()
		.filter(|seen| seen.header("apns-collapse-id").is_some())
		.map(|seen| seen.json()["aps"]["badge"].as_i64())
		.collect::<Vec<_>>();
	badges.sort_unstable();

	assert_eq!(
		badges,
		vec![None, None, None, None, Some(2), Some(4)],
		"only the badge-unread send carries badges, one per recipient: {badges:?}"
	);
}

/// The payload limits degrade the optional extras in the documented
/// order — platform overrides and image first, then sound, badge, and
/// link, then the send's `data` — before the id-only envelope.
#[tokio::test(flavor = "multi_thread")]
async fn payload_limits_degrade_options_in_the_documented_order() {
	let mock = Mock::new();
	let mut service = service(&mock, &all_transports()).await;

	assert_eq!(
		enroll(&service, ALICE, apns_credential(&device_token(1)))
			.await
			.0,
		200
	);
	assert_eq!(
		enroll(&service, ALICE, fcm_credential("fcm-degrade"))
			.await
			.0,
		200
	);
	mock.scripted(passing_exchange());
	service.resume();

	// Sized so the full payload is past Apple's 4 KiB, the
	// platform-and-image trim is past it too, and the sound-badge-link
	// trim fits: the wire carries the data without any of the extras.
	let link = format!("/l/{}", "x".repeat(900));
	let (status, body) = send(
		&service,
		"apns-degrade",
		json!({
			"category": "billing",
			"title": "T",
			"body": "x".repeat(600),
			"data": json!({ "k": "d".repeat(3_300) }).to_string(),
			"options": {
				"image": "https://cdn.example.test/i.png",
				"sound": "default",
				"badge": { "set": 7 },
				"link": link,
				"apple": { "subtitle": "s".repeat(100) },
			},
		}),
		&[ALICE],
	)
	.await;
	assert_eq!(status, 200, "{body}");

	let outcomes = settled(&service, "apns-degrade").await;
	assert!(
		outcomes.iter().all(|(state, _)| state == "accepted"),
		"{outcomes:?}"
	);

	let apns = mock
		.requests_to(APNS_HOST)
		.iter()
		.map(Seen::json)
		.max_by_key(|payload| payload["lkn"].as_str().map_or(0, str::len))
		.expect("the degraded push went out");
	assert!(apns["lk"].is_object(), "the data survived: {apns}");
	assert!(apns["aps"].get("sound").is_none(), "sound went: {apns}");
	assert!(apns["aps"].get("badge").is_none(), "badge went: {apns}");
	assert!(apns.get("lklink").is_none(), "link went: {apns}");
	assert!(apns.get("lkimg").is_none(), "image went: {apns}");
	assert!(
		apns["aps"]["alert"].get("subtitle").is_none(),
		"platform overrides went: {apns}"
	);

	// Past every trim, the id-only envelope: no data, no options, the
	// alert and the id alone — with the collapse id still its own.
	let (status, body) = send(
		&service,
		"apns-minimal",
		json!({
			"category": "billing",
			"title": "T",
			"body": "b".repeat(1_900),
			"data": json!({ "k": "d".repeat(3_300) }).to_string(),
			"options": { "sound": "default", "badge": { "set": 7 } },
		}),
		&[ALICE],
	)
	.await;
	assert_eq!(status, 200, "{body}");
	settled(&service, "apns-minimal").await;

	let minimal = mock
		.requests_to(APNS_HOST)
		.iter()
		.map(Seen::json)
		.find(|payload| payload["aps"]["alert"]["title"] == "T" && payload.get("lk").is_none())
		.expect("the id-only envelope went out");
	assert!(minimal["aps"].get("sound").is_none(), "{minimal}");
	assert!(minimal["aps"].get("badge").is_none(), "{minimal}");

	// The same order on FCM: the Android notification, the image, and
	// the Apple extras go before the link, and the data outlasts them
	// all short of the id-only envelope. Sized on what FCM counts — the
	// keys and values the device receives — so the full payload is about
	// 4,170 bytes and the platform-and-image trim brings it to about
	// 4,030: past 4 KiB until the platform extras leave, and not after.
	let (status, body) = send(
		&service,
		"fcm-degrade",
		json!({
			"category": "billing",
			"title": "T",
			"body": "y".repeat(1_050),
			"data": json!({ "k": "d".repeat(2_840) }).to_string(),
			"options": {
				"image": "https://cdn.example.test/i.png",
				"badge": { "set": 7 },
				"link": "/l",
				"android": { "channel_id": "inbox", "tag": "billing" },
			},
		}),
		&[ALICE],
	)
	.await;
	assert_eq!(status, 200, "{body}");
	settled(&service, "fcm-degrade").await;

	let degraded = mock
		.requests_to(FCM_HOST)
		.iter()
		.map(|seen| seen.json()["message"].clone())
		.find(|message| {
			message["notification"]["body"]
				.as_str()
				.is_some_and(|body| body.starts_with('y'))
		})
		.expect("the degraded FCM message went out");
	assert!(
		degraded["data"].get("k").is_some(),
		"the data outlasts the platform block: {degraded}"
	);

	// Stage one dropped the platform overrides and the image, and kept
	// everything the later stages own: the badge and the link, and the
	// data and the collapse key, which never go short of the id-only
	// envelope.
	let android_notification = &degraded["android"]["notification"];
	assert!(
		android_notification.get("channel_id").is_none()
			&& android_notification.get("tag").is_none(),
		"the platform block went: {degraded}"
	);
	assert_eq!(
		android_notification["notification_count"], 7,
		"the badge outlasts the platform block: {degraded}"
	);
	assert!(
		degraded["notification"].get("image").is_none(),
		"the image went: {degraded}"
	);
	assert!(
		degraded["apns"]["payload"].get("fcm_options").is_none(),
		"the iOS image went with it: {degraded}"
	);
	assert!(
		degraded["data"].get("lklink").is_some(),
		"the link stayed: {degraded}"
	);
	assert!(
		degraded["android"].get("collapse_key").is_some(),
		"the collapse key never goes: {degraded}"
	);
}

// Provider rules, as each provider documents them.

/// Real browsers' subscriptions enroll: Chrome's FCM endpoint, Firefox's
/// autopush, Safari's Apple Web Push service on a `push.apple.com`
/// subdomain, and Edge's Windows Push Notification Services channel on a
/// `notify.windows.com` subdomain. Lookalikes of the new domains, a wrong
/// port, and credentials in the URL are still refused, without an echo.
#[tokio::test(flavor = "multi_thread")]
async fn every_major_browsers_subscription_enrolls() {
	let mock = Mock::new();
	let service = service(&mock, &all_transports()).await;

	for (browser, endpoint) in [
		(
			"Chrome",
			"https://fcm.googleapis.com/fcm/send/dQw4w9WgXcQ:APA91bFakeChromeEndpoint",
		),
		(
			"Firefox",
			"https://updates.push.services.mozilla.com/wpush/v2/gAAAAABfakeFirefoxEndpoint",
		),
		(
			"Safari",
			"https://web.push.apple.com/QFakeSafariEndpointToken",
		),
		(
			"Edge",
			"https://wns2-par02p.notify.windows.com/w/?token=BQYAAADfakeEdgeEndpoint",
		),
	] {
		let (status, body) = enroll(&service, ALICE, web_credential(endpoint)).await;
		assert_eq!(status, 200, "{browser} enrolls: {body}");
	}

	for (name, endpoint) in [
		(
			"Apple's domain under another",
			"https://web.push.apple.com.attacker.test/push/1",
		),
		(
			"a host that only ends like Apple's",
			"https://webpush.apple.com/push/1",
		),
		("Apple's bare domain", "https://push.apple.com/push/1"),
		(
			"Windows's domain under another",
			"https://wns2-par02p.notify.windows.com.attacker.test/w/?token=1",
		),
		(
			"a host that only ends like Windows's",
			"https://attackernotify.windows.com/w/?token=1",
		),
		("a trailing dot", "https://web.push.apple.com./push/1"),
		("a wrong port", "https://web.push.apple.com:8443/push/1"),
		(
			"credentials in the URL",
			"https://user:pass@wns2-par02p.notify.windows.com/w/?token=1",
		),
		(
			"userinfo hiding the real host",
			"https://web.push.apple.com@attacker.test/push/1",
		),
	] {
		let (status, body) = enroll(&service, BOB, web_credential(endpoint)).await;
		assert_eq!(status, 400, "{name} is refused: {body}");
		assert!(
			!body.contains(endpoint) && !body.contains("attacker"),
			"the refusal doesn't echo the URL: {body}"
		);
	}
}

/// FCM's 4,096-byte limit is on the payload a device receives — the keys
/// and values of the notification and the data — not on the HTTP request
/// around it, and the visible text rides once: the largest title and
/// body a send allows reach an Android device whole, options and data
/// included.
#[tokio::test(flavor = "multi_thread")]
async fn the_largest_send_text_reaches_an_fcm_device() {
	let mock = Mock::new();
	let (rsa_pem, _) = test_rsa();
	let config = config::PushConfig {
		apns: None,
		fcm: Some(fcm_config_for(&rsa_pem)),
		web: None,
	};
	let mut service = service(&mock, &config).await;

	let (status, _) = enroll(&service, ALICE, fcm_credential("fcm-token-large")).await;
	assert_eq!(status, 200);
	mock.scripted(passing_exchange());
	service.resume();

	let title = "t".repeat(leptoskit_notifications::MAX_TITLE_LEN);
	let body = "b".repeat(leptoskit_notifications::MAX_BODY_LEN);
	let (status, answer) = send(
		&service,
		"fcm-large",
		json!({
			"category": "news",
			"title": title,
			"body": body,
			"data": json!({ "order": "42" }).to_string(),
			"options": { "image": "https://cdn.example.test/i.png" },
		}),
		&[ALICE],
	)
	.await;
	assert_eq!(status, 200, "{answer}");

	let outcomes = settled(&service, "fcm-large").await;
	assert_eq!(outcomes, vec![("accepted".to_owned(), None)]);

	let sent = mock.requests_to(FCM_HOST);
	assert_eq!(sent.len(), 1);
	let message = sent[0].json()["message"].clone();
	assert_eq!(message["notification"]["title"], title.as_str());
	assert_eq!(message["notification"]["body"], body.as_str());
	assert_eq!(
		message["notification"]["image"],
		"https://cdn.example.test/i.png"
	);
	assert_eq!(message["data"]["order"], "42", "nothing was trimmed");
	assert!(
		message["apns"]["payload"]["aps"].get("alert").is_none(),
		"FCM builds the iOS alert from the notification; the text rides once"
	);
	assert_eq!(
		String::from_utf8_lossy(&sent[0].body)
			.matches(body.as_str())
			.count(),
		1,
		"the body is on the wire once"
	);
}

/// APNs answers that mean the operator's environment or topic is wrong —
/// `BadDeviceToken` is what a sandbox token gets from production, and
/// `DeviceTokenNotForTopic` what any token gets under the wrong bundle id
/// — end the delivery as the operator's configuration and leave the
/// device enrolled: one bad deploy must not kill every iPhone. Only
/// `410 Unregistered` invalidates.
#[tokio::test(flavor = "multi_thread")]
async fn apns_configuration_answers_never_invalidate_a_device() {
	let mock = Mock::new();
	let mut service = service(&mock, &all_transports()).await;

	for n in 1..=3 {
		let (status, body) = enroll(&service, ALICE, apns_credential(&device_token(n))).await;
		assert_eq!(status, 200, "{body}");
	}

	mock.scripted(|request, _| {
		if !request.url.contains(APNS_HOST) {
			return Scripted::Default;
		}

		let (status, reason) = if request.url.ends_with(&device_token(1)) {
			(400, "BadDeviceToken")
		} else if request.url.ends_with(&device_token(2)) {
			(400, "DeviceTokenNotForTopic")
		} else {
			(410, "Unregistered")
		};

		Scripted::Reply(status, Vec::new(), json!({ "reason": reason }).to_string())
	});
	service.resume();
	let (status, _) = send(&service, "misconfigured", spec(), &[ALICE]).await;
	assert_eq!(status, 200);

	let mut outcomes = settled(&service, "misconfigured").await;
	outcomes.sort();
	assert_eq!(
		outcomes,
		vec![
			(
				"rejected".to_owned(),
				Some("operator_configuration".to_owned())
			),
			(
				"rejected".to_owned(),
				Some("operator_configuration".to_owned())
			),
			("rejected".to_owned(), Some("unregistered".to_owned())),
		],
		"{outcomes:?}"
	);

	// Oldest first: the two the operator's setup refused stay live.
	let endpoints = endpoints_of(&service, ALICE).await;
	assert!(endpoints[0]["invalidated_at"].is_null(), "{endpoints:?}");
	assert!(endpoints[1]["invalidated_at"].is_null(), "{endpoints:?}");
	assert!(endpoints[2]["invalidated_at"].is_string(), "{endpoints:?}");
}

/// A provider token APNs refuses — stale, or not valid — is dropped from
/// the cache, and the retry signs a fresh one rather than failing every
/// delivery with the refused token until it ages out.
#[tokio::test(flavor = "multi_thread")]
async fn a_refused_apns_provider_token_is_replaced_and_the_push_retried() {
	let mock = Mock::new();
	let mut service = service(&mock, &all_transports()).await;

	let (status, _) = enroll(&service, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 200);
	mock.scripted(|request, index| {
		if request.url.contains(APNS_HOST) && index == 0 {
			return Scripted::Reply(
				403,
				Vec::new(),
				json!({ "reason": "ExpiredProviderToken" }).to_string(),
			);
		}

		Scripted::Default
	});
	service.resume();
	let (status, _) = send(&service, "stale-token", spec(), &[ALICE]).await;
	assert_eq!(status, 200);

	// The retry waits out the engine's one-minute backoff like any other,
	// so the test moves the queue's clock rather than waiting for it.
	let outcomes = settled_rewinding(&service, "stale-token").await;
	assert_eq!(
		outcomes,
		vec![("accepted".to_owned(), None)],
		"{outcomes:?}"
	);

	let sent = mock.requests_to(APNS_HOST);
	assert_eq!(sent.len(), 2, "one refused push, one retry");
	assert_ne!(
		sent[0].header("authorization"),
		sent[1].header("authorization"),
		"the retry carries a freshly signed provider token"
	);
	assert!(endpoints_of(&service, ALICE).await[0]["invalidated_at"].is_null());
}

/// FCM answers that name the sender's setup — `SENDER_ID_MISMATCH`, the
/// token belongs to another Firebase project, and
/// `THIRD_PARTY_AUTH_ERROR`, the project's APNs or Web Push credential is
/// missing — end at once as the operator's configuration: no retry fixes
/// them, and neither means the device is gone.
#[tokio::test(flavor = "multi_thread")]
async fn fcm_sender_configuration_answers_end_without_retrying() {
	let mock = Mock::new();
	let (rsa_pem, _) = test_rsa();
	let config = config::PushConfig {
		apns: None,
		fcm: Some(fcm_config_for(&rsa_pem)),
		web: None,
	};
	let mut service = service(&mock, &config).await;

	for token in ["fcm-other-project", "fcm-third-party"] {
		let (status, body) = enroll(&service, ALICE, fcm_credential(token)).await;
		assert_eq!(status, 200, "{body}");
	}

	mock.scripted(|request, _| {
		if request.url.contains(FCM_TOKEN_HOST) {
			return Scripted::Reply(
				200,
				Vec::new(),
				json!({ "access_token": "ya29.test-token", "expires_in": 3600 }).to_string(),
			);
		}

		if !request.url.contains(FCM_HOST) {
			return Scripted::Default;
		}

		let message: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
		let (status, error, code) = if message["message"]["token"] == "fcm-other-project" {
			(403, "PERMISSION_DENIED", "SENDER_ID_MISMATCH")
		} else {
			(401, "UNAUTHENTICATED", "THIRD_PARTY_AUTH_ERROR")
		};

		Scripted::Reply(
			status,
			Vec::new(),
			json!({
				"error": {
					"status": error,
					"details": [{
						"@type": "type.googleapis.com/google.firebase.fcm.v1.FcmError",
						"errorCode": code,
					}],
				},
			})
			.to_string(),
		)
	});
	service.resume();
	let (status, _) = send(&service, "fcm-sender", spec(), &[ALICE]).await;
	assert_eq!(status, 200);

	let items = wait_not_enqueued(&service, "fcm-sender").await;
	assert_eq!(items.len(), 2);
	for item in &items {
		assert_eq!(item["state"], "rejected", "{items:?}");
		assert_eq!(item["reason"], "operator_configuration", "{items:?}");
	}
	assert_eq!(
		mock.requests_to(FCM_HOST).len(),
		2,
		"one push each, no retry"
	);
	assert!(
		endpoints_of(&service, ALICE)
			.await
			.iter()
			.all(|endpoint| endpoint["invalidated_at"].is_null())
	);
}

/// A push service refusing the VAPID signature — RFC 8292's 401 or 403,
/// or Apple's `VapidPkHashMismatch` after the operator rotated the key
/// pair — is the operator's configuration: the delivery ends saying so,
/// and the subscription stays enrolled.
#[tokio::test(flavor = "multi_thread")]
async fn a_refused_vapid_signature_is_the_operators_configuration() {
	let mock = Mock::new();
	let config = config::PushConfig {
		apns: None,
		fcm: None,
		web: Some(web_config_for(&generated_pem().0).1),
	};
	let mut service = service(&mock, &config).await;

	for path in ["unauthorized", "forbidden"] {
		let endpoint = format!("https://{WEB_HOST}/push/{path}");
		let (status, body) = enroll(&service, ALICE, web_credential(&endpoint)).await;
		assert_eq!(status, 200, "{body}");
	}

	mock.scripted(|request, _| {
		if request.url.ends_with("/unauthorized") {
			return Scripted::Reply(401, Vec::new(), String::new());
		}

		if request.url.ends_with("/forbidden") {
			return Scripted::Reply(
				403,
				Vec::new(),
				json!({ "reason": "VapidPkHashMismatch" }).to_string(),
			);
		}

		Scripted::Default
	});
	service.resume();
	let (status, _) = send(&service, "vapid-refused", spec(), &[ALICE]).await;
	assert_eq!(status, 200);

	let outcomes = settled(&service, "vapid-refused").await;
	assert_eq!(
		outcomes,
		vec![
			(
				"rejected".to_owned(),
				Some("operator_configuration".to_owned())
			);
			2
		],
		"{outcomes:?}"
	);
	assert!(
		endpoints_of(&service, ALICE)
			.await
			.iter()
			.all(|endpoint| endpoint["invalidated_at"].is_null())
	);
}

/// A subscription whose `p256dh` value has the right length but is no
/// point on P-256 can never be encrypted to. Its delivery ends at once,
/// with no request, as an unusable subscription — not as `oversized`,
/// which would send the operator looking at the message.
#[tokio::test(flavor = "multi_thread")]
async fn a_subscription_key_off_the_curve_ends_as_unusable() {
	let mock = Mock::new();
	let config = config::PushConfig {
		apns: None,
		fcm: None,
		web: Some(web_config_for(&generated_pem().0).1),
	};
	let mut service = service(&mock, &config).await;

	let mut off_curve = vec![0x04];
	off_curve.extend_from_slice(&[0x01; 64]);
	let credential = json!({
		"web_push": {
			"endpoint": format!("https://{WEB_HOST}/push/off-curve"),
			"keys": {
				"p256dh": URL_SAFE_NO_PAD.encode(&off_curve),
				"auth": URL_SAFE_NO_PAD.encode([7_u8; 16]),
			},
		}
	});
	let (status, body) = enroll(&service, ALICE, credential).await;
	assert_eq!(status, 200, "the shape check passes it: {body}");

	service.resume();
	let (status, _) = send(&service, "off-curve", spec(), &[ALICE]).await;
	assert_eq!(status, 200);

	let outcomes = settled(&service, "off-curve").await;
	assert_eq!(
		outcomes,
		vec![(
			"skipped".to_owned(),
			Some("subscription_unusable".to_owned())
		)],
		"{outcomes:?}"
	);
	assert!(mock.requests_to(WEB_HOST).is_empty(), "nothing was sent");
}

/// The operator's extra Web Push hosts mean nothing without a VAPID pair
/// to push with: setting them alone is half a transport, refused with
/// every missing variable named in full, not a server that runs with Web
/// Push off.
#[test]
fn extra_hosts_without_a_vapid_pair_are_half_configured() {
	let error = config::PushConfig::from_vars([(
		"LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_EXTRA_HOSTS",
		"push.example.net",
	)])
	.expect_err("extra hosts alone are half a transport");

	for missing in [
		"LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_VAPID_PRIVATE_KEY",
		"LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_VAPID_PUBLIC_KEY",
		"LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_VAPID_SUBJECT",
	] {
		assert!(error.contains(missing), "{missing} is named: {error}");
	}
}

#[cfg(feature = "postgres")]
#[path = "support/postgres.rs"]
mod postgres;

/// Held answers from an old credential revision cannot change the rotated
/// queue, regardless of whether they accept, retry, or invalidate it.
async fn credential_generations(db: Db) {
	let mock = Mock::new();
	let service = start(db, &all_transports(), &mock).await;
	let endpoint = format!("https://{WEB_HOST}/push/generation-test");
	for reactivate in [false, true] {
		for status in [410, 503, 200] {
			generation_case(&service, &mock, &endpoint, reactivate, status).await;
		}
	}
	// Even inconsistent persisted ownership must never route somebody else's
	// payload.
	let sent = mock.requests_to(WEB_HOST).len();
	assert_eq!(send(&service, "wrong-owner", spec(), &[ALICE]).await.0, 200);
	leptoskit_notifications::push::PushDelivery::update()
		.filter(|row| row.send_key.eq("wrong-owner"))
		.set(|row| row.person, BOB)
		.run(&service.db)
		.await
		.unwrap();
	spawn_round(&service, "wrong-owner", 0).await.unwrap();
	assert_eq!(mock.requests_to(WEB_HOST).len(), sent);
	assert_eq!(
		delivery_status(&service, "wrong-owner").await[0]["state"],
		"skipped"
	);
	assert_eq!(
		send(&service, "wrong-notification-owner", spec(), &[ALICE])
			.await
			.0,
		200
	);
	leptoskit_notifications::Notification::update()
		.filter(|row| row.send_key.eq("wrong-notification-owner"))
		.set(|row| row.person, BOB)
		.run(&service.db)
		.await
		.unwrap();
	spawn_round(&service, "wrong-notification-owner", 0)
		.await
		.unwrap();
	assert_eq!(mock.requests_to(WEB_HOST).len(), sent);
	assert_eq!(
		delivery_status(&service, "wrong-notification-owner").await[0]["state"],
		"skipped"
	);
}

/// Runs one response across a credential transition with the HTTP boundary
/// held.
async fn generation_case(
	service: &Service,
	mock: &Mock,
	endpoint: &str,
	reactivate: bool,
	status: u16,
) {
	let suffix = format!("{reactivate}-{status}");
	let old = format!("old-{suffix}");
	let fresh = format!("fresh-{suffix}");
	let (code, body) = enroll(service, ALICE, web_credential(endpoint)).await;
	assert_eq!(code, 200, "{body}");
	assert_eq!(send(service, &old, spec(), &[ALICE]).await.0, 200);
	mock.scripted(move |_, _| Scripted::Hold(status, Vec::new(), String::new()));
	let held = spawn_round(service, &old, 0);
	mock.wait_held(1).await;
	if reactivate {
		mock.scripted(|_, _| Scripted::Reply(410, Vec::new(), String::new()));
		let invalidate = format!("invalidate-{suffix}");
		assert_eq!(send(service, &invalidate, spec(), &[ALICE]).await.0, 200);
		spawn_round(service, &invalidate, 0).await.unwrap();
	}
	let (_, _, auth, public) = web_subscription(endpoint);
	let rotated = json!({"web_push": {"endpoint": endpoint, "keys": {"p256dh": public, "auth": URL_SAFE_NO_PAD.encode(auth)}}});
	let (code, body) = enroll(service, ALICE, rotated).await;
	assert_eq!(code, 200, "{body}");
	assert_eq!(send(service, &fresh, spec(), &[ALICE]).await.0, 200);
	if !reactivate {
		leptoskit_notifications::push::PushDelivery::update()
			.filter(|row| row.send_key.eq(old.clone()))
			.set(|row| row.next_attempt_at, None)
			.run(&service.db)
			.await
			.unwrap();
	}
	let before = delivery_status(service, &old).await;
	mock.release();
	held.await.unwrap();
	if !reactivate {
		let jobs = rpc::server::jobs::JobRow::query()
			.filter(|row| {
				row.name.eq("push_send") & row.state.eq(rpc::server::jobs::JobState::Pending)
			})
			.all(
				&service
					.db
					.system("inspect retry job in credential fencing test"),
			)
			.await
			.unwrap();
		assert!(
			jobs.iter().any(|job| {
				job.arguments.0["send_key"] == old.as_str() && job.arguments.0["round"] == 1
			}),
			"a fenced response must schedule pending work"
		);
	}
	assert_eq!(
		delivery_status(service, &old).await,
		before,
		"stale {status} changed the old delivery"
	);
	assert_rotated_queue(service, mock, &fresh, &old, reactivate).await;
}

/// Verifies new queued work and old terminal states after a held response
/// lands.
async fn assert_rotated_queue(
	service: &Service,
	mock: &Mock,
	fresh: &str,
	old: &str,
	reactivate: bool,
) {
	let queued = delivery_status(service, fresh).await;
	assert_eq!(queued[0]["state"], "enqueued", "{queued:?}");
	assert_eq!(queued[0]["attempts"], 0);
	assert!(endpoints_of(service, ALICE).await[0]["invalidated_at"].is_null());
	mock.scripted(|_, _| Scripted::Default);
	spawn_round(service, fresh, 0).await.unwrap();
	assert_eq!(
		delivery_status(service, fresh).await[0]["state"],
		"accepted"
	);
	if !reactivate {
		spawn_round(service, old, 1).await.unwrap();
		assert_eq!(delivery_status(service, old).await[0]["state"], "accepted");
	}
}

async fn reactivation_capacity(db: Db) {
	let mock = Mock::new();
	let service = start(db, &all_transports(), &mock).await;
	for n in 1..=10 {
		assert_eq!(
			enroll(&service, ALICE, apns_credential(&device_token(n)))
				.await
				.0,
			200
		);
	}
	mock.scripted(|request, _| {
		if request.url.ends_with(&device_token(1)) || request.url.ends_with(&device_token(2)) {
			Scripted::Reply(
				410,
				Vec::new(),
				json!({"reason":"Unregistered"}).to_string(),
			)
		} else {
			Scripted::Default
		}
	});
	assert_eq!(
		send(&service, "invalidate-two", spec(), &[ALICE]).await.0,
		200
	);
	spawn_round(&service, "invalidate-two", 0).await.unwrap();
	for n in 11..=12 {
		assert_eq!(
			enroll(&service, ALICE, apns_credential(&device_token(n)))
				.await
				.0,
			200
		);
	}
	assert_eq!(
		enroll(&service, ALICE, apns_credential(&device_token(1)))
			.await
			.0,
		400
	);
	assert_eq!(
		enroll(&service, ALICE, apns_credential(&device_token(12)))
			.await
			.0,
		200
	);
	// Revoking one live endpoint frees exactly one slot for two concurrent
	// reactivations.
	let live = endpoints_of(&service, ALICE)
		.await
		.into_iter()
		.find(|row| row["invalidated_at"].is_null())
		.unwrap();
	assert_eq!(
		post(
			&service,
			"revoke_push_endpoint",
			json!({"endpoint":live["id"]}),
			Some(&signed_in(ALICE))
		)
		.await
		.0,
		200
	);
	let (a, b) = tokio::join!(
		enroll(&service, ALICE, apns_credential(&device_token(1))),
		enroll(&service, ALICE, apns_credential(&device_token(2)))
	);
	let mut codes = [a.0, b.0];
	codes.sort_unstable();
	assert_eq!(codes, [200, 400], "{a:?} {b:?}");
	assert_eq!(
		endpoints_of(&service, ALICE)
			.await
			.iter()
			.filter(|row| row["invalidated_at"].is_null())
			.count(),
		10
	);
	revoke_and_reenroll(&service).await;
}

/// An explicitly revoked destination enrolls to a different owner with a new
/// id.
async fn revoke_and_reenroll(service: &Service) {
	let invalid = endpoints_of(service, ALICE)
		.await
		.into_iter()
		.find(|row| row["invalidated_at"].is_string())
		.unwrap();
	let credential = credential_of(service, invalid["id"].as_str().unwrap()).await;
	assert_eq!(
		post(
			service,
			"revoke_push_endpoint",
			json!({"endpoint": invalid["id"]}),
			Some(&signed_in(ALICE))
		)
		.await
		.0,
		200
	);
	let (status, body) = enroll(service, BOB, apns_credential(&credential)).await;
	assert_eq!(status, 200, "{body}");
	let replacement: Value = serde_json::from_str(&body).unwrap();
	assert_ne!(replacement["id"], invalid["id"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_credential_generation_and_capacity_races() {
	credential_generations(db().await).await;
	reactivation_capacity(db().await).await;
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres_credential_generation_and_capacity_races() {
	let Some(postgres) = postgres::Postgres::start() else {
		return;
	};
	for generations in [true, false] {
		let db = Db::connect(&postgres.database()).await.unwrap();
		db.migrate(leptoskit_notifications::POSTGRES_MIGRATIONS)
			.await
			.unwrap();
		if generations {
			credential_generations(db).await;
		} else {
			reactivation_capacity(db).await;
		}
	}
}

/// Drive retry boundaries directly; workers stay paused and no wall-clock wait
/// is needed to exercise a deadline or an eight-request budget.
#[tokio::test(flavor = "multi_thread")]
async fn auth_requests_are_bounded_and_oauth_failures_have_a_horizon() {
	use leptoskit_notifications::Notification;
	use leptoskit_notifications::push::PushDelivery;

	let mock = Mock::new();
	let service = service(&mock, &all_transports()).await;
	assert_eq!(
		enroll(&service, ALICE, fcm_credential("bounded-auth"))
			.await
			.0,
		200
	);
	mock.scripted(|request, _| {
		if request.url.contains(FCM_TOKEN_HOST) {
			Scripted::Reply(
				200,
				Vec::new(),
				json!({"access_token": "test-token", "expires_in": 3600}).to_string(),
			)
		} else {
			Scripted::Reply(401, Vec::new(), "private diagnostic".into())
		}
	});
	assert_eq!(send(&service, "auth-budget", spec(), &[ALICE]).await.0, 200);
	for round in 0..9 {
		// Simulate the queue's scheduled wake without waiting for backoff.
		PushDelivery::update()
			.filter(|row| row.send_key.eq("auth-budget"))
			.set(|row| row.next_attempt_at, None)
			.run(&service.db)
			.await
			.unwrap();
		spawn_round(&service, "auth-budget", round).await.unwrap();
	}
	let status = delivery_status(&service, "auth-budget").await;
	assert_eq!(status[0]["state"], "rejected");
	assert_eq!(status[0]["reason"], "attempts_exhausted");
	assert_eq!(status[0]["attempts"], 8);
	assert_eq!(mock.requests_to(FCM_HOST).len(), 8);

	// A separate runtime has no cached OAuth token. Exchange failures spend
	// no delivery requests, but the visibility horizon still ends recovery.
	let mock = Mock::new();
	let service = self::service(&mock, &all_transports()).await;
	assert_eq!(
		enroll(&service, ALICE, fcm_credential("broken-oauth"))
			.await
			.0,
		200
	);
	mock.scripted(|_, _| Scripted::Reply(400, Vec::new(), "private diagnostic".into()));
	assert_eq!(
		send(&service, "oauth-horizon", spec(), &[ALICE]).await.0,
		200
	);
	spawn_round(&service, "oauth-horizon", 0).await.unwrap();
	let status = delivery_status(&service, "oauth-horizon").await;
	assert_eq!(status[0]["state"], "retrying");
	assert_eq!(status[0]["reason"], "operator_configuration");
	assert_eq!(status[0]["attempts"], 0);
	Notification::update()
		.filter(|row| row.send_key.eq("oauth-horizon"))
		.set(
			|row| row.visible_at,
			leptoskit_db::now() - chrono::Duration::hours(25),
		)
		.run(&service.db)
		.await
		.unwrap();
	PushDelivery::update()
		.filter(|row| row.send_key.eq("oauth-horizon"))
		.set(|row| row.next_attempt_at, None)
		.run(&service.db)
		.await
		.unwrap();
	spawn_round(&service, "oauth-horizon", 1).await.unwrap();
	let status = delivery_status(&service, "oauth-horizon").await;
	assert_eq!(status[0]["reason"], "horizon_exceeded");
	assert_eq!(status[0]["attempts"], 0);
	assert_eq!(mock.requests_to(FCM_TOKEN_HOST).len(), 1);
	assert!(mock.requests_to(FCM_HOST).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn provider_wait_is_not_shortened_and_cancellation_prevents_another_request() {
	use leptoskit_notifications::push::PushDelivery;
	let mock = Mock::new();
	let service = service(&mock, &all_transports()).await;
	assert_eq!(
		enroll(&service, ALICE, apns_credential(&device_token(91)))
			.await
			.0,
		200
	);
	mock.scripted(|_, _| {
		Scripted::Reply(
			429,
			vec![("retry-after".into(), "25200".into())],
			String::new(),
		)
	});
	assert_eq!(send(&service, "long-wait", spec(), &[ALICE]).await.0, 200);
	let before = leptoskit_db::now();
	spawn_round(&service, "long-wait", 0).await.unwrap();
	let delivery = PushDelivery::query()
		.filter(|row| row.send_key.eq("long-wait"))
		.one(&service.db)
		.await
		.unwrap();
	assert!(delivery.next_attempt_at.unwrap() >= before + chrono::Duration::hours(7));
	// Simulate cancellation committed while the delivery waited. The public
	// cancel operation itself only accepts sends that are not visible yet.
	leptoskit_notifications::Notification::update()
		.filter(|row| row.send_key.eq("long-wait"))
		.set(|row| row.cancelled_at, Some(leptoskit_db::now()))
		.run(&service.db)
		.await
		.unwrap();
	PushDelivery::update()
		.filter(|row| row.send_key.eq("long-wait"))
		.set(|row| row.next_attempt_at, None)
		.run(&service.db)
		.await
		.unwrap();
	spawn_round(&service, "long-wait", 1).await.unwrap();
	assert_eq!(mock.requests_to(APNS_HOST).len(), 1);
	assert_eq!(
		delivery_status(&service, "long-wait").await[0]["reason"],
		"cancelled"
	);
}

/// A lost revoke response can be replayed only by its original actor.
#[tokio::test(flavor = "multi_thread")]
async fn revoke_receipts_replay_without_crossing_accounts_or_new_enrollments() {
	let mock = Mock::new();
	let service = service(&mock, &all_transports()).await;
	let credential = web_credential(&format!("https://{WEB_HOST}/push/receipt"));
	let (status, body) = enroll(&service, ALICE, credential.clone()).await;
	assert_eq!(status, 200, "{body}");
	let old: Value = serde_json::from_str(&body).unwrap();
	let key = format!("notifications:disable:{}", old["id"].as_str().unwrap());
	let revoke = |person| {
		reqwest::Client::new()
			.post(format!(
				"http://{}{PREFIX}/rpc/revoke_push_endpoint",
				service.address
			))
			.bearer_auth(signed_in(person))
			.header("idempotency-key", &key)
			.json(&json!({"endpoint": old["id"]}))
			.send()
	};
	assert_eq!(revoke(ALICE).await.unwrap().status().as_u16(), 200);
	let (status, body) = enroll(&service, BOB, credential).await;
	assert_eq!(status, 200, "{body}");
	let new: Value = serde_json::from_str(&body).unwrap();
	assert_ne!(new["id"], old["id"]);
	// Alice can recover her acknowledgement after the destination is reused.
	assert_eq!(revoke(ALICE).await.unwrap().status().as_u16(), 200);
	// Bob cannot reuse Alice's receipt, even with exactly the same key and
	// args.
	assert_eq!(revoke(BOB).await.unwrap().status().as_u16(), 404);
	let bob = endpoints_of(&service, BOB).await;
	assert_eq!(bob.len(), 1);
	assert_eq!(bob[0]["id"], new["id"]);
}

// The round engine: every fanout batch pushed, rounds bounded, and one
// delivery's failure kept to itself.

/// A topic send larger than two batches pushes every batch, however its
/// jobs interleave. With one worker running them one at a time, the first
/// batch's round runs — and finds nothing left waiting — before the second
/// batch is even written; the later batches' deliveries are still all
/// attempted.
#[tokio::test(flavor = "multi_thread")]
async fn every_fanout_batch_is_pushed() {
	let mock = Mock::new();
	let mut service = service(&mock, &all_transports()).await;
	let audience: Vec<Uuid> = (0..1_201_u128)
		.map(|n| Uuid::from_u128(0x0F1A_0000_0000_0000_0000_0000_0000_0000 | n))
		.collect();

	for chunk in audience.chunks(1_000) {
		let (status, body) = post(
			&service,
			"subscribe_to_topic",
			json!({ "topic": "news.batches", "persons": chunk }),
			Some(TOKEN),
		)
		.await;
		assert_eq!(status, 200, "{body}");
	}

	enroll_directly(&service, &audience).await;

	let (status, body) = post(
		&service,
		"send",
		json!({
			"idempotency_key": "every-batch",
			"spec": spec(),
			"target": { "topic": "news.batches" },
		}),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");

	service.jobs = Some(
		leptoskit_notifications::service::workers_with_push(
			service.db.clone(),
			service.push.clone(),
		)
		.workers(1)
		.poll(Duration::from_millis(50))
		.start(),
	);

	all_accepted(&service, "every-batch", 1_201).await;
	assert_eq!(in_state(&service, "every-batch", "enqueued").await, 0);
	assert_eq!(
		mock.requests_to(APNS_HOST).len(),
		1_201,
		"each delivery went out once"
	);
}

/// One round attempts a bounded share and hands the rest on at once: two
/// thousand due deliveries take more than one round, and every one of them
/// is accepted.
#[tokio::test(flavor = "multi_thread")]
async fn a_round_attempts_a_bounded_share_and_hands_on_the_rest() {
	let mock = Mock::new();
	let mut service = service(&mock, &all_transports()).await;
	let people: Vec<Uuid> = (0..1_000_u128)
		.map(|n| Uuid::from_u128(0x0B0D_0000_0000_0000_0000_0000_0000_0000 | n))
		.collect();
	let twice: Vec<Uuid> = people.iter().chain(&people).copied().collect();
	enroll_directly(&service, &twice).await;

	let (status, body) = send(&service, "bounded", spec(), &people).await;
	assert_eq!(status, 200, "{body}");
	assert_eq!(in_state(&service, "bounded", "enqueued").await, 2_000);

	spawn_round(&service, "bounded", 0)
		.await
		.expect("the round runs");
	assert_eq!(
		mock.requests_to(APNS_HOST).len(),
		1_000,
		"one round attempts its share, not the whole send"
	);

	let next = rpc::server::jobs::JobRow::query()
		.filter(|job| job.name.eq("push_send") & job.state.eq(rpc::server::jobs::JobState::Pending))
		.all(&service.db.system("read the next round in a test"))
		.await
		.unwrap()
		.into_iter()
		.find(|job| job.arguments.0["round"] == 1)
		.expect("the next round is queued");
	assert!(
		next.run_at <= leptoskit_db::now(),
		"and due at once: {:?}",
		next.run_at
	);
	assert!(
		next.serial_key.is_some(),
		"one send's rounds run one at a time"
	);

	service.resume();
	all_accepted(&service, "bounded", 2_000).await;
	assert_eq!(
		mock.requests_to(APNS_HOST).len(),
		2_000,
		"each delivery went out once"
	);
}

/// A stored options value this build can't read stalls its own delivery —
/// the way an operator-side failure does — and nothing else: its peer in
/// the same round goes out, and the round ends cleanly.
#[tokio::test(flavor = "multi_thread")]
async fn an_unreadable_delivery_never_fails_its_round() {
	let mock = Mock::new();
	let service = service(&mock, &all_transports()).await;
	assert_eq!(
		enroll(&service, ALICE, apns_credential(&device_token(1)))
			.await
			.0,
		200
	);
	assert_eq!(
		enroll(&service, BOB, apns_credential(&device_token(3)))
			.await
			.0,
		200
	);
	let (status, body) = send(&service, "unreadable", spec(), &[ALICE, BOB]).await;
	assert_eq!(status, 200, "{body}");

	// Alice's row carries options written by a build that knows a kind
	// this one doesn't.
	leptoskit_notifications::Notification::update()
		.filter(|row| row.send_key.eq("unreadable") & row.person.eq(ALICE))
		.set(|row| row.options, Some(r#"{"kind":"hologram"}"#.to_owned()))
		.run(&service.db)
		.await
		.unwrap();

	spawn_round(&service, "unreadable", 0)
		.await
		.expect("the round ends cleanly");

	let items = delivery_status(&service, "unreadable").await;
	let of = |person: Uuid| {
		items
			.iter()
			.find(|item| item["person"] == json!(person))
			.cloned()
			.expect("a delivery")
	};
	assert_eq!(of(BOB)["state"], "accepted", "{items:?}");
	assert_eq!(of(ALICE)["state"], "retrying", "{items:?}");
	assert_eq!(of(ALICE)["reason"], "operator_configuration", "{items:?}");
	assert_eq!(of(ALICE)["attempts"], 0, "nothing was pushed for it");
	assert_eq!(mock.requests_to(APNS_HOST).len(), 1, "only bob's went out");
}

/// A notification row gone between an attempt and its outcome's write
/// ends that delivery as cancelled, the way an attempt that finds it gone
/// does, and the round ends cleanly.
#[tokio::test(flavor = "multi_thread")]
async fn a_row_gone_mid_attempt_ends_its_delivery_not_the_round() {
	let mock = Mock::new();
	let service = service(&mock, &all_transports()).await;
	assert_eq!(
		enroll(&service, ALICE, apns_credential(&device_token(1)))
			.await
			.0,
		200
	);
	let (status, body) = send(&service, "vanishing", spec(), &[ALICE]).await;
	assert_eq!(status, 200, "{body}");

	mock.scripted(|request, _| {
		if request.url.contains(APNS_HOST) {
			return Scripted::Hold(503, Vec::new(), String::new());
		}

		Scripted::Default
	});

	let racing = spawn_round(&service, "vanishing", 0);
	mock.wait_held(1).await;
	leptoskit_notifications::Notification::query()
		.filter(|row| row.send_key.eq("vanishing"))
		.delete(&service.db)
		.await
		.unwrap();
	mock.release();
	racing.await.expect("the round ends cleanly");

	let items = delivery_status(&service, "vanishing").await;
	assert_eq!(items[0]["state"], "skipped", "{items:?}");
	assert_eq!(items[0]["reason"], "cancelled", "{items:?}");
	assert_eq!(items[0]["attempts"], 1, "the request it made is counted");
}

/// A test send holds no transaction while a provider answers: a write made
/// while its request is in flight finishes before the provider answers it.
#[tokio::test(flavor = "multi_thread")]
async fn a_test_send_holds_up_no_write_while_a_provider_answers() {
	let mock = Mock::new();
	let service = service(&mock, &all_transports()).await;
	assert_eq!(
		enroll(&service, ALICE, apns_credential(&device_token(1)))
			.await
			.0,
		200
	);
	mock.scripted(|request, _| {
		if request.url.contains(APNS_HOST) {
			return Scripted::Hold(200, Vec::new(), String::new());
		}

		Scripted::Default
	});

	let address = service.address;
	let probe = tokio::spawn(async move {
		let response = reqwest::Client::new()
			.post(format!("http://{address}{PREFIX}/rpc/send_test"))
			.bearer_auth(TOKEN)
			.json(&json!({ "spec": spec(), "person": ALICE }))
			.send()
			.await
			.unwrap();
		(response.status().as_u16(), response.text().await.unwrap())
	});
	mock.wait_held(1).await;

	let (status, body) = tokio::time::timeout(
		Duration::from_secs(10),
		post(
			&service,
			"subscribe_to_topic",
			json!({ "topic": "news.meanwhile", "persons": [BOB] }),
			Some(TOKEN),
		),
	)
	.await
	.expect("the write finishes while the provider still holds the test send");
	assert_eq!(status, 200, "{body}");

	mock.release();
	let (status, body) = probe.await.expect("the test send answers");
	assert_eq!(status, 200, "{body}");
	let outcomes: Value = serde_json::from_str(&body).unwrap();
	assert_eq!(outcomes[0]["state"], "accepted", "{outcomes}");
}

/// The longest idempotency key pushes too, from a recipients send and a
/// topic send: the push rounds name the send by its hash when their own
/// keys would be too long.
#[tokio::test(flavor = "multi_thread")]
async fn the_longest_key_pushes() {
	let mock = Mock::new();
	let mut service = service(&mock, &all_transports()).await;
	assert_eq!(
		enroll(&service, ALICE, apns_credential(&device_token(1)))
			.await
			.0,
		200
	);
	let (status, body) = post(
		&service,
		"subscribe_to_topic",
		json!({ "topic": "news.long", "persons": [ALICE] }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	service.resume();

	for (name, target) in [
		("r", json!({ "recipients": [ALICE] })),
		("t", json!({ "topic": "news.long" })),
	] {
		let key = format!("{name}{}", "k".repeat(254));
		let (status, body) = post(
			&service,
			"send",
			json!({ "idempotency_key": key, "spec": spec(), "target": target }),
			Some(TOKEN),
		)
		.await;
		assert_eq!(status, 200, "{name}: {body}");
		assert_eq!(
			settled(&service, &key).await,
			vec![("accepted".to_owned(), None)],
			"{name}"
		);
	}
}

/// A send dated more than a day back is a send from now: its push goes out
/// rather than ending past its delivery window, and the inbox shows it.
#[tokio::test(flavor = "multi_thread")]
async fn an_earlier_visible_at_is_now() {
	let mock = Mock::new();
	let mut service = service(&mock, &all_transports()).await;
	assert_eq!(
		enroll(&service, ALICE, apns_credential(&device_token(1)))
			.await
			.0,
		200
	);
	service.resume();

	let earlier = (chrono::Utc::now() - chrono::Duration::hours(48)).to_rfc3339();
	let (status, body) = send(
		&service,
		"backdated",
		json!({ "category": "news", "title": "Backdated", "visible_at": earlier }),
		&[ALICE],
	)
	.await;
	assert_eq!(status, 200, "{body}");

	assert_eq!(
		settled(&service, "backdated").await,
		vec![("accepted".to_owned(), None)],
		"inside its window, which starts now"
	);

	let (status, body) = post(
		&service,
		"inbox",
		json!({ "page": { "limit": 10 } }),
		Some(&signed_in(ALICE)),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let page: Value = serde_json::from_str(&body).unwrap();
	assert_eq!(page["items"][0]["title"], "Backdated", "{page}");
}

// Previews.

/// One case of [`a_preview_is_the_bytes_each_provider_is_sent`]: a spec,
/// and what the preview should say was trimmed on each platform — APNs,
/// FCM, Web Push — or `None` where the push never goes out.
struct PreviewCase {
	name: &'static str,
	spec: Value,
	/// The locale the person reads variants in, set as their attribute.
	locale: Option<&'static str>,
	trimmed: [Option<&'static [&'static str]>; 3],
}

#[expect(
	clippy::too_many_lines,
	reason = "one case for each option a push can carry"
)]
fn preview_cases() -> Vec<PreviewCase> {
	const NONE: Option<&[&str]> = Some(&[]);
	const ONE: Option<&[&str]> = Some(&["platform_and_image"]);
	const TWO: Option<&[&str]> = Some(&["platform_and_image", "sound_badge_link"]);
	const ALL: Option<&[&str]> = Some(&["platform_and_image", "sound_badge_link", "data"]);

	vec![
		PreviewCase {
			name: "plain",
			spec: spec(),
			locale: None,
			trimmed: [NONE, NONE, NONE],
		},
		PreviewCase {
			name: "data-and-priority",
			spec: json!({
				"category": "news",
				"title": "It ships",
				"body": "Push is here.",
				"data": json!({ "order": "42", "nested": { "a": [1, 2] } }).to_string(),
				"options": { "priority": "high" },
			}),
			locale: None,
			trimmed: [NONE, NONE, NONE],
		},
		PreviewCase {
			name: "every-option",
			spec: json!({
				"category": "billing",
				"title": "Invoice paid",
				"body": "See the attachment.",
				"data": json!({ "invoice": "42" }).to_string(),
				"options": {
					"priority": "normal",
					"image": "https://cdn.example.test/paid.png",
					"sound": { "named": "chime" },
					"badge": { "set": 7 },
					"link": "/invoice/42",
					"apple": {
						"subtitle": "Invoice #42",
						"thread_id": "billing",
						"category": "BILLING",
						"interruption_level": "time_sensitive",
						"relevance_score": 0.9,
					},
					"android": {
						"channel_id": "inbox",
						"color": "#10b981",
						"icon": "ic_income",
						"tag": "billing",
					},
					"web": {
						"icon": "https://cdn.example.test/icon.png",
						"badge_icon": "/badge.png",
						"actions": [{ "action": "open", "title": "Open" }],
						"require_interaction": true,
						"dir": "rtl",
						"lang": "de",
					},
				},
			}),
			locale: None,
			trimmed: [NONE, NONE, NONE],
		},
		PreviewCase {
			name: "silent-sound-and-unread-badge",
			spec: json!({
				"category": "news",
				"title": "Quietly",
				"body": "Nothing rings.",
				"options": { "sound": "silent", "badge": "unread" },
			}),
			locale: None,
			trimmed: [NONE, NONE, NONE],
		},
		PreviewCase {
			name: "default-sound",
			spec: json!({
				"category": "news",
				"title": "Ding",
				"options": { "sound": "default" },
			}),
			locale: None,
			trimmed: [NONE, NONE, NONE],
		},
		PreviewCase {
			name: "silent-data-message",
			spec: json!({
				"category": "sync",
				"title": "sync",
				"data": json!({ "refresh": "orders" }).to_string(),
				"options": { "kind": "data", "badge": { "set": 3 }, "priority": "high" },
			}),
			locale: None,
			// A data message is never pushed to a browser.
			trimmed: [NONE, NONE, None],
		},
		PreviewCase {
			name: "a-variant",
			spec: json!({
				"category": "news",
				"title": "It ships",
				"body": "Push is here.",
				"options": {
					"apple": { "subtitle": "Today" },
					"web": { "actions": [{ "action": "open", "title": "Open" }] },
				},
				"variants": {
					"fr": {
						"title": "C'est parti",
						"body": "Les notifications sont là.",
						"subtitle": "Aujourd'hui",
						"actions": { "open": "Ouvrir" },
					},
				},
			}),
			locale: Some("fr-CA"),
			trimmed: [NONE, NONE, NONE],
		},
		// A collapse key of the app's own and a Firebase-era channel.
		PreviewCase {
			name: "collapse-key-and-raw-channel",
			spec: json!({
				"category": "scores",
				"title": "2 : 1",
				"body": "Full time.",
				"options": {
					"collapse_key": "match-7",
					"android": { "raw_channel_id": "scores" },
				},
			}),
			locale: None,
			trimmed: [NONE, NONE, NONE],
		},
		PreviewCase {
			name: "expiring",
			spec: spec_expiring(),
			locale: None,
			trimmed: [NONE, NONE, NONE],
		},
		// Past Apple's 4 KiB until the platform options, the image, the
		// sound, the badge, and the link have all gone; the data stays.
		PreviewCase {
			name: "trimmed-twice-on-apns",
			spec: json!({
				"category": "billing",
				"title": "T",
				"body": "x".repeat(600),
				"data": json!({ "k": "d".repeat(3_300) }).to_string(),
				"options": {
					"image": "https://cdn.example.test/i.png",
					"sound": "default",
					"badge": { "set": 7 },
					"link": format!("/l/{}", "x".repeat(900)),
					"apple": { "subtitle": "s".repeat(100) },
				},
			}),
			locale: None,
			trimmed: [TWO, TWO, ALL],
		},
		// Past FCM's count until the platform options and the image go,
		// and not after.
		PreviewCase {
			name: "trimmed-once-on-fcm",
			spec: json!({
				"category": "billing",
				"title": "T",
				"body": "y".repeat(1_050),
				"data": json!({ "k": "d".repeat(2_840) }).to_string(),
				"options": {
					"image": "https://cdn.example.test/i.png",
					"badge": { "set": 7 },
					"link": "/l",
					"android": { "channel_id": "inbox", "tag": "billing" },
				},
			}),
			locale: None,
			trimmed: [NONE, ONE, ALL],
		},
		// Only the id-only envelope fits anywhere.
		PreviewCase {
			name: "only-the-envelope-fits",
			spec: json!({
				"category": "news",
				"title": "It ships",
				"body": "x".repeat(1_900),
				"data": json!({ "x": "x".repeat(4_000) }).to_string(),
				"options": { "sound": "default", "badge": { "set": 7 } },
			}),
			locale: None,
			trimmed: [ALL, ALL, ALL],
		},
	]
}

/// The preview of `spec` for the notification `id`.
async fn preview(service: &Service, spec: &Value, sample: Value) -> Vec<Value> {
	let (status, body) = post(
		service,
		"preview_push",
		json!({ "spec": spec, "sample": sample }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	serde_json::from_str(&body).unwrap()
}

/// A preview's headers, as the pairs a request carries.
fn preview_headers(preview: &Value) -> Vec<(String, String)> {
	preview["headers"]
		.as_array()
		.unwrap()
		.iter()
		.map(|header| {
			(
				header["name"].as_str().unwrap().to_owned(),
				header["value"].as_str().unwrap().to_owned(),
			)
		})
		.collect()
}

/// A posted request's headers, without the ones a preview leaves out: the
/// authorization, and the APNs topic that names the app.
fn posted_headers(seen: &Seen) -> Vec<(String, String)> {
	seen.headers
		.iter()
		.filter(|(name, _)| name != "authorization" && name != "apns-topic")
		.cloned()
		.collect()
}

/// For every options case — plain, every option at once, each sound, a
/// badge set and a badge resolved from the unread count, a silent data
/// message, a translated variant, an expiry, and payloads trimmed once,
/// twice, and down to the envelope — the preview's body is byte for byte
/// the body the provider was posted through the injected boundary, its
/// headers are the posted headers without the credentials, the bytes it
/// counts are the bytes that went, and what it says was trimmed is the
/// documented order's prefix.
///
/// The only stand-ins are the ones a preview documents: the notification's
/// id (read from the posted request and passed as the sample), the FCM
/// token (the placeholder, replaced here by the real one), and for Web
/// Push the encryption, which this test opens with the receiver's own key.
#[expect(
	clippy::too_many_lines,
	reason = "one send per case, followed to each of the three providers"
)]
#[tokio::test(flavor = "multi_thread")]
async fn a_preview_is_the_bytes_each_provider_is_sent() {
	let mock = Mock::new();
	let mut service = service(&mock, &all_transports()).await;
	mock.scripted(passing_exchange());
	service.resume();

	for (index, case) in preview_cases().into_iter().enumerate() {
		let n = u8::try_from(index).unwrap() + 1;
		let who = Uuid::from_u128(0x9E11_0000_0000_0000_0000_0000_0000_0000 | u128::from(n));
		let apns_token = device_token(100 + n);
		let fcm_token = format!("fcm-preview-{n}");
		let web_endpoint = format!("https://{WEB_HOST}/push/preview{n:06}");
		let (receiver, web, auth_secret, public) = web_subscription(&web_endpoint);
		let name = case.name;

		for credential in [
			apns_credential(&apns_token),
			fcm_credential(&fcm_token),
			web,
		] {
			let (status, body) = enroll(&service, who, credential).await;
			assert_eq!(status, 200, "{name}: {body}");
		}

		if let Some(locale) = case.locale {
			let (status, body) = post(
				&service,
				"set_person_attributes",
				json!({ "persons": [who], "set": { "locale": locale }, "remove": [] }),
				Some(TOKEN),
			)
			.await;
			assert_eq!(status, 200, "{name}: {body}");
		}

		let key = format!("preview-{name}");
		let (status, body) = send(&service, &key, case.spec.clone(), &[who]).await;
		assert_eq!(status, 200, "{name}: {body}");
		let outcomes = settled(&service, &key).await;
		assert_eq!(outcomes.len(), 3, "{name}: {outcomes:?}");

		// What went out, found by this case's own destinations.
		let apns = mock
			.requests_to(APNS_HOST)
			.into_iter()
			.find(|seen| seen.url.ends_with(&apns_token));
		let fcm = mock
			.requests_to(FCM_HOST)
			.into_iter()
			.find(|seen| seen.json()["message"]["token"] == fcm_token.as_str());
		let web = mock
			.requests_to(WEB_HOST)
			.into_iter()
			.find(|seen| seen.url == web_endpoint);
		let id = apns
			.as_ref()
			.and_then(|seen| seen.header("apns-id"))
			.unwrap_or_else(|| panic!("{name}: the APNs push went out"))
			.to_owned();

		// The person has exactly this one unread notification, which is
		// what an unread badge resolved to when the push went.
		let mut sample = json!({ "notification": id, "unread": 1 });

		if let Some(locale) = case.locale {
			sample["locale"] = json!(locale);
		}

		let previews = preview(&service, &case.spec, sample).await;
		assert_eq!(
			previews
				.iter()
				.map(|preview| preview["platform"].as_str().unwrap())
				.collect::<Vec<_>>(),
			["apns", "fcm", "web_push"],
			"{name}"
		);

		for (preview, trimmed) in previews.iter().zip(case.trimmed) {
			let platform = preview["platform"].as_str().unwrap();

			if let Some(trimmed) = trimmed {
				assert_eq!(
					preview.get("trimmed").unwrap_or(&json!([])),
					&json!(trimmed),
					"{name} on {platform}"
				);
				assert_eq!(preview["fits"], true, "{name} on {platform}");
				assert!(preview.get("skipped").is_none(), "{name} on {platform}");
			} else {
				assert_eq!(
					preview["skipped"], "platform_unsupported",
					"{name} on {platform}"
				);
				assert_eq!(preview["body"], "", "{name} on {platform}");
			}
		}

		// APNs: the body and the headers, exactly.
		let apns = apns.expect("checked above");
		assert_eq!(
			previews[0]["body"].as_str().unwrap().as_bytes(),
			apns.body.as_slice(),
			"{name}: the APNs body"
		);
		assert_eq!(
			preview_headers(&previews[0]),
			posted_headers(&apns),
			"{name}"
		);
		assert_eq!(
			previews[0]["bytes"],
			apns.body.len(),
			"{name}: Apple counts the encoded payload"
		);
		assert_eq!(previews[0]["limit"], 4_096, "{name}");

		// FCM: the same body once the placeholder is the device's token.
		// An expiry rides as the seconds left, measured a moment apart.
		let fcm = fcm.unwrap_or_else(|| panic!("{name}: the FCM push went out"));
		let previewed = previews[1]["body"].as_str().unwrap();
		assert!(
			previewed.contains("\"token\":\"<device token>\""),
			"{name}: {previewed}"
		);
		let previewed = previewed.replace("<device token>", &fcm_token);

		if case.spec.get("expires_at").is_some() {
			let seconds = |body: &Value| {
				body["message"]["android"]["ttl"]
					.as_str()
					.and_then(|ttl| ttl.strip_suffix('s'))
					.and_then(|ttl| ttl.parse::<i64>().ok())
					.unwrap_or_else(|| panic!("{name}: a ttl in {body}"))
			};
			let mut previewed: Value = serde_json::from_str(&previewed).unwrap();
			let mut posted = fcm.json();
			assert!(
				(0..=120).contains(&(seconds(&posted) - seconds(&previewed))),
				"{name}: the preview was made after the push"
			);

			for body in [&mut previewed, &mut posted] {
				body["message"]["android"]["ttl"] = json!("");
				body["message"]["apns"]["headers"]["apns-expiration"] = json!("");
			}

			assert_eq!(
				previewed, posted,
				"{name}: the FCM body, apart from the time left"
			);
		} else {
			assert_eq!(
				previewed.as_bytes(),
				fcm.body.as_slice(),
				"{name}: the FCM body"
			);
		}

		assert_eq!(
			preview_headers(&previews[1]),
			posted_headers(&fcm),
			"{name}"
		);

		// Web Push: the envelope the browser decrypts, and the headers.
		match (web, case.trimmed[2]) {
			(Some(web), Some(_)) => {
				let point = URL_SAFE_NO_PAD.decode(&public).unwrap();
				let opened = open_aes128gcm(&web.body, receiver, &point, &auth_secret);
				assert_eq!(
					previews[2]["body"].as_str().unwrap().as_bytes(),
					opened.as_slice(),
					"{name}: the Web Push envelope"
				);
				assert_eq!(
					previews[2]["bytes"],
					web.body.len(),
					"{name}: the push service counts the encrypted body"
				);

				let (mut previewed, mut posted) =
					(preview_headers(&previews[2]), posted_headers(&web));

				if case.spec.get("expires_at").is_some() {
					for headers in [&mut previewed, &mut posted] {
						headers.retain(|(name, _)| name != "ttl");
					}
				}

				assert_eq!(previewed, posted, "{name}");
			}
			(None, None) => {}
			(web, expected) => {
				panic!(
					"{name}: a Web Push request {} and the preview {}",
					if web.is_some() {
						"went out"
					} else {
						"did not go out"
					},
					if expected.is_some() {
						"says one would"
					} else {
						"says none would"
					},
				);
			}
		}
	}
}

/// A push that fits nowhere is previewed as one that would not be sent:
/// `fits` is false after every trim, as the delivery ends `oversized`
/// with no request.
#[tokio::test(flavor = "multi_thread")]
async fn a_preview_says_when_nothing_would_be_sent() {
	let mock = Mock::new();
	let mut service = service(&mock, &all_transports()).await;
	assert_eq!(
		enroll(&service, ALICE, apns_credential(&device_token(1)))
			.await
			.0,
		200
	);
	service.resume();

	// A body of backslashes doubles when it is encoded: past 4 KiB with
	// nothing left to trim.
	let spec = json!({
		"category": "news",
		"title": "\\".repeat(200),
		"body": "\\".repeat(2_000),
	});
	let (status, body) = send(&service, "fits-nowhere", spec.clone(), &[ALICE]).await;
	assert_eq!(status, 200, "{body}");
	assert_eq!(
		settled(&service, "fits-nowhere").await,
		vec![("rejected".to_owned(), Some("oversized".to_owned()))]
	);
	assert!(mock.requests_to(APNS_HOST).is_empty(), "nothing was posted");

	let previews = preview(&service, &spec, json!({})).await;
	let apns = &previews[0];
	assert_eq!(apns["fits"], false, "{apns}");
	assert_eq!(
		apns["trimmed"],
		json!(["platform_and_image", "sound_badge_link", "data"])
	);
	assert!(apns["bytes"].as_u64().unwrap() > 4_096, "{apns}");
}

/// A preview needs no push configuration and no endpoint, reads nothing
/// about anyone, answers only the app, and refuses what a send would.
#[tokio::test(flavor = "multi_thread")]
async fn only_the_app_previews_and_a_bad_spec_is_refused() {
	let service = start_push(db().await, None).await;

	let previews = preview(&service, &spec(), json!({})).await;
	assert_eq!(previews.len(), 3, "with no transport configured");
	let apns: Value = serde_json::from_str(previews[0]["body"].as_str().unwrap()).unwrap();
	assert_eq!(apns["aps"]["alert"]["title"], "It ships");
	assert_eq!(
		apns["lkn"], "00000000-0000-0000-0000-000000000000",
		"the nil id stands in for the notification's"
	);

	// Leaving the sample out is the same as an empty one.
	let (status, body) = post(
		&service,
		"preview_push",
		json!({ "spec": spec() }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	assert_eq!(
		serde_json::from_str::<Value>(&body).unwrap(),
		json!(previews)
	);

	for token in [None, Some(signed_in(ALICE))] {
		let (status, body) = post(
			&service,
			"preview_push",
			json!({ "spec": spec() }),
			token.as_deref(),
		)
		.await;
		assert_eq!(status, 403, "{body}");
	}

	for (bad, names) in [
		(json!({ "category": "news", "title": "" }), "title"),
		(
			json!({ "category": "news", "title": "t", "options": { "link": "javascript:alert(1)" } }),
			"link",
		),
	] {
		let (status, body) = post(
			&service,
			"preview_push",
			json!({ "spec": bad }),
			Some(TOKEN),
		)
		.await;
		assert_eq!(status, 400, "{body}");
		assert!(body.contains(names), "{body}");
	}

	let (status, body) = post(
		&service,
		"preview_push",
		json!({ "spec": spec(), "sample": { "locale": "not a locale" } }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 400, "{body}");
}

/// One number read from the database.
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

/// The first request to `host` that `pick` accepts.
fn request_where(mock: &Mock, host: &str, pick: impl Fn(&Seen) -> bool) -> Seen {
	mock.requests_to(host)
		.into_iter()
		.find(|seen| pick(seen))
		.unwrap_or_else(|| panic!("a request to {host}"))
}

/// A send that names a collapse key of its own carries it wherever a
/// provider collapses: `apns-collapse-id` at Apple, the `collapse_key`
/// and — so a banner already shown is replaced too — the notification's
/// `tag` at FCM, and the `topic` header and the worker's tag on the web.
/// The notification's own id stays the id everywhere else, an explicit
/// Android tag wins over the derived one, and a key no provider would
/// take is refused by name.
#[tokio::test(flavor = "multi_thread")]
async fn an_app_chosen_collapse_key_rides_every_provider() {
	let mock = Mock::new();
	let mut service = service(&mock, &all_transports()).await;
	mock.scripted(passing_exchange());
	service.resume();

	let apns_token = device_token(61);
	let web_endpoint = format!("https://{WEB_HOST}/push/collapse-key-1");
	let (receiver, web, auth_secret, public) = web_subscription(&web_endpoint);

	for credential in [
		apns_credential(&apns_token),
		fcm_credential("fcm-collapse-1"),
		web,
	] {
		let (status, body) = enroll(&service, ALICE, credential).await;
		assert_eq!(status, 200, "{body}");
	}

	let mut spec = spec();
	spec["options"] = json!({ "collapse_key": "score-update" });
	let (status, body) = send(&service, "collapse-1", spec.clone(), &[ALICE]).await;
	assert_eq!(status, 200, "{body}");
	assert_eq!(settled(&service, "collapse-1").await.len(), 3);

	let apns = request_where(&mock, APNS_HOST, |seen| seen.url.ends_with(&apns_token));
	assert_eq!(apns.header("apns-collapse-id"), Some("score-update"));
	let id = apns.header("apns-id").expect("an id").to_owned();
	Uuid::parse_str(&id).expect("the request id is still the notification's id");
	assert_eq!(apns.json()["lkn"], id.as_str());

	let fcm = request_where(&mock, FCM_HOST, |seen| {
		seen.json()["message"]["token"] == "fcm-collapse-1"
	})
	.json();
	let message = &fcm["message"];
	assert_eq!(message["android"]["collapse_key"], "score-update");
	assert_eq!(
		message["android"]["notification"]["tag"], "score-update",
		"a shown banner is replaced by its tag: {message}"
	);
	assert_eq!(
		message["apns"]["headers"]["apns-collapse-id"],
		"score-update"
	);
	assert_eq!(message["data"]["lkn"], id.as_str());

	let web = request_where(&mock, WEB_HOST, |seen| seen.url == web_endpoint);
	assert_eq!(web.header("topic"), Some("score-update"));
	let point = URL_SAFE_NO_PAD.decode(&public).unwrap();
	let envelope: Value =
		serde_json::from_slice(&open_aes128gcm(&web.body, receiver, &point, &auth_secret)).unwrap();
	assert_eq!(envelope["lktag"], "score-update", "{envelope}");
	assert_eq!(envelope["lkn"], id.as_str());

	// An Android tag the send names itself is the tag.
	spec["options"] = json!({
		"collapse_key": "score-update",
		"android": { "tag": "match-7" },
	});
	let (status, body) = send(&service, "collapse-2", spec.clone(), &[ALICE]).await;
	assert_eq!(status, 200, "{body}");
	assert_eq!(settled(&service, "collapse-2").await.len(), 3);
	let tagged = mock
		.requests_to(FCM_HOST)
		.into_iter()
		.map(|seen| seen.json())
		.find(|body| body["message"]["android"]["notification"]["tag"] == "match-7")
		.expect("the second FCM push");
	assert_eq!(tagged["message"]["android"]["collapse_key"], "score-update");

	// A data message collapses too, and shows nothing to tag.
	let (status, body) = send(
		&service,
		"collapse-3",
		json!({
			"category": "news",
			"title": "sync",
			"data": json!({ "k": "v" }).to_string(),
			"options": { "kind": "data", "collapse_key": "sync" },
		}),
		&[ALICE],
	)
	.await;
	assert_eq!(status, 200, "{body}");
	settled(&service, "collapse-3").await;
	let silent = mock
		.requests_to(FCM_HOST)
		.into_iter()
		.map(|seen| seen.json())
		.find(|body| body["message"]["android"]["collapse_key"] == "sync")
		.expect("the data push");
	assert!(
		silent["message"]["android"].get("notification").is_none(),
		"{silent}"
	);

	for (key, why) in [
		(json!(""), "empty"),
		(json!("x".repeat(33)), "past the web's 32 characters"),
		(json!("score update"), "a space"),
		(json!("score/update"), "not URL-safe"),
	] {
		spec["options"] = json!({ "collapse_key": key });
		let (status, body) = send(&service, "collapse-bad", spec.clone(), &[ALICE]).await;
		assert_eq!(status, 400, "{why}: {body}");
		assert!(body.contains("options.collapse_key"), "{why}: {body}");
	}
}

/// `options.push_ttl` is how long a provider may hold the push, counted
/// from when the notification becomes visible, and is not the inbox
/// item's expiry: the providers are told the lifetime, the inbox row
/// keeps no expiry, and when both are set the providers get the earlier.
#[tokio::test(flavor = "multi_thread")]
async fn a_push_lifetime_is_not_the_inbox_expiry() {
	let mock = Mock::new();
	let mut service = service(&mock, &all_transports()).await;
	mock.scripted(passing_exchange());
	service.resume();

	let apns_token = device_token(62);
	let web_endpoint = format!("https://{WEB_HOST}/push/lifetime-1");

	for credential in [
		apns_credential(&apns_token),
		fcm_credential("fcm-lifetime-1"),
		web_credential(&web_endpoint),
	] {
		let (status, body) = enroll(&service, ALICE, credential).await;
		assert_eq!(status, 200, "{body}");
	}

	let seconds = |text: Option<&str>| -> i64 {
		text.and_then(|text| text.trim_end_matches('s').parse().ok())
			.unwrap_or_else(|| panic!("a number of seconds, not {text:?}"))
	};

	// Five minutes at the providers, forever in the inbox.
	let before = chrono::Utc::now().timestamp();
	let mut spec = spec();
	spec["options"] = json!({ "push_ttl": 300 });
	let (status, body) = send(&service, "lifetime-1", spec.clone(), &[ALICE]).await;
	assert_eq!(status, 200, "{body}");
	assert_eq!(settled(&service, "lifetime-1").await.len(), 3);
	let after = chrono::Utc::now().timestamp();

	let apns = request_where(&mock, APNS_HOST, |seen| seen.url.ends_with(&apns_token));
	let until = seconds(apns.header("apns-expiration"));
	assert!(
		(before + 299..=after + 301).contains(&until),
		"APNs holds it five minutes: {until} against {before}..{after}"
	);

	let fcm = request_where(&mock, FCM_HOST, |seen| {
		seen.json()["message"]["token"] == "fcm-lifetime-1"
	})
	.json();
	let ttl = seconds(fcm["message"]["android"]["ttl"].as_str());
	assert!(
		(240..=300).contains(&ttl),
		"FCM holds it five minutes: {fcm}"
	);
	let until = seconds(fcm["message"]["apns"]["headers"]["apns-expiration"].as_str());
	assert!((before + 299..=after + 301).contains(&until), "{fcm}");

	let web = request_where(&mock, WEB_HOST, |seen| seen.url == web_endpoint);
	let ttl = seconds(web.header("ttl"));
	assert!((240..=300).contains(&ttl), "the web holds it five minutes");

	assert_eq!(
		count(
			&service,
			"SELECT COUNT(*) FROM notifications WHERE expires_at IS NULL"
		)
		.await,
		1,
		"the inbox item has no expiry"
	);

	// With an inbox expiry too, the providers are told the earlier.
	spec["expires_at"] = json!((chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339());
	spec["options"] = json!({ "push_ttl": 60 });
	let (status, body) = send(&service, "lifetime-2", spec.clone(), &[ALICE]).await;
	assert_eq!(status, 200, "{body}");
	assert_eq!(settled(&service, "lifetime-2").await.len(), 3);
	let ttl = mock
		.requests_to(WEB_HOST)
		.iter()
		.map(|seen| seconds(seen.header("ttl")))
		.min()
		.unwrap();
	assert!((1..=60).contains(&ttl), "the shorter lifetime wins: {ttl}");

	// Zero is "now or never": each provider's own spelling of it.
	spec.as_object_mut().unwrap().remove("expires_at");
	spec["options"] = json!({ "push_ttl": 0 });
	let (status, body) = send(&service, "lifetime-3", spec.clone(), &[ALICE]).await;
	assert_eq!(status, 200, "{body}");
	assert_eq!(
		settled(&service, "lifetime-3").await,
		vec![("accepted".to_owned(), None); 3],
		"a zero lifetime is still attempted once"
	);
	assert!(
		mock.requests_to(APNS_HOST)
			.iter()
			.any(|seen| seen.header("apns-expiration") == Some("0")),
		"APNs is told not to store it"
	);
	assert!(mock.requests_to(FCM_HOST).iter().any(|seen| {
		seen.json()["message"]["android"]["ttl"] == "0s"
			&& seen.json()["message"]["apns"]["headers"]["apns-expiration"] == "0"
	}));
	assert!(
		mock.requests_to(WEB_HOST)
			.iter()
			.any(|seen| seen.header("ttl") == Some("0"))
	);

	// The lifetime has a named ceiling.
	spec["options"] = json!({ "push_ttl": 2_419_201 });
	let (status, body) = send(&service, "lifetime-bad", spec, &[ALICE]).await;
	assert_eq!(status, 400, "{body}");
	assert!(body.contains("options.push_ttl"), "{body}");
}

/// A lifetime that ran out before the attempt ends the delivery as
/// `expired` with nothing posted, while the inbox item stays.
#[tokio::test(flavor = "multi_thread")]
async fn a_lapsed_push_lifetime_ends_the_delivery_and_keeps_the_inbox_item() {
	let mock = Mock::new();
	let mut service = service(&mock, &all_transports()).await;
	let (status, body) = enroll(&service, ALICE, apns_credential(&device_token(64))).await;
	assert_eq!(status, 200, "{body}");

	let mut spec = spec();
	spec["options"] = json!({ "push_ttl": 1 });
	let (status, body) = send(&service, "lapsed", spec, &[ALICE]).await;
	assert_eq!(status, 200, "{body}");

	// The workers were busy for longer than the push was worth.
	tokio::time::sleep(Duration::from_millis(2_100)).await;
	service.resume();

	assert_eq!(
		settled(&service, "lapsed").await,
		vec![("skipped".to_owned(), Some("expired".to_owned()))]
	);
	assert!(mock.requests_to(APNS_HOST).is_empty(), "nothing was posted");
	assert_eq!(
		count(
			&service,
			"SELECT COUNT(*) FROM notifications WHERE expires_at IS NULL AND cancelled_at IS NULL"
		)
		.await,
		1,
		"the inbox still has the item"
	);
}

/// A zero lifetime is one attempt: a provider that answers "try again"
/// is not tried again.
#[tokio::test(flavor = "multi_thread")]
async fn a_zero_lifetime_is_never_retried() {
	let mock = Mock::new();
	let mut service = service(&mock, &all_transports()).await;
	mock.scripted(|request, _| {
		if request.url.contains(APNS_HOST) {
			Scripted::Reply(503, Vec::new(), String::new())
		} else {
			Scripted::Default
		}
	});
	let (status, body) = enroll(&service, ALICE, apns_credential(&device_token(63))).await;
	assert_eq!(status, 200, "{body}");
	service.resume();

	let mut spec = spec();
	spec["options"] = json!({ "push_ttl": 0 });
	let (status, body) = send(&service, "once", spec, &[ALICE]).await;
	assert_eq!(status, 200, "{body}");
	assert_eq!(
		settled_rewinding(&service, "once").await,
		vec![("rejected".to_owned(), Some("expired".to_owned()))],
		"ended after its attempt, as a push whose expiry passes between retries is"
	);
	assert_eq!(
		mock.requests_to(APNS_HOST).len(),
		1,
		"one attempt and no more"
	);
}

/// `android.raw_channel_id` names a channel exactly as the device holds
/// it — one an earlier, Firebase-era build of the app created — where
/// `android.channel_id` names one the plugin made, under its prefix. A
/// send names one or the other.
#[tokio::test(flavor = "multi_thread")]
async fn a_raw_android_channel_id_is_sent_as_given() {
	let mock = Mock::new();
	let mut service = service(&mock, &all_transports()).await;
	mock.scripted(passing_exchange());
	let (status, body) = enroll(&service, ALICE, fcm_credential("fcm-raw-channel")).await;
	assert_eq!(status, 200, "{body}");
	service.resume();

	let mut spec = spec();
	spec["options"] = json!({ "android": { "raw_channel_id": "orders" } });
	let (status, body) = send(&service, "raw-channel", spec.clone(), &[ALICE]).await;
	assert_eq!(status, 200, "{body}");
	assert_eq!(
		settled(&service, "raw-channel").await,
		vec![("accepted".to_owned(), None)]
	);
	let fcm = request_where(&mock, FCM_HOST, |seen| {
		seen.json()["message"]["token"] == "fcm-raw-channel"
	})
	.json();
	assert_eq!(
		fcm["message"]["android"]["notification"]["channel_id"], "orders",
		"no prefix: {fcm}"
	);

	spec["options"] = json!({ "android": { "raw_channel_id": "orders", "channel_id": "inbox" } });
	let (status, body) = send(&service, "raw-channel-both", spec.clone(), &[ALICE]).await;
	assert_eq!(status, 400, "{body}");
	assert!(body.contains("options.android.raw_channel_id"), "{body}");

	spec["options"] = json!({ "android": { "raw_channel_id": "x".repeat(65) } });
	let (status, body) = send(&service, "raw-channel-long", spec, &[ALICE]).await;
	assert_eq!(status, 400, "{body}");
	assert!(body.contains("options.android.raw_channel_id"), "{body}");
}
