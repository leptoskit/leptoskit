//! The native iOS enrollment's typed request and identity, against the real
//! service on a real port.
//!
//! A fake host answer — the exact bytes a real iOS app delegate path
//! replies with, serialized through the notifications plugin's own codec —
//! is decoded the way the wasm guest decodes it, mapped through
//! `native::decide` into the typed [`crate::push::PushCredential`], and
//! sent as the JSON envelope the RPC client sends. What the socket sees is
//! the production path's own: the service reads the person from the
//! forwarded identity, never from the request, dedupes a destination to one
//! endpoint, answers revocations only to the owner, and keeps one receipt
//! per idempotency key. Nothing is sent to a provider: the boundary would
//! refuse, and the tests assert it stayed silent.
//!
//! What is **not** exercised here is the guest's own control flow — the
//! enroll/reconcile/rotate decisions, their plugin calls, and the guest
//! RPC transport. Those run for real, in a compiled wasm guest against
//! played OS and HTTP plugin boundaries, in
//! `crates/leptoskit_wasm/tests/notifications_guest.rs`; the two suites
//! meet at the wire contract each side really speaks.

#![cfg(all(feature = "native", feature = "service", not(target_arch = "wasm32")))]

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use base64::Engine as _;
use leptoskit_db::Db;
use leptoskit_notifications::MIGRATIONS;
use leptoskit_notifications::SERVICE;
use leptoskit_notifications::native;
use leptoskit_notifications::native::EnrollError;
use leptoskit_notifications::push::DeviceDetails;
use leptoskit_notifications::service::config;
use leptoskit_notifications::service::push;
use leptoskit_notifications::service::routes_with_push;
use leptoskit_notifications::service::transport::Boundary;
use leptoskit_notifications::service::transport::PushRuntime;
use leptoskit_notifications::service::transport::WireError;
use leptoskit_notifications::service::transport::WireRequest;
use leptoskit_notifications::service::transport::WireResponse;
use leptoskit_rpc as rpc;
use leptoskit_rpc::server::IDEMPOTENCY_KEY;
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

/// A device token of the length Apple issues today, lower-case hex exactly
/// as the plugin's broker hands it over. It is fake material: nothing is
/// ever sent to a provider from these tests.
fn device_token(seed: u8) -> String {
	use std::fmt::Write as _;

	let mut hex = String::new();

	for byte in 0..32_u8 {
		let _ = write!(hex, "{:02x}", byte.wrapping_add(seed));
	}

	hex
}

/// The fake host's answer for `token`: the plugin codec's own serialization
/// of a revisioned token snapshot — the bytes a real iOS host replies with.
fn fake_host_token(token: &str) -> leptoskit_plugin_notifications::RemoteRegistration {
	use leptoskit_plugin_notifications::DeviceToken;
	use leptoskit_plugin_notifications::RemoteState;

	leptoskit_plugin_notifications::RemoteRegistration::new(
		1,
		RemoteState::Token(DeviceToken::new(token).expect("the fake token is in bounds")),
	)
	.expect("a revisioned snapshot")
}

/// What the boundary received, for the tests to inspect.
struct Seen {
	url: String,
}

/// The mock provider boundary: the production request path with a scripted
/// socket. These tests never push, so the script stays silent and the
/// recording only proves nothing leaked out.
struct Mock {
	seen: std::sync::Mutex<Vec<Seen>>,
}

impl Boundary for Mock {
	fn post(
		&self,
		request: WireRequest,
	) -> Pin<Box<dyn Future<Output = Result<WireResponse, WireError>> + Send + '_>> {
		self.seen.lock().unwrap().push(Seen {
			url: request.url.clone(),
		});

		Box::pin(async { Err(WireError::Unreachable) })
	}
}

impl Mock {
	fn new() -> Arc<Self> {
		Arc::new(Self {
			seen: std::sync::Mutex::new(Vec::new()),
		})
	}

	fn requests(&self) -> Vec<String> {
		self.seen
			.lock()
			.unwrap()
			.iter()
			.map(|seen| seen.url.clone())
			.collect()
	}
}

/// A generated `.p8` PEM, the way a real operator's key file looks.
fn generated_pem() -> String {
	let rng = ring::rand::SystemRandom::new();
	let document =
		EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).expect("a key");
	let begin = "-----BEGIN PRIVATE KEY-----"; // gitleaks:allow (a key made at test time)

	format!(
		"{begin}\n{}\n-----END PRIVATE KEY-----\n",
		base64::engine::general_purpose::STANDARD.encode(document.as_ref())
	)
}

struct Service {
	address: std::net::SocketAddr,
}

/// The service with its APNs transport on, over the mock boundary, and the
/// forwarded identity registered for the tests' people. Each test serves
/// its own database, so they run in parallel without sharing rows.
async fn start(mock: &Arc<Mock>) -> Service {
	let push_config = config::PushConfig {
		apns: Some(config::ApnsConfig {
			key_p8: generated_pem(),
			key_id: "KEY123".into(),
			team_id: "TEAM456".into(),
			topic: "dev.leptoskit.app".into(),
			environment: config::ApnsEnvironment::Sandbox,
		}),
		fcm: None,
		web: None,
	};
	let runtime = PushRuntime::build_over(&push_config, mock.clone()).expect("the runtime builds");
	let push = push::Push {
		paused: push::Paused::default(),
		runtime: Some(Arc::new(runtime)),
	};
	let callers = ServiceCallers::new(Anonymous)
		.caller(TOKEN, rpc::server::APP_CALLER)
		.forwarded_identity(SERVICE, TOKEN);
	let db: Db = leptoskit_db::testing::sqlite_migrated(MIGRATIONS).await;
	let (router, live) = routes_with_push(db, Some(ORIGIN), callers, push);
	live.start().await.expect("live starts");
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	tokio::spawn(async move {
		leptoskit_axum::axum::serve(listener, router).await.unwrap();
	});

	Service { address }
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

/// The POST the wasm guest's own client sends: the operation's JSON
/// envelope, with the person forwarded as the bearer identity.
async fn post(
	service: &Service,
	operation: &str,
	args: Value,
	token: Option<&str>,
	idempotency_key: Option<&str>,
) -> (u16, Value) {
	let mut request = reqwest::Client::new()
		.post(format!(
			"http://{}{PREFIX}/rpc/{operation}",
			service.address
		))
		.json(&args);

	if let Some(token) = token {
		request = request.bearer_auth(token);
	}

	if let Some(key) = idempotency_key {
		request = request.header(IDEMPOTENCY_KEY, key);
	}

	let response = request.send().await.unwrap();
	let status = response.status().as_u16();
	(status, response.json().await.unwrap_or(Value::Null))
}

/// The guest's own enrollment half, replayed on the wire: decode the fake
/// host answer the way the guest does, decide, serialize, POST.
async fn enroll_as(
	service: &Service,
	person: Uuid,
	snapshot: &leptoskit_plugin_notifications::RemoteRegistration,
) -> (u16, Value) {
	let credential = native::decide(snapshot)
		.expect("the snapshot decides")
		.expect("the fake host holds a token");
	let request =
		serde_json::to_value(native::request(credential, DeviceDetails::default())).unwrap();
	post(
		service,
		"register_push_endpoint",
		request,
		Some(&signed_in(person)),
		None,
	)
	.await
}

async fn endpoints_of(service: &Service, person: Uuid) -> Vec<Value> {
	let (status, body) = post(
		service,
		"push_endpoints",
		json!({ "page": { "limit": 100 } }),
		Some(&signed_in(person)),
		None,
	)
	.await;
	assert_eq!(status, 200, "{body}");
	body["items"].as_array().cloned().unwrap_or_default()
}

/// The typed request enrolls the fake host's token under the forwarded
/// identity alone: the endpoint is Alice's, the credential is never echoed
/// back, a refresh re-registers the same destination to the same id, and
/// Bob's identity sees nothing of it.
#[tokio::test(flavor = "multi_thread")]
async fn the_typed_request_enrolls_under_the_forwarded_identity() {
	let mock = Mock::new();
	let service = start(&mock).await;
	let token = device_token(0);

	let (status, enrolled) = enroll_as(&service, ALICE, &fake_host_token(&token)).await;
	assert_eq!(status, 200, "{enrolled}");
	let endpoint = enrolled["id"].as_str().unwrap().parse::<Uuid>().unwrap();
	assert_eq!(enrolled["platform"], "apns");
	assert!(
		!enrolled.to_string().contains(&token),
		"the credential is never echoed back: {enrolled}"
	);

	// The endpoint belongs to the person the app forwarded, and nobody else.
	let mine = endpoints_of(&service, ALICE).await;
	assert_eq!(mine.len(), 1);
	assert_eq!(mine[0]["id"], enrolled["id"]);
	assert!(
		endpoints_of(&service, BOB).await.is_empty(),
		"Bob's list is his own"
	);

	// A refresh — the same token again — re-registers one destination.
	let (status, again) = enroll_as(&service, ALICE, &fake_host_token(&token)).await;
	assert_eq!(status, 200, "{again}");
	assert_eq!(again["id"], enrolled["id"], "the destination keeps its id");
	assert_eq!(endpoints_of(&service, ALICE).await.len(), 1);

	// Only the owner can act on it: Bob's revoke answers as if it were
	// missing.
	let (status, body) = post(
		&service,
		"revoke_push_endpoint",
		json!({ "endpoint": endpoint }),
		Some(&signed_in(BOB)),
		None,
	)
	.await;
	assert_eq!(status, 404, "{body}");
	assert_eq!(endpoints_of(&service, ALICE).await.len(), 1);

	assert!(
		mock.requests().is_empty(),
		"enrollment never pushes: {:?}",
		mock.requests()
	);
}

/// A rotated token is a new destination: it becomes its own endpoint while
/// the old one lives, and the explicit old-handle cleanup — a revoke with
/// the module's stable idempotency key, while still authenticated — brings
/// the person back under the endpoint bound. Churn is bounded, not
/// accumulated.
#[tokio::test(flavor = "multi_thread")]
async fn a_rotated_token_needs_explicit_cleanup_and_the_key_replays() {
	let mock = Mock::new();
	let service = start(&mock).await;

	let (status, first) = enroll_as(&service, ALICE, &fake_host_token(&device_token(0))).await;
	assert_eq!(status, 200, "{first}");
	let old = first["id"].as_str().unwrap().parse::<Uuid>().unwrap();

	let (status, rotated) = enroll_as(&service, ALICE, &fake_host_token(&device_token(1))).await;
	assert_eq!(status, 200, "{rotated}");
	assert_ne!(
		rotated["id"], first["id"],
		"a new destination is a new endpoint"
	);
	assert_eq!(endpoints_of(&service, ALICE).await.len(), 2);

	// The app's sign-out calls `disable` while authenticated: one revoke,
	// acknowledged, with the module's key.
	let key = native::disable_key(old);
	let (status, body) = post(
		&service,
		"revoke_push_endpoint",
		json!({ "endpoint": old }),
		Some(&signed_in(ALICE)),
		Some(&key),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	assert_eq!(endpoints_of(&service, ALICE).await.len(), 1);

	// A replay of the same key answers the same acknowledged revoke — the
	// receipt is Alice's, so a retried sign-out is safe — while a stranger
	// with the same key gets nothing.
	let (status, _) = post(
		&service,
		"revoke_push_endpoint",
		json!({ "endpoint": old }),
		Some(&signed_in(ALICE)),
		Some(&key),
	)
	.await;
	assert_eq!(status, 200, "the receipt replays for its owner");
	let (status, _) = post(
		&service,
		"revoke_push_endpoint",
		json!({ "endpoint": old }),
		Some(&signed_in(BOB)),
		Some(&key),
	)
	.await;
	assert_eq!(status, 404, "Bob cannot reuse Alice's receipt");
	assert_eq!(endpoints_of(&service, ALICE).await.len(), 1);
}

/// Without the app's forwarded identity there is no enrollment: an
/// anonymous request is refused before anything is stored, and a refused
/// snapshot — the OS's own provisioning answer — decides nothing.
#[tokio::test(flavor = "multi_thread")]
async fn anonymous_calls_enroll_nothing_and_refusals_decide_nothing() {
	let mock = Mock::new();
	let service = start(&mock).await;

	let credential = native::decide(&fake_host_token(&device_token(2)))
		.unwrap()
		.unwrap();
	let request =
		serde_json::to_value(native::request(credential, DeviceDetails::default())).unwrap();

	let (status, _) = post(&service, "register_push_endpoint", request, None, None).await;
	assert_eq!(status, 403, "no identity, no enrollment");
	assert!(
		endpoints_of(&service, ALICE).await.is_empty(),
		"the refused enrollment stored nothing"
	);

	// A snapshot whose answer is the OS's refusal never reaches the wire,
	// and silence offers nothing to enroll. The refusal is the contract's
	// stable code, never the OS's own text.
	let refused = leptoskit_plugin_notifications::RemoteRegistration::new(
		1,
		leptoskit_plugin_notifications::RemoteState::Failed(
			leptoskit_plugin_notifications::remote_failure::APS_ENVIRONMENT.into(),
		),
	)
	.unwrap();
	assert_eq!(
		native::decide(&refused),
		Err(EnrollError::Failed(
			leptoskit_plugin_notifications::remote_failure::APS_ENVIRONMENT.into()
		))
	);
	assert_eq!(
		native::decide(&leptoskit_plugin_notifications::RemoteRegistration::none()),
		Ok(None)
	);
}
