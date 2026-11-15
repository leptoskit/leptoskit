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
use rpc::server::Anonymous;
use rpc::server::ServiceCallers;
use rsa::pkcs8::EncodePrivateKey;
use rsa::sha2::Sha256;
use rsa::signature::Verifier;
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
	instance: Arc<str>,
	jobs: Option<rpc::server::JobsHandle>,
}

/// A fresh, migrated service on a free port, its push transports built
/// over `mock` and carried by its routes, its workers not yet running:
/// tests start them when the queue should move. The instance key is
/// unique per service, except where a test restarts one over the same
/// database and passes the same key.
async fn start(db: Db, config: &config::PushConfig, mock: &Arc<Mock>) -> Service {
	start_keyed(db, config, mock, &Uuid::now_v7().to_string()).await
}

/// [`start`], with an explicit instance key: the restart test's stable
/// one, or `None` for a service with no push configuration at all.
async fn start_keyed(
	db: Db,
	config: &config::PushConfig,
	mock: &Arc<Mock>,
	instance: &str,
) -> Service {
	let runtime = if config.apns.is_some() || config.fcm.is_some() || config.web.is_some() {
		Some(PushRuntime::build_over(config, mock.clone()).expect("the runtime builds"))
	} else {
		None
	};

	start_push(db, runtime, instance).await
}

/// [`start_keyed`], over an already-built (or absent) runtime.
async fn start_push(db: Db, runtime: Option<PushRuntime>, instance: &str) -> Service {
	let push = push::Push {
		instance: instance.to_owned().into(),
		runtime: runtime.map(Arc::new),
	};

	let callers = ServiceCallers::new(Anonymous)
		.caller(TOKEN, rpc::server::APP_CALLER)
		.forwarded_identity(SERVICE, TOKEN);
	let (router, live) =
		leptoskit_notifications::service::routes_with_push(db.clone(), Some(ORIGIN), callers, push);
	live.start().await.expect("live starts");
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	tokio::spawn(async move {
		leptoskit_axum::axum::serve(listener, router).await.unwrap();
	});

	Service {
		address,
		db,
		instance: instance.to_owned().into(),
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

/// Runs one push round directly, outside any worker, as the race tests
/// do: the deliveries' own writes decide everything. Owned, so it can be
/// spawned beside whatever it races.
fn spawn_round(service: &Service, send_key: &str, round: u32) -> tokio::task::JoinHandle<()> {
	let db = service.db.clone();
	let instance = Arc::clone(&service.instance);
	let send_key = send_key.to_owned();

	tokio::spawn(async move {
		let cx = rpc::JobCx::new(
			db,
			rpc::server::Identity::anonymous(),
			format!("test:{send_key}:{round}"),
			1,
		);
		push::round(&cx, &send_key, round, &instance)
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

/// A generated RSA private key in the PKCS#8 PEM form Google's service
/// account JSON carries.
fn generated_rsa() -> (String, rsa::PrivateKey) {
	// The rsa crate's key generation wants a `TryCryptoRng`; ring's
	// system randomness is one, behind a small infallible adapter.
	struct RingRng(SystemRandom);

	impl rsa::rand_core::TryRng for RingRng {
		type Error = core::convert::Infallible;

		fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
			let mut bytes = [0_u8; 4];
			self.0.fill(&mut bytes).expect("system randomness");
			Ok(u32::from_be_bytes(bytes))
		}

		fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
			let mut bytes = [0_u8; 8];
			self.0.fill(&mut bytes).expect("system randomness");
			Ok(u64::from_be_bytes(bytes))
		}

		fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), Self::Error> {
			self.0.fill(dest).expect("system randomness");
			Ok(())
		}
	}

	impl rsa::rand_core::TryCryptoRng for RingRng {}

	let mut rng = RingRng(SystemRandom::new());
	let private = rsa::PrivateKey::new(&mut rng, 2048).expect("an RSA key");
	let pem = private
		.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF)
		.expect("PKCS#8 PEM")
		.to_string();
	(pem, private)
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

/// Verifies an RS256 JWT against `private`'s public half — an
/// independent check, the rsa crate's own verifier against the
/// jsonwebtoken crate's signer — answering its header and claims.
fn verify_rs256(token: &str, private: &rsa::PrivateKey) -> (Value, Value) {
	let parts: Vec<&str> = token.split('.').collect();
	assert_eq!(parts.len(), 3, "a JWT has three parts: {token}");
	let signing = format!("{}.{}", parts[0], parts[1]);
	let signature = URL_SAFE_NO_PAD
		.decode(parts[2])
		.expect("the signature is base64url");

	let public = rsa::PublicKey::from(private);
	let key = rsa::pkcs1v15::VerifyingKey::<Sha256>::new(public);
	let signature = rsa::pkcs1v15::Signature::try_from(signature.as_slice()).expect("a signature");
	key.verify(signing.as_bytes(), &signature)
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
	let simple = endpoint.replace('-', "");
	let stored = rows(
		service,
		&format!("SELECT credential FROM push_endpoints WHERE lower(hex(id)) = '{simple}'"),
	)
	.await;

	assert!(!stored.is_empty(), "no endpoint row for {endpoint}");
	text(&stored[0][0])
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
	let (rsa_pem, _) = generated_rsa();
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
	let unconfigured = start_push(db().await, None, &Uuid::now_v7().to_string()).await;

	let (status, body) = enroll(&unconfigured, ALICE, apns_credential(&device_token(1))).await;
	assert_eq!(status, 400, "with push disabled, nothing enrolls: {body}");
	assert!(body.contains("not configured"), "{body}");
	assert!(
		bare.requests().is_empty(),
		"nothing reached any provider through any instance"
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_destination_of_another_live_account_is_refused_not_reassigned() {
	let mock = Mock::new();
	let service = service(&mock, &all_transports()).await;

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

	// The same holds where only the keys differ: bob can't take alice's
	// Web Push URL by presenting his own keys for it.
	let endpoint = format!("https://{WEB_HOST}/push/abcdef123456");
	let (status, _) = enroll(&service, ALICE, web_credential(&endpoint)).await;
	assert_eq!(status, 200);

	let (status, body) = enroll(&service, BOB, web_credential(&endpoint)).await;
	assert_eq!(status, 409, "a live URL is its owner's, keys or no: {body}");
	assert!(
		!body.contains(WEB_HOST) && !body.contains("abcdef"),
		"the refusal carries no endpoint: {body}"
	);

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
async fn an_invalidated_destination_can_re_enroll_and_be_taken_over() {
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

	// An invalidated endpoint another account re-enrolls is taken over
	// clean: the previous holder's hold ended with the invalidation.
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
		status, 200,
		"an invalidated destination is nobody's: {body}"
	);
	let taken: Value = serde_json::from_str(&body).unwrap();
	assert_eq!(taken["id"], current["id"], "the row is taken over in place");

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

	// History is bounded: however many invalidations come and go, the
	// person's rows never grow past the live bound plus the history
	// bound. Each wave has the provider invalidate one live endpoint —
	// growing the history and freeing a live slot — and enrolls a fresh
	// victim in the room.
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

	let count = rows(&service, "SELECT count(*) FROM push_endpoints").await;
	let total: usize = text(&count[0][0]).parse().unwrap();
	assert!(
		total
			<= leptoskit_notifications::push::MAX_ENDPOINTS
				+ leptoskit_notifications::push::MAX_ENDPOINT_HISTORY,
		"{total} rows for one person is past the bounds"
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
	let (rsa_pem, rsa_private) = generated_rsa();
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
	// verifies each assertion independently — RS256, the rsa crate's
	// own verifier — and checks the exact claims the grant needs. The
	// account's JSON names a token_uri of its own; the exchange must go
	// to the fixed first-party endpoint regardless.
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
		let (header, claims) = verify_rs256(assertion, &rsa_private);
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
	let (rsa_pem, _) = generated_rsa();
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
				"google.sneaky": "no",
				"gcm.sneaky": "no",
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
			&& message["message"]["data"].get("google.sneaky").is_none()
			&& message["message"]["data"].get("gcm.sneaky").is_none(),
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
	let (rsa_pem, _) = generated_rsa();
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
	let (rsa_pem, _) = generated_rsa();
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
	let outcomes = settled(&service, "stale").await;
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
		"a fresh 401 stalls without spending attempts: {items:?}"
	);
	assert_eq!(items[0]["attempts"], 0, "{items:?}");
	assert_eq!(
		fresh_refusals.load(Ordering::SeqCst),
		2,
		"the send and its one refreshed retry: bounded"
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
	let (rsa_pem, _) = generated_rsa();
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
			json!({ "error": { "code": 404, "status": "UNREGISTERED" } }).to_string(),
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
	let (_, claims) = verify_jwt(token, &vapids);
	assert_eq!(claims["aud"], format!("https://{WEB_HOST}"));
	assert_eq!(claims["sub"], "mailto:push@leptoskit.dev");
	assert!(claims["exp"].as_i64().unwrap() > leptoskit_db::now().timestamp());
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
	let (one_pem, one_private) = generated_rsa();
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
	let (two_pem, two_private) = generated_rsa();
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

	// Service one sends first; then service two, while one's token is
	// still cached.
	let mut one = one;
	one.resume();
	let (status, _) = send(&one, "mine", spec(), &[ALICE]).await;
	assert_eq!(status, 200);
	settled(&one, "mine").await;

	two.resume();
	let (status, _) = send(&two, "theirs", spec(), &[BOB]).await;
	assert_eq!(status, 200);
	settled(&two, "theirs").await;

	// Each service's assertion is its own account's, and each send
	// carries its own access token.
	let one_assertion =
		String::from_utf8(one_mock.requests_to(FCM_TOKEN_HOST)[0].body.clone()).unwrap();
	let assertion = one_assertion.split("assertion=").nth(1).unwrap().to_owned();
	let (_, claims) = verify_rs256(&assertion, &one_private);
	assert_eq!(
		claims["iss"], "push@leptoskit-push.iam.gserviceaccount.com",
		"service one signs as itself"
	);

	let two_assertion =
		String::from_utf8(two_mock.requests_to(FCM_TOKEN_HOST)[0].body.clone()).unwrap();
	let assertion = two_assertion.split("assertion=").nth(1).unwrap().to_owned();
	let (_, claims) = verify_rs256(&assertion, &two_private);
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

	let outcomes = settled(&service, "unplugged").await;
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

	let outcomes = settled(&service, "doomed").await;
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
	service.resume();

	// Two sends, so the unregistered endpoint holds two deliveries. The
	// script is in place before they fire, so no attempt can slip past
	// it while the workers are already polling.
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

	// A topic mismatch is the operator's configuration: the delivery
	// ends, the endpoint stays.
	mock.scripted(|request, _| {
		if request.url.contains(APNS_HOST) {
			return Scripted::Reply(
				403,
				Vec::new(),
				json!({ "reason": "InvalidProviderToken" }).to_string(),
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

	// A send too big even for the minimal envelope — at FCM the visible
	// notification rides twice, in `notification` and again inside the
	// `apns` payload, so a full-size body pushes the encoded message past
	// 4 KiB whatever is dropped. It ends oversized, with no request at
	// all — and the fallback keeps its collapse ids, so a repeat that
	// fits coalesces with the full payload would have.
	let mock2 = Mock::new();
	let (rsa_pem, _) = generated_rsa();
	let mut service2 = start(
		db().await,
		&config::PushConfig {
			apns: None,
			fcm: Some(fcm_config_for(&rsa_pem)),
			web: None,
		},
		&mock2,
	)
	.await;
	let (status, _) = enroll(&service2, ALICE, fcm_credential("fcm-token-2")).await;
	assert_eq!(status, 200);
	// The exchange would answer normally; the delivery never gets there.
	mock2.scripted(|request, _| {
		if request.url.contains(FCM_TOKEN_HOST) {
			return Scripted::Reply(
				200,
				vec![("content-type".to_owned(), "application/json".to_owned())],
				json!({ "access_token": "ya29.too-big", "expires_in": 3600 }).to_string(),
			);
		}

		Scripted::Default
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

	// The minimal envelope, when it does fit, keeps the collapse key and
	// the apns headers: a fallback that dropped them would let a repeat
	// stack banners the full payload would have replaced.
	let mock3 = Mock::new();
	let (rsa_pem3, _) = generated_rsa();
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

#[tokio::test(flavor = "multi_thread")]
async fn a_restart_recovers_pending_work() {
	let file = tempfile::Builder::new()
		.prefix("notifications-push-restart-")
		.suffix(".sqlite3")
		.tempfile()
		.expect("a database file");
	let url = format!("sqlite://{}", file.path().display());
	let mock = Arc::new(Mock::new());

	// The instance key is stable across the restart, like a service's
	// database URL: the queued job names it, and the new process's
	// registry answers to it.
	let instance = format!("restart:{url}");

	// The first process: enroll, send, and stop before any attempt runs.
	let db = Db::connect(&url).await.expect("the database opens");
	db.migrate(MIGRATIONS).await.expect("migrations apply");
	{
		let service = start_keyed(db, &all_transports(), &mock, &instance).await;
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
	let mut restarted = start_keyed(db, &all_transports(), &mock, &instance).await;
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
