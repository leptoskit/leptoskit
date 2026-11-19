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
//!   answer scripted APNs snapshots through the plugin's own codec, and method
//!   9 the device details the OS reports, and method 10 the batches of taps,
//!   shown notifications, and data messages a host received — and the service —
//!   the `http` plugin answers scripted JSON for the RPC routes the client
//!   really addresses, with a destination-deduping, quota-refusing,
//!   receipt-replaying stand-in that keeps the real service's ownership rules:
//!   a renewal of an enrollment the person no longer holds answers "gone" and
//!   registers nothing. No real APNs, no real server, no device.
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
use leptoskit_plugin_notifications::DeviceInfo;
use leptoskit_plugin_notifications::DeviceToken;
use leptoskit_plugin_notifications::FcmToken;
use leptoskit_plugin_notifications::ReceivedItem;
use leptoskit_plugin_notifications::ReceivedKind;
use leptoskit_plugin_notifications::RemoteRegistration;
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

/// What the fake OS answers for one `REMOTE_REGISTER`, in order. `Token` is
/// an APNs hex device token — iOS's answer — and `Fcm` is a registration
/// token in Firebase's own alphabet — Android's answer.
enum Answer {
	Token(String),
	Fcm(String),
	Failed(&'static str),
	Unconfirmed,
}

/// What the fake OS answers for one `DEVICE` read, in order; the last one
/// repeats, and with none scripted the device reports nothing.
/// `Unavailable` is a host that predates the method.
#[derive(Clone)]
enum Device {
	Reports(DeviceInfo),
	Unavailable,
}

/// The details a phone reports, with `app_version`.
fn phone(app_version: &str) -> Device {
	Device::Reports(DeviceInfo {
		locale: Some("en_GB".into()),
		time_zone: Some("Europe/London".into()),
		app_version: Some(app_version.into()),
	})
}

/// The fake service: what the RPC routes answer, and what crossed them.
struct Service {
	/// Destination dedupe: a token always answers the same endpoint id.
	destinations: BTreeMap<String, Uuid>,
	/// The next endpoint id the service mints, deterministic per run.
	minted: u128,
	/// The endpoints this person owns. A successful registration adds its
	/// endpoint, as the real service's does.
	owned: Vec<Uuid>,
	/// Whether the server revokes every enrollment as soon as it is made:
	/// another device's revoke, the app's server, or an erasure, landing
	/// between the gesture and the next launch.
	revokes_behind: bool,
	/// Whether the standing check answers a transport failure.
	standing_fails: bool,
	/// Per-register-call script: a failure, or quota, or success.
	register: Vec<Refusal>,
	/// Whether revoke calls answer a server error.
	revoke_fails: bool,
	/// Idempotency keys already receipted, and what they revoked.
	receipts: BTreeMap<String, Uuid>,
	/// One line per RPC: `operation [detail…] auth=… key=…`.
	log: Vec<String>,
	/// The device details each `register_push_endpoint` call carried, in
	/// order.
	details: Vec<serde_json::Value>,
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
			revokes_behind: false,
			standing_fails: false,
			register: Vec::new(),
			revoke_fails: false,
			receipts: BTreeMap::new(),
			log: Vec::new(),
			details: Vec::new(),
		}
	}

	/// One `register_push_endpoint` call. The credential is whatever the
	/// guest sent — APNs from an iOS host, FCM from an Android one — and
	/// the platform rides the reply, as the real service's would.
	fn register(&mut self, body: &str) -> (u16, String) {
		let body = body_json(body);
		let credential = &body["credential"];
		let (platform, destination) =
			if let Some(token) = credential["apns"]["device_token"].as_str() {
				("apns", token)
			} else if let Some(token) = credential["fcm"]["registration_token"].as_str() {
				("fcm", token)
			} else {
				panic!("the guest sent no credential the service can parse: {body}");
			};
		let destination = destination.to_owned();

		// A renewal stands on the enrollment it names, before anything
		// else is looked at: once the person no longer holds it, nothing
		// registers, in the service's own words.
		if let Some(renews) = body["renews"].as_str() {
			let renews: Uuid = renews.parse().expect("a uuid");

			if !self.owned.contains(&renews) {
				self.log.push(format!(
					"register({destination}, renews {renews}) gone auth=…"
				));
				return (
					404,
					"ServerError|this enrollment is gone: it was revoked or erased on the server, \
					 so nothing was registered; enrolling again takes the person's own gesture"
						.into(),
				);
			}
		}

		self.details.push(body["device"].clone());
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

		if !self.revokes_behind && !self.owned.contains(&id) {
			self.owned.push(id);
		}

		(
			200,
			format!(
				r#"{{"id":"{id}","platform":"{platform}","created_at":"2026-11-16T00:00:00Z","invalidated_at":null}}"#
			),
		)
	}

	/// One `push_endpoints` call: the first page of the endpoints the
	/// person owns, at most twenty, as the real service pages them. The
	/// enrollment flow has no business listing: it asks by id.
	fn list(&mut self) -> (u16, String) {
		self.log.push("list() auth=…".into());
		let items: Vec<String> = self
			.owned
			.iter()
			.take(20)
			.map(|id| {
				format!(
					r#"{{"id":"{id}","platform":"apns","created_at":"2026-11-16T00:00:00Z","invalidated_at":null}}"#
				)
			})
			.collect();
		(
			200,
			format!(
				r#"{{"items":[{}],"end":null,"has_more":{}}}"#,
				items.join(","),
				self.owned.len() > 20
			),
		)
	}

	/// One `push_endpoint_standing` call: whether the endpoint is still
	/// this person's.
	fn standing(&mut self, body: &str) -> (u16, String) {
		let endpoint: Uuid = body_json(body)["endpoint"]
			.as_str()
			.expect("the endpoint the guest asks about")
			.parse()
			.expect("a uuid");
		self.log.push(format!("standing({endpoint}) auth=…"));

		if self.standing_fails {
			return (0, String::new());
		}

		(200, self.owned.contains(&endpoint).to_string())
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
	/// The wait bound of each `RECEIVED` call the guest made, in order.
	waits: Vec<u32>,
}

/// What the fake host has received, as it answers each `RECEIVED` call in
/// order. `None` is a host that predates the method.
type Inbox = Option<Vec<Vec<ReceivedItem>>>;

/// Runs the guest once for `scenario`, playing the OS and service scripts.
fn run(wasm: &[u8], scenario: u32, os: Vec<Answer>, service: Service) -> Run {
	run_with(wasm, scenario, os, Vec::new(), service)
}

/// [`run`], with the device details the OS reports scripted too.
fn run_with(
	wasm: &[u8],
	scenario: u32,
	os: Vec<Answer>,
	devices: Vec<Device>,
	service: Service,
) -> Run {
	run_receiving(wasm, scenario, os, devices, None, service)
}

/// [`run_with`], with what the host received scripted too.
fn run_receiving(
	wasm: &[u8],
	scenario: u32,
	os: Vec<Answer>,
	devices: Vec<Device>,
	inbox: Inbox,
	mut service: Service,
) -> Run {
	const REPORT: &str = "notifications-guest-report";
	const ASK: MethodId = MethodId(0);
	const PANICKED: MethodId = MethodId(1);
	const STEP: MethodId = MethodId(2);
	const REMOTE_TOKEN: MethodId = MethodId(7);
	const REMOTE_REGISTER: MethodId = MethodId(8);
	const DEVICE: MethodId = MethodId(9);
	const RECEIVED: MethodId = MethodId(10);

	let mut host = WasmHost::<HeadlessBackend>::headless(wasm).expect("compile the guest");
	let report = register(&mut host, REPORT, &[ASK, PANICKED, STEP]);
	let mut methods = vec![REMOTE_TOKEN, REMOTE_REGISTER, DEVICE];
	if inbox.is_some() {
		methods.push(RECEIVED);
	}
	let notifications = register(&mut host, "notifications", &methods);
	let mut batches = inbox.unwrap_or_default().into_iter();
	let mut waits = Vec::new();
	let mut devices = devices.into_iter();
	let mut reported = Device::Reports(DeviceInfo::default());
	let http = register(&mut host, "http", &[MethodId(0)]);
	host.start(100.0, 100.0).expect("start the guest");

	// The OS's scripted answers, and the last snapshot it handed out.
	let mut answers = os.into_iter().map(|answer| {
		match answer {
			Answer::Token(hex) => {
				RemoteRegistration::new(
					1,
					RemoteState::Token(DeviceToken::new(hex).expect("a fake token in bounds")),
				)
				.expect("a snapshot")
				.encode()
				.expect("the plugin codec")
			}
			Answer::Fcm(fcm) => {
				RemoteRegistration::new(
					1,
					RemoteState::FcmToken(FcmToken::new(fcm).expect("a fake token in bounds")),
				)
				.expect("a snapshot")
				.encode()
				.expect("the plugin codec")
			}
			Answer::Failed(code) => {
				RemoteRegistration::new(1, RemoteState::Failed(code.into()))
					.expect("a snapshot")
					.encode()
					.expect("the plugin codec")
			}
			Answer::Unconfirmed => {
				RemoteRegistration::unconfirmed()
					.encode()
					.expect("the codec")
			}
		}
	});
	let mut latest = RemoteRegistration::none().encode().expect("the codec");

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
					DEVICE => {
						if let Some(next) = devices.next() {
							reported = next;
						}
						match &reported {
							Device::Reports(info) => Ok(info.encode().expect("the plugin codec")),
							Device::Unavailable => Err(Error::Unavailable),
						}
					}
					RECEIVED => {
						waits.push(
							leptoskit_plugin_notifications::decode_received_wait(&request.payload)
								.expect("a wait bound"),
						);
						let batch = batches
							.next()
							.expect("the scenario scripts every received call");
						Ok(ReceivedItem::encode_batch(&batch).expect("the plugin codec"))
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
			return Run {
				steps,
				service,
				waits,
			};
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
		"push_endpoint_standing" => service.standing(&body),
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

/// An FCM registration token, in Firebase's own printable alphabet. Fake
/// material: nothing here ever reaches a provider.
fn fcm_token(seed: u8) -> String {
	use std::fmt::Write as _;

	let mut fcm = String::new();
	for byte in 0..16_u8 {
		let _ = write!(fcm, "{}", char::from(b'a' + (byte.wrapping_add(seed) % 26)));
	}
	format!("{}:{}{}", fcm, "_-0", seed)
}

#[test]
#[ignore = "run by test:plugins:guest, which compiles the wasm fixture"]
fn a_gesture_enrolls_and_a_cold_relaunch_keeps_the_same_endpoint() {
	let first = token(0);
	let service = Service::new();
	let run = run_with(
		&guest(),
		0,
		vec![Answer::Token(first.clone()), Answer::Token(first.clone())],
		vec![phone("2.1.0")],
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
		1,
		"a relaunch with the same token and the same details registers nothing"
	);
	assert_eq!(
		run.service
			.log
			.iter()
			.filter(|line| !line.starts_with("register("))
			.collect::<Vec<_>>(),
		vec![&format!("standing({id}) auth=…")],
		"it asks once, by id, whether the enrollment still stands"
	);
	assert_eq!(
		run.service.details,
		vec![serde_json::json!({
			"locale": "en-GB",
			"time_zone": "Europe/London",
			"app_version": "2.1.0",
		})],
		"the enrollment carried the device's details, the locale in canonical case"
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

/// A previous handle that is not this person's — a session switch left it
/// behind — is gone like any enrollment the service no longer has for
/// them: the rotated token registers nothing under the new identity, and
/// the handle is never operated.
#[test]
#[ignore = "run by test:plugins:guest, which compiles the wasm fixture"]
fn a_previous_handle_of_another_account_registers_nothing() {
	let previous = Uuid::from_u128(0x5555_5555_5555_7555_8555_5555_5555_5555);
	let rotated = token(5);
	let mut service = Service::new();
	service.owned = Vec::new();
	let run = run(&guest(), 5, vec![Answer::Token(rotated.clone())], service);

	assert_eq!(run.steps, vec![format!("reconcile=ok:gone:{previous}")]);
	assert!(
		run.service.destinations.is_empty() && run.service.owned.is_empty(),
		"nothing was enrolled for an account that never opted in on this device"
	);
	assert!(
		!run.service
			.log
			.iter()
			.any(|line| line.starts_with("revoke(")),
		"a handle another account owns is never operated under this identity"
	);
}

/// A server-side revoke sticks. The person enrolls; the server then no
/// longer has the enrollment — revoked from another device's list, by the
/// app's server, or erased with the person. The next launch, with nothing
/// about the device changed, asks once, hears so, and registers nothing.
#[test]
#[ignore = "run by test:plugins:guest, which compiles the wasm fixture"]
fn an_unchanged_launch_learns_its_enrollment_is_gone_and_registers_nothing() {
	let first = token(11);
	let mut service = Service::new();
	service.revokes_behind = true;
	let run = run_with(
		&guest(),
		0,
		vec![Answer::Token(first.clone()), Answer::Token(first.clone())],
		vec![phone("2.1.0")],
		service,
	);
	let id = run.service.destinations[&first];

	assert_eq!(
		run.steps,
		vec![format!("enroll=ok:{id}"), format!("reconcile=ok:gone:{id}")],
		"the launch drops the enrollment the server no longer has"
	);
	assert_eq!(
		run.service.log,
		vec![
			format!("register({first}) = {id} auth=…"),
			format!("standing({id}) auth=…"),
		],
		"one read, and no registration after it: {:?}",
		run.service.log
	);
}

/// The same when the launch has something new to send — an app update: the
/// renewal names the enrollment it renews, the service registers nothing
/// in its place, and the launch answers gone rather than enrolling again.
#[test]
#[ignore = "run by test:plugins:guest, which compiles the wasm fixture"]
fn a_changed_launch_never_re_creates_a_revoked_enrollment() {
	let first = token(12);
	let mut service = Service::new();
	service.revokes_behind = true;
	let run = run_with(
		&guest(),
		9,
		vec![Answer::Token(first.clone()), Answer::Token(first.clone())],
		vec![phone("2.1.0"), phone("2.2.0")],
		service,
	);
	let id = run.service.destinations[&first];

	assert_eq!(
		run.steps,
		vec![format!("enroll=ok:{id}"), format!("reconcile=ok:gone:{id}")]
	);
	assert_eq!(
		run.service.log,
		vec![
			format!("register({first}) = {id} auth=…"),
			format!("register({first}, renews {id}) gone auth=…"),
		],
		"the update was sent as a renewal, and the service registered nothing"
	);
	assert_eq!(
		run.service.details.len(),
		1,
		"only the gesture's enrollment was ever stored"
	);
}

/// An unchanged launch that can't reach the service keeps its enrollment:
/// an unanswered question is not a revoke, and the next launch asks again.
#[test]
#[ignore = "run by test:plugins:guest, which compiles the wasm fixture"]
fn an_unanswered_standing_check_keeps_the_enrollment() {
	let first = token(13);
	let mut service = Service::new();
	service.standing_fails = true;
	let run = run_with(
		&guest(),
		0,
		vec![Answer::Token(first.clone()), Answer::Token(first.clone())],
		vec![phone("2.1.0")],
		service,
	);
	let id = run.service.destinations[&first];

	assert_eq!(
		run.steps,
		vec![format!("enroll=ok:{id}"), format!("reconcile=ok:kept:{id}")]
	);
}

/// Ownership is asked by id, so it is right however many endpoints the
/// person holds: with more than a page of them, a rotated token still
/// retires the previous handle rather than leaving it live as "stale".
#[test]
#[ignore = "run by test:plugins:guest, which compiles the wasm fixture"]
fn a_rotation_retires_a_handle_beyond_the_first_page_of_endpoints() {
	let previous = Uuid::from_u128(0x1111_1111_1111_7111_8111_1111_1111_1111);
	let rotated = token(14);
	let mut service = Service::new();
	// Twenty-five older endpoints, then this device's: past the twenty a
	// page of the listing holds.
	service.owned = (1..=25_u128)
		.map(|n| Uuid::from_u128(0x0A00_0000_0000_7000_8000_0000_0000_0000 | n))
		.chain([previous])
		.collect();
	let run = run(&guest(), 1, vec![Answer::Token(rotated.clone())], service);
	let current = run.service.destinations[&rotated];

	assert_eq!(
		run.steps,
		vec![format!("reconcile=ok:rotated:{current}:revoked:{previous}")],
		"the previous handle is this person's, wherever a listing would put it"
	);
	assert!(!run.service.owned.contains(&previous), "and it is revoked");
	assert!(
		!run.service.log.iter().any(|line| line.starts_with("list(")),
		"no listing is read: {:?}",
		run.service.log
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

/// Android's answer is an FCM registration token: the same gesture and
/// cold-relaunch flow enrolls it, and the exact Firebase bytes are the
/// destination the service dedupes on — nothing assumed hex or APNs.
#[test]
#[ignore = "run by test:plugins:guest, which compiles the wasm fixture"]
fn an_android_token_enrolls_and_relaunches_as_the_fcm_credential() {
	let first = fcm_token(0);
	let service = Service::new();
	let run = run(
		&guest(),
		0,
		vec![Answer::Fcm(first.clone()), Answer::Fcm(first.clone())],
		service,
	);
	let id = run.service.destinations[&first];

	assert_eq!(
		run.steps,
		vec![format!("enroll=ok:{id}"), format!("reconcile=ok:kept:{id}")]
	);
	assert_eq!(
		run.service.destinations.keys().collect::<Vec<_>>(),
		vec![&first],
		"the FCM token is the destination, not a hex token: {:?}",
		run.service.destinations
	);
	assert_eq!(
		run.service
			.log
			.iter()
			.filter(|line| line.starts_with("register("))
			.count(),
		1,
		"a relaunch with the same token and the same details sends nothing"
	);
	assert_eq!(
		run.service.details,
		vec![serde_json::json!({})],
		"a device that reported nothing sends no details"
	);
	assert!(
		!run.service
			.log
			.iter()
			.any(|line| line.starts_with("revoke(")),
		"nothing needs retiring when the destination is unchanged"
	);
}

/// A rotated FCM token enrolls a new endpoint and retires the previous
/// handle under its owner — rotation logic shared with APNs, on a
/// credential whose kind the snapshot itself named.
#[test]
#[ignore = "run by test:plugins:guest, which compiles the wasm fixture"]
fn a_rotated_fcm_token_enrolls_and_retires_the_previous_handle() {
	let previous = Uuid::from_u128(0x1111_1111_1111_7111_8111_1111_1111_1111);
	let rotated = fcm_token(1);
	let mut service = Service::new();
	service.owned = vec![previous];
	let run = run(&guest(), 1, vec![Answer::Fcm(rotated.clone())], service);
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

/// An Android host whose Firebase is not configured refuses with its stable
/// code, and nothing reaches the service.
#[test]
#[ignore = "run by test:plugins:guest, which compiles the wasm fixture"]
fn an_unconfigured_firebase_refuses_with_its_code_and_registers_nothing() {
	let run = run(
		&guest(),
		8,
		vec![Answer::Failed("firebase-not-configured")],
		Service::new(),
	);

	assert_eq!(
		run.steps,
		vec!["enroll=err:failed:firebase-not-configured".to_owned()]
	);
	assert!(
		run.service.log.is_empty(),
		"nothing reached the service: {:?}",
		run.service.log
	);
}

/// An app update reaches the service at the next launch: the same token
/// with a new app version sends the new details under the same endpoint,
/// and the launch after that, with nothing changed, sends nothing.
#[test]
#[ignore = "run by test:plugins:guest, which compiles the wasm fixture"]
fn changed_details_are_sent_once_under_the_same_endpoint() {
	let first = token(9);
	let run = run_with(
		&guest(),
		9,
		vec![
			Answer::Token(first.clone()),
			Answer::Token(first.clone()),
			Answer::Token(first.clone()),
		],
		vec![phone("2.1.0"), phone("2.2.0")],
		Service::new(),
	);
	let id = run.service.destinations[&first];

	assert_eq!(
		run.steps,
		vec![
			format!("enroll=ok:{id}"),
			format!("reconcile=ok:kept:{id}"),
			format!("reconcile=ok:kept:{id}"),
		]
	);
	let versions: Vec<_> = run
		.service
		.details
		.iter()
		.map(|details| details["app_version"].clone())
		.collect();
	assert_eq!(
		versions,
		vec![serde_json::json!("2.1.0"), serde_json::json!("2.2.0")],
		"the enrollment and the update each sent once; the unchanged launch sent nothing"
	);
	assert!(
		!run.service
			.log
			.iter()
			.any(|line| line.starts_with("revoke(")),
		"a details change retires nothing"
	);
}

/// An enrollment the app saved before device details existed — no record
/// of what it sent — sends once at the next launch, keeps its endpoint, and
/// then sends nothing. A host that predates the device method enrolls with
/// no details at all.
#[test]
#[ignore = "run by test:plugins:guest, which compiles the wasm fixture"]
fn an_enrollment_saved_before_details_sends_once() {
	let first = token(10);
	let mut service = Service::new();
	// The enrollment the app saved is one the service still holds.
	service.owned = vec![Uuid::from_u128(0x0E5A_0000_0000_0000_0000_0000_0000_0001)];
	let run = run_with(
		&guest(),
		10,
		vec![Answer::Token(first.clone()), Answer::Token(first.clone())],
		vec![Device::Unavailable],
		service,
	);
	let id = run.service.destinations[&first];

	assert_eq!(
		id,
		Uuid::from_u128(0x0E5A_0000_0000_0000_0000_0000_0000_0001)
	);
	assert_eq!(
		run.steps,
		vec![
			format!("reconcile=ok:kept:{id}"),
			format!("reconcile=ok:kept:{id}"),
		]
	);
	assert_eq!(
		run.service.details,
		vec![serde_json::json!({})],
		"one send, with no details from a host that predates the method"
	);
}

/// One received item, as a host builds it.
fn item(kind: ReceivedKind, action: &str, payload: &serde_json::Value) -> ReceivedItem {
	ReceivedItem::new(kind, action, "", payload.to_string()).expect("an item in bounds")
}

#[test]
#[ignore = "run by test:plugins:guest, which compiles the wasm fixture"]
fn taps_shown_notifications_and_data_messages_reach_the_app_in_order() {
	let launch = "0190a1b2-0000-7000-8000-000000000001";
	let later = "0190a1b2-0000-7000-8000-000000000002";
	let silent = "0190a1b2-0000-7000-8000-000000000003";
	let run = run_receiving(
		&guest(),
		11,
		Vec::new(),
		Vec::new(),
		Some(vec![
			// Waiting before the app asked: the tap that launched it, in
			// the APNs payload's shape.
			vec![item(
				ReceivedKind::Opened,
				"",
				&serde_json::json!({
					"aps": { "alert": { "title": "Paid", "body": "Invoice 42" } },
					"lkn": launch,
					"lklink": "/invoice/42",
					"lk": { "invoice": 42 },
				}),
			)],
			// A wait that ended with nothing.
			Vec::new(),
			// While the app runs: a data message and a foreground
			// notification in FCM's data-map shape, then a tap on an
			// action button whose send had no link.
			vec![
				item(
					ReceivedKind::Message,
					"",
					&serde_json::json!({
						"lkn": silent,
						"sync": "orders",
						"lkbadge": "3",
						"google.message_id": "0:1",
					}),
				),
				item(
					ReceivedKind::Presented,
					"",
					&serde_json::json!({ "lkn": later, "invoice": "43" }),
				),
				item(
					ReceivedKind::Opened,
					"approve",
					&serde_json::json!({ "lkn": later, "invoice": "43" }),
				),
			],
		]),
		Service::new(),
	);

	assert_eq!(
		run.steps,
		vec![
			format!("opened:{launch}:/invoice/42:-:launched:{{\"invoice\":42}}"),
			format!("data:{silent}:3:{{\"sync\":\"orders\"}}"),
			format!("shown:{later}:{{\"invoice\":\"43\"}}"),
			format!("opened:{later}:/inbox?lkn={later}:approve:running:{{\"invoice\":\"43\"}}"),
		]
	);
	assert_eq!(
		run.waits,
		vec![0, 10_000, 10_000],
		"the first ask never waits, so what it answers is what launched the app"
	);
	assert!(run.service.log.is_empty(), "receiving calls no service");
}

#[test]
#[ignore = "run by test:plugins:guest, which compiles the wasm fixture"]
fn a_host_that_predates_receiving_answers_unavailable() {
	let run = run_receiving(&guest(), 12, Vec::new(), Vec::new(), None, Service::new());

	assert_eq!(run.steps, vec!["received=err:unavailable".to_owned()]);
	assert!(run.waits.is_empty());
}
