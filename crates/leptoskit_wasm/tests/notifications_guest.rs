//! The notifications native enrollment in a compiled guest: the exact
//! production control flow — `native::enroll`, `reconcile`, and `disable`,
//! through their real bridge guest transport and their real RPC client
//! over the real HTTP plugin guest.
//!
//! What is real and what is played:
//!
//! - **Real**: the guest is the shipped `native` module code, its plugin guest
//!   client, the RPC client's guest transport, the idempotency-key and
//!   credential headers, and both wire codecs end to end.
//! - **Played**: the OS boundary — the `notifications` plugin's methods 7 and 8
//!   answer scripted APNs snapshots through the plugin's own codec — and the
//!   service — the `http` plugin answers scripted JSON for the RPC routes the
//!   client really addresses, with a destination-deduping, quota-refusing,
//!   receipt-replaying stand-in. No real APNs, no real server, no device.
//!
//! Runs as `test:plugins:guest` does, from the wasm the script builds.

use std::collections::BTreeMap;
use std::time::Duration;
use std::time::Instant;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::PluginId;
use leptoskit_native::EventOutcome;
use leptoskit_native::HeadlessBackend;
use leptoskit_plugin_http::protocol;
use leptoskit_plugin_notifications::ApnsRegistration;
use leptoskit_plugin_notifications::DeviceToken;
use leptoskit_plugin_notifications::RemoteState;
use leptoskit_wasm::EventKind;
use leptoskit_wasm::WasmHost;
use uuid::Uuid;

/// A device token of the length Apple issues today. Fake material: nothing
/// here ever reaches a provider.
fn token(seed: u8) -> String {
	use std::fmt::Write as _;

	let mut hex = String::new();
	for byte in 0..32_u8 {
		let _ = write!(hex, "{:02x}", byte.wrapping_add(seed));
	}
	hex
}

/// What the fake OS answers for one `REMOTE_REGISTER`, in order.
enum Answer {
	Token(String),
	Unconfirmed,
}

/// The fake service: what the RPC routes answer, and what crossed them.
struct Service {
	/// Destination dedupe: a token always answers the same endpoint id.
	destinations: BTreeMap<String, Uuid>,
	/// The next endpoint id the service mints, deterministic per run.
	minted: u128,
	/// The endpoints the listing says this person owns.
	owned: Vec<Uuid>,
	/// Per-register-call script: a failure, or quota, or success.
	register: Vec<Refusal>,
	/// Whether revoke calls answer a server error.
	revoke_fails: bool,
	/// Idempotency keys already receipted, and what they revoked.
	receipts: BTreeMap<String, Uuid>,
	/// One line per RPC: `operation [detail…] auth=… key=…`.
	log: Vec<String>,
}

#[derive(Clone, Copy)]
enum Refusal {
	Success,
	Network,
	Quota,
}

impl Service {
	fn new() -> Self {
		Self {
			destinations: BTreeMap::new(),
			minted: 0,
			owned: Vec::new(),
			register: Vec::new(),
			revoke_fails: false,
			receipts: BTreeMap::new(),
			log: Vec::new(),
		}
	}

	/// One `register_push_endpoint` call.
	fn register(&mut self, body: &str) -> (u16, String) {
		let destination = body_json(body)["credential"]["apns"]["device_token"]
			.as_str()
			.expect("the credential the guest sends")
			.to_owned();
		match self.register.first().copied().unwrap_or(Refusal::Success) {
			Refusal::Network => {
				self.register.remove(0);
				self.log
					.push(format!("register({destination}, network) auth=…"));
				return (0, String::new()); // signalled as a transport failure
			}
			Refusal::Quota => {
				self.register.remove(0);
				self.log
					.push(format!("register({destination}, quota) auth=…"));
				return (
					400,
					"Args|a person holds at most 10 live endpoints; revoke one first".into(),
				);
			}
			Refusal::Success => {
				if !self.register.is_empty() {
					self.register.remove(0);
				}
			}
		}

		let id = *self
			.destinations
			.entry(destination.clone())
			.or_insert_with(|| {
				self.minted += 1;
				Uuid::from_u128(0x0E5A_0000_0000_0000_0000_0000_0000_0000 | self.minted)
			});
		self.log
			.push(format!("register({destination}) = {id} auth=…"));
		(
			200,
			format!(
				r#"{{"id":"{id}","platform":"apns","created_at":"2026-11-16T00:00:00Z","invalidated_at":null}}"#
			),
		)
	}

	/// One `push_endpoints` call: the endpoints the person owns.
	fn list(&mut self) -> (u16, String) {
		self.log.push("list() auth=…".into());
		let items: Vec<String> = self
			.owned
			.iter()
			.map(|id| {
				format!(
					r#"{{"id":"{id}","platform":"apns","created_at":"2026-11-16T00:00:00Z","invalidated_at":null}}"#
				)
			})
			.collect();
		(
			200,
			format!(
				r#"{{"items":[{}],"end":null,"has_more":false}}"#,
				items.join(",")
			),
		)
	}

	/// One `revoke_push_endpoint` call, with the receipt replay the real
	/// service gives a repeated idempotency key.
	fn revoke(&mut self, body: &str, key: Option<&str>) -> (u16, String) {
		let endpoint: Uuid = body_json(body)["endpoint"]
			.as_str()
			.expect("the endpoint the guest revokes")
			.parse()
			.expect("a uuid");
		let key = key.map(str::to_owned);
		self.log.push(format!(
			"revoke({endpoint}) key={} auth=…",
			key.clone().unwrap_or_default()
		));

		if let Some(seen) = key.as_ref()
			&& self.receipts.contains_key(seen)
		{
			return (200, String::new());
		}

		if !self.owned.contains(&endpoint) {
			return (404, "ServerError|not yours".into());
		}
		if self.revoke_fails {
			return (500, "ServerError|the revoke did not commit".into());
		}

		self.owned.retain(|owned| *owned != endpoint);
		if let Some(key) = key {
			self.receipts.insert(key, endpoint);
		}
		(200, String::new())
	}
}

/// The body's JSON, or null when the body is empty.
fn body_json(body: &str) -> serde_json::Value {
	serde_json::from_str(body).unwrap_or(serde_json::Value::Null)
}

/// The steps the guest reported, and what the fake service saw.
struct Run {
	steps: Vec<String>,
	service: Service,
}

/// Runs the guest once for `scenario`, playing the OS and service scripts.
fn run(wasm: &[u8], scenario: u32, os: Vec<Answer>, mut service: Service) -> Run {
	const REPORT: &str = "notifications-guest-report";
	const ASK: MethodId = MethodId(0);
	const PANICKED: MethodId = MethodId(1);
	const STEP: MethodId = MethodId(2);
	const REMOTE_TOKEN: MethodId = MethodId(7);
	const REMOTE_REGISTER: MethodId = MethodId(8);

	let mut host = WasmHost::<HeadlessBackend>::headless(wasm).expect("compile the guest");
	let report = register(&mut host, REPORT, &[ASK, PANICKED, STEP]);
	let notifications = register(&mut host, "notifications", &[REMOTE_TOKEN, REMOTE_REGISTER]);
	let http = register(&mut host, "http", &[MethodId(0)]);
	host.start(100.0, 100.0).expect("start the guest");

	// The OS's scripted answers, and the last snapshot it handed out.
	let mut answers = os.into_iter().map(|answer| {
		match answer {
			Answer::Token(hex) => {
				ApnsRegistration::new(
					1,
					RemoteState::Token(DeviceToken::new(hex).expect("a fake token in bounds")),
				)
				.expect("a snapshot")
				.encode()
				.expect("the plugin codec")
			}
			Answer::Unconfirmed => ApnsRegistration::unconfirmed().encode().expect("the codec"),
		}
	});
	let mut latest = ApnsRegistration::none().encode().expect("the codec");

	let mut steps = Vec::new();
	let deadline = Instant::now() + Duration::from_secs(10);
	loop {
		while let Some(request) = host.plugins().next_request() {
			let reply = if request.plugin == report {
				match request.method {
					ASK => Ok(Bytes::copy_from_slice(&scenario.to_le_bytes())),
					STEP => {
						steps.push(String::from_utf8_lossy(&request.payload).into_owned());
						Ok(Bytes::new())
					}
					PANICKED => {
						panic!(
							"the guest panicked: {}",
							String::from_utf8_lossy(&request.payload)
						)
					}
					m => panic!("unknown report method {}", m.0),
				}
			} else if request.plugin == notifications {
				match request.method {
					REMOTE_TOKEN => Ok(latest.clone()),
					REMOTE_REGISTER => {
						let answer = answers
							.next()
							.expect("the scenario scripts every registration");
						latest = answer.clone();
						Ok(answer)
					}
					m => panic!("unknown notifications method {}", m.0),
				}
			} else if request.plugin == http {
				http_reply(&mut service, &request.payload)
			} else {
				Err(Error::Unavailable)
			};
			host.plugins()
				.complete(request.id, reply)
				.expect("complete");
		}
		host.wake_plugins().expect("wake the guest");
		let done = host
			.dispatch_wire(0, EventKind::Press, None)
			.expect("poll the guest")
			== EventOutcome::Handled;
		if done && host.plugins().next_request().is_none() {
			// The guest's exit poll can submit its last step after the
			// handled event: nothing more is queued only once every step
			// the scenario owes has been read.
			return Run { steps, service };
		}
		assert!(Instant::now() < deadline, "the guest didn't finish");
		std::thread::sleep(Duration::from_millis(2));
	}
}

/// The report plugin's own replies: the scenario in, the steps and panics
/// out — folded into the loop above by closure-free dispatch below.
fn register(host: &mut WasmHost<HeadlessBackend>, name: &str, methods: &[MethodId]) -> PluginId {
	let plugin = host
		.plugins()
		.register(name, methods.iter().copied())
		.expect("register");
	for method in methods {
		host.plugins().grant(plugin, *method, true).expect("grant");
	}
	plugin
}

/// One HTTP-plugin call: decode the frame, assert the signed transport,
/// answer from the fake service's script.
fn http_reply(service: &mut Service, payload: &[u8]) -> Result<Bytes, Error> {
	let request = protocol::decode_request(payload).expect("an HTTP request");
	let operation = request.url.rsplit('/').next().unwrap_or_default();
	let authorization = request
		.headers
		.iter()
		.find(|header| header.name.eq_ignore_ascii_case("authorization"))
		.map(|header| String::from_utf8_lossy(&header.value).into_owned());
	let key = request
		.headers
		.iter()
		.find(|header| header.name.eq_ignore_ascii_case("idempotency-key"))
		.map(|header| String::from_utf8_lossy(&header.value).into_owned());

	assert_eq!(
		authorization.as_deref(),
		Some("Bearer probe-token"),
		"every call the guest makes is signed by the configured credentials"
	);
	let body = String::from_utf8_lossy(&request.body).into_owned();

	let (status, answer) = match operation {
		"register_push_endpoint" => {
			let (status, body) = service.register(&body);
			let _ = key;
			(status, body)
		}
		"push_endpoints" => service.list(),
		"revoke_push_endpoint" => service.revoke(&body, key.as_deref()),
		other => panic!("the guest addressed an unexpected operation {other}"),
	};

	if status == 0 {
		// A refused transport, the way a dropped connection reaches the RPC
		// client.
		return Err(Error::Native);
	}

	protocol::encode_response(&leptoskit_plugin_http::Response {
		status,
		url: request.url,
		headers: Vec::new(),
		body: answer.into_bytes(),
	})
}

/// The compiled guest, from the path `test:plugins:guest` builds.
fn guest() -> Vec<u8> {
	let path = std::env::var("LEPTOSKIT_NOTIFICATIONS_GUEST").expect("fixture wasm path");
	std::fs::read(path).expect("the built guest")
}

#[test]
#[ignore = "run by test:plugins:guest, which compiles the wasm fixture"]
fn a_gesture_enrolls_and_a_cold_relaunch_keeps_the_same_endpoint() {
	let first = token(0);
	let service = Service::new();
	let run = run(
		&guest(),
		0,
		vec![Answer::Token(first.clone()), Answer::Token(first.clone())],
		service,
	);
	let id = run.service.destinations[&first];

	assert_eq!(
		run.steps,
		vec![format!("enroll=ok:{id}"), format!("reconcile=ok:kept:{id}"),]
	);
	assert_eq!(
		run.service
			.log
			.iter()
			.filter(|line| line.starts_with("register("))
			.count(),
		2,
		"a same-token relaunch re-registers the destination idempotently"
	);
	assert!(
		!run.service
			.log
			.iter()
			.any(|line| line.starts_with("revoke(")),
		"nothing needs retiring when the destination is unchanged"
	);
}

#[test]
#[ignore = "run by test:plugins:guest, which compiles the wasm fixture"]
fn a_rotated_token_enrolls_and_retires_the_previous_handle() {
	let previous = Uuid::from_u128(0x1111_1111_1111_7111_8111_1111_1111_1111);
	let rotated = token(1);
	let mut service = Service::new();
	service.owned = vec![previous];
	let run = run(&guest(), 1, vec![Answer::Token(rotated.clone())], service);
	let current = run.service.destinations[&rotated];

	assert_eq!(
		run.steps,
		vec![format!("reconcile=ok:rotated:{current}:revoked:{previous}")]
	);
	assert!(
		run.service.log.iter().any(|line| {
			line == &format!("revoke({previous}) key=notifications:disable:{previous} auth=…")
		}),
		"the previous handle is retired under its owner with the stable key: {:?}",
		run.service.log
	);
	assert!(!run.service.owned.contains(&previous));
}

#[test]
#[ignore = "run by test:plugins:guest, which compiles the wasm fixture"]
fn rotation_at_the_endpoint_quota_makes_room_and_completes() {
	let previous = Uuid::from_u128(0x2222_2222_2222_7222_8222_2222_2222_2222);
	let rotated = token(2);
	let mut service = Service::new();
	service.owned = vec![previous];
	service.register = vec![Refusal::Quota];
	let run = run(&guest(), 2, vec![Answer::Token(rotated.clone())], service);
	let current = run.service.destinations[&rotated];

	assert_eq!(
		run.steps,
		vec![format!("reconcile=ok:rotated:{current}:revoked:{previous}")],
		"retiring the known-stale handle makes the room the quota demanded"
	);
	assert_eq!(
		run.service
			.log
			.iter()
			.filter(|line| line.starts_with("register("))
			.count(),
		2,
		"the quota refusal is followed by exactly one re-registration"
	);
}

#[test]
#[ignore = "run by test:plugins:guest, which compiles the wasm fixture"]
fn a_revoke_that_fails_names_both_handles_for_retry() {
	let previous = Uuid::from_u128(0x3333_3333_3333_7333_8333_3333_3333_3333);
	let rotated = token(3);
	let mut service = Service::new();
	service.owned = vec![previous];
	service.revoke_fails = true;
	let run = run(&guest(), 3, vec![Answer::Token(rotated.clone())], service);
	let current = run.service.destinations[&rotated];

	assert_eq!(
		run.steps,
		vec![format!("reconcile=err:retire:{current}:{previous}:server")]
	);
	assert!(
		run.service.owned.contains(&previous),
		"the unacknowledged handle still stands server-side"
	);
}

#[test]
#[ignore = "run by test:plugins:guest, which compiles the wasm fixture"]
fn failed_enrollments_keep_the_retry_state() {
	let previous = Uuid::from_u128(0x4444_4444_4444_7444_8444_4444_4444_4444);
	let rotated = token(4);
	let mut service = Service::new();
	service.owned = vec![previous];
	// A plain failure keeps the previous handle; the quota detour retires
	// it, so the second failure retries from the opt-in alone.
	service.register = vec![Refusal::Network, Refusal::Quota, Refusal::Network];
	let run = run(
		&guest(),
		4,
		vec![
			Answer::Token(rotated.clone()),
			Answer::Token(rotated.clone()),
		],
		service,
	);

	assert_eq!(
		run.steps,
		vec![
			format!("reconcile=err:enroll:{previous}:network"),
			"reconcile=err:enroll:none:network".to_owned(),
		]
	);
	assert!(
		!run.service.owned.contains(&previous),
		"the quota detour did retire the handle before the failed retry"
	);
}

#[test]
#[ignore = "run by test:plugins:guest, which compiles the wasm fixture"]
fn a_stale_previous_handle_is_dropped_not_operated() {
	let previous = Uuid::from_u128(0x5555_5555_5555_7555_8555_5555_5555_5555);
	let rotated = token(5);
	let mut service = Service::new();
	service.owned = Vec::new();
	let run = run(&guest(), 5, vec![Answer::Token(rotated.clone())], service);
	let current = run.service.destinations[&rotated];

	assert_eq!(
		run.steps,
		vec![format!("reconcile=ok:rotated:{current}:stale:{previous}")]
	);
	assert!(
		!run.service
			.log
			.iter()
			.any(|line| line.starts_with("revoke(")),
		"a handle another account owns is never operated under this identity"
	);
}

#[test]
#[ignore = "run by test:plugins:guest, which compiles the wasm fixture"]
fn an_unconfirmed_registration_leaves_everything_untouched() {
	let run = run(&guest(), 6, vec![Answer::Unconfirmed], Service::new());

	assert_eq!(
		run.steps,
		vec![
			"reconcile=err:unconfirmed".to_owned(),
			"reconcile=ok:off".to_owned()
		],
		"a silent OS answers unconfirmed, and a device with no previous enrollment registers \
		 nothing"
	);
	assert!(
		run.service.log.is_empty(),
		"nothing reached the service: {:?}",
		run.service.log
	);
}

#[test]
#[ignore = "run by test:plugins:guest, which compiles the wasm fixture"]
fn disable_replays_its_stable_key() {
	let enrollment = Uuid::from_u128(0x7777_7777_7777_7777_8777_7777_7777_7777);
	let mut service = Service::new();
	service.owned = vec![enrollment];
	let run = run(&guest(), 7, Vec::new(), service);

	assert_eq!(
		run.steps,
		vec!["disable=ok:done".to_owned(), "disable=ok:done".to_owned()]
	);
	let revokes: Vec<_> = run
		.service
		.log
		.iter()
		.filter(|line| line.starts_with("revoke("))
		.collect();
	assert_eq!(
		revokes,
		vec![
			&format!("revoke({enrollment}) key=notifications:disable:{enrollment} auth=…"),
			&format!("revoke({enrollment}) key=notifications:disable:{enrollment} auth=…"),
		],
		"a retried sign-out replays the same idempotency key"
	);
}
