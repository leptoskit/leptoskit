//! Compiled guest acceptance for the notifications native enrollment: the
//! real `native` module — its real plugin guest transport, its real RPC
//! client over the real HTTP plugin guest — run by the interpreter while
//! the test plays the OS (`notifications` methods 7 to 10) and the service
//! (`http`), scripted per scenario
//! (`crates/leptoskit_wasm/tests/notifications_guest.rs`).
//!
//! The guest asks the test's report plugin which scenario it runs, reports
//! each step as a short stable line, and the test asserts on those lines
//! and on what crossed its fake boundaries. Panics are reported as text.
#![cfg(target_arch = "wasm32")]
#![allow(unsafe_code)] // Exported C names form the interpreter's entry points.

use std::cell::Cell;
use std::cell::RefCell;

use futures::executor::LocalPool;
use futures::executor::LocalSpawner;
use futures::task::LocalSpawnExt as _;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::guest::Client;
use leptoskit_notifications::native;
use leptoskit_notifications::native::EnrollError;
use leptoskit_notifications::native::Enrollment;
use leptoskit_notifications::native::Outcome;
use leptoskit_notifications::native::Retirement;
use leptoskit_notifications::receive::ReceiveError;
use leptoskit_notifications::receive::Received;
use leptoskit_rpc::FnError;
use leptoskit_rpc::client::ClientConfig;
use leptoskit_rpc::client::Credentials;

/// The plugin the test registers to hand the scenario in and hear steps.
const REPORT: &str = "notifications-guest-report";
const ASK_SCENARIO: u32 = 0;
const PANICKED: u32 = 1;
const STEP: u32 = 2;

thread_local! {
	static POOL: RefCell<LocalPool> = RefCell::new(LocalPool::new());
	static SPAWNER: LocalSpawner = POOL.with_borrow(LocalPool::spawner);
	static DONE: Cell<bool> = const { Cell::new(false) };
}

fn report(method: u32, message: &[u8]) {
	// Only the panic hook uses this synchronous form: it fires once, and a
	// forgotten reply costs one bridge slot. Everything else awaits.
	if let Ok(client) = Client::new(REPORT)
		&& let Ok(call) = client.call(MethodId(method), message)
	{
		std::mem::forget(call);
	}
}

/// Reports and drains the reply, so the bridge slot it borrowed is freed.
async fn send(method: u32, message: &[u8]) {
	if let Ok(client) = Client::new(REPORT)
		&& let Ok(call) = client.call(MethodId(method), message)
	{
		let _ = call.await;
	}
}

#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_main() {
	std::panic::set_hook(Box::new(|info| {
		report(PANICKED, info.to_string().as_bytes());
	}));
	any_spawner::Executor::init_local_custom_executor(Executor).expect("install executor");
	SPAWNER.with(|spawner| {
		spawner
			.spawn_local(async {
				leptoskit_rpc::client::configure(
					ClientConfig::new()
						.origin("https://service.test")
						.service_credentials("notifications", ProbeToken),
				);

				let scenario = ask_scenario().await;
				run_scenario(scenario).await;
				DONE.with(|done| done.set(true));
			})
			.expect("spawn acceptance");
	});
	leptoskit_poll();
}

/// Asks the test which scenario this run plays; a missing report plugin
/// fails the run loudly rather than guessing.
async fn ask_scenario() -> u32 {
	let reply = Client::new(REPORT)
		.expect("the report plugin is registered")
		.call(MethodId(ASK_SCENARIO), &[])
		.expect("the scenario is asked")
		.await
		.expect("the scenario is answered");
	u32::from_le_bytes(reply.as_ref().try_into().expect("a u32 scenario"))
}

/// A fixed bearer: identity is the app's session, and the fake service only
/// checks that whatever the guest sends arrives signed.
struct ProbeToken;

impl Credentials for ProbeToken {
	fn authorization(&self) -> leptoskit_query::BoxFuture<'_, Result<Option<String>, FnError>> {
		Box::pin(std::future::ready(Ok(Some(
			"Bearer probe-token".to_owned(),
		))))
	}
}

/// One stable line per step, so the test asserts on meaning instead of
/// `Debug` text that carries messages.
fn step(name: &str, result: &Result<impl Steps, EnrollError>) -> String {
	match result {
		Ok(outcome) => format!("{name}=ok:{}", outcome.line()),
		Err(error) => format!("{name}=err:{}", error_line(error)),
	}
}

/// What each result type renders as its one stable line.
trait Steps {
	fn line(&self) -> String;
}

impl Steps for Enrollment {
	fn line(&self) -> String {
		self.endpoint.to_string()
	}
}

impl Steps for Outcome {
	fn line(&self) -> String {
		match self {
			Outcome::Off => "off".into(),
			Outcome::Kept(enrollment) => format!("kept:{}", enrollment.endpoint),
			Outcome::Rotated { current, retired } => {
				format!(
					"rotated:{}:{}",
					current.endpoint,
					match retired {
						Retirement::Revoked(handle) => format!("revoked:{}", handle.endpoint),
						Retirement::Stale(handle) => format!("stale:{}", handle.endpoint),
					}
				)
			}
			Outcome::Gone(enrollment) => format!("gone:{}", enrollment.endpoint),
		}
	}
}

impl Steps for () {
	fn line(&self) -> String {
		"done".into()
	}
}

/// The error's own stable line: variant first, then the handles it names
/// and the failure class, so retry state is part of the assertion.
fn error_line(error: &EnrollError) -> String {
	match error {
		EnrollError::Unavailable => "unavailable".into(),
		EnrollError::NoPlugin => "no-plugin".into(),
		EnrollError::Register(reason) => format!("register:{reason}"),
		EnrollError::Failed(code) => format!("failed:{code}"),
		EnrollError::Unconfirmed => "unconfirmed".into(),
		EnrollError::Enroll(keep, cause) => {
			format!(
				"enroll:{}:{}",
				keep.map_or("none".into(), |e| e.endpoint.to_string()),
				class(cause)
			)
		}
		EnrollError::RetireFailed {
			current,
			retired,
			cause,
		} => {
			format!(
				"retire:{}:{}:{}",
				current.map_or("none".into(), |e| e.endpoint.to_string()),
				retired.endpoint,
				class(cause)
			)
		}
		EnrollError::OwnershipUnknown {
			current,
			retired,
			cause,
		} => {
			format!(
				"owner:{}:{}:{}",
				current.map_or("none".into(), |e| e.endpoint.to_string()),
				retired.endpoint,
				class(cause)
			)
		}
	}
}

/// The failure class of an RPC error: all the test needs to assert on.
fn class(cause: &FnError) -> &'static str {
	match cause {
		FnError::Network(_) => "network",
		FnError::Invalid(_) => "invalid",
		FnError::Server(_) => "server",
		FnError::Decode(_) => "decode",
		FnError::Other(_) => "other",
	}
}

/// One launch reconciliation against a fixed previous handle, reported.
async fn reconcile(previous: &str) {
	let previous = Enrollment {
		endpoint: previous.parse().unwrap(),
		sent: None,
	};
	report_step("reconcile", &native::reconcile(Some(&previous)).await).await;
}

async fn run_scenario(scenario: u32) {
	match scenario {
		// A gesture enrolls; a cold relaunch with the same token and the
		// same device details keeps the same endpoint and sends nothing.
		0 => {
			let enrolled = native::enroll().await;
			report_step("enroll", &enrolled).await;
			let enrollment = enrolled.expect("the gesture enrolls");
			report_step("reconcile", &native::reconcile(Some(&enrollment)).await).await;
		}
		// A rotated token enrolls a new endpoint and retires the previous
		// handle under its owner.
		1 => reconcile("11111111-1111-7111-8111-111111111111").await,
		// The person is at the endpoint quota when the token rotates.
		2 => reconcile("22222222-2222-7222-8222-222222222222").await,
		// The rotated enrollment succeeds; the previous handle's revoke
		// does not.
		3 => reconcile("33333333-3333-7333-8333-333333333333").await,
		// Registering fails outright; then the quota detour retires the
		// previous handle and the re-registration still fails.
		4 => {
			reconcile("44444444-4444-7444-8444-444444444444").await;
			reconcile("44444444-4444-7444-8444-444444444444").await;
		}
		// The previous handle belongs to a previous account: the service
		// answers that this person holds no such enrollment, and nothing is
		// registered or revoked.
		5 => reconcile("55555555-5555-7555-8555-555555555555").await,
		// The OS stays silent: unconfirmed, everything untouched, and a
		// device with no previous enrollment registers nothing at all.
		6 => {
			reconcile("66666666-6666-7666-8666-666666666666").await;
			report_step("reconcile", &native::reconcile(None).await).await;
		}
		// Sign-out revokes with the stable key, and a replay is safe.
		7 => {
			let enrollment = Enrollment {
				endpoint: "77777777-7777-7777-8777-777777777777".parse().unwrap(),
				sent: None,
			};
			report_unit("disable", &disable(&enrollment).await).await;
			report_unit("disable", &disable(&enrollment).await).await;
		}
		// An Android host whose Firebase is not configured — the app
		// generated without the push opt-in, or without its
		// google-services.json — refuses with its stable code, and nothing
		// reaches the service.
		8 => report_step("enroll", &native::enroll().await).await,
		// A gesture enrolls; the app updates, so the next launch sends the
		// new details under the same endpoint, and the launch after that
		// sends nothing.
		9 => {
			let enrolled = native::enroll().await;
			report_step("enroll", &enrolled).await;
			let enrollment = enrolled.expect("the gesture enrolls");
			let updated = native::reconcile(Some(&enrollment)).await;
			report_step("reconcile", &updated).await;
			let Ok(Outcome::Kept(kept)) = updated else {
				return;
			};
			report_step("reconcile", &native::reconcile(Some(&kept)).await).await;
		}
		// An enrollment saved before device details existed sends once,
		// keeps its endpoint, and then sends nothing.
		10 => {
			let saved = Enrollment {
				endpoint: "0e5a0000-0000-0000-0000-000000000001".parse().unwrap(),
				sent: None,
			};
			let first = native::reconcile(Some(&saved)).await;
			report_step("reconcile", &first).await;
			let Ok(Outcome::Kept(kept)) = first else {
				return;
			};
			report_step("reconcile", &native::reconcile(Some(&kept)).await).await;
		}
		// What the host received reaches the app in order: the tap that
		// launched it first, then what arrives while it runs.
		11 => {
			let mut receiver = native::Receiver::new();

			for _ in 0..4 {
				send(STEP, received_line(receiver.next().await).as_bytes()).await;
			}
		}
		// A host without the receiving method answers that it has none.
		12 => {
			let mut receiver = native::Receiver::new();
			send(STEP, received_line(receiver.next().await).as_bytes()).await;
		}
		// Each tap is recorded as an open with one call, as the signed-in
		// person: a tap with an id, the same tap again, a tap the service
		// refuses, and a local notification's tap, which calls nothing.
		13 => {
			let mut receiver = native::Receiver::new();

			for _ in 0..4 {
				let line = match receiver.next().await {
					Ok(Received::Opened(opened)) => {
						match opened.mark_opened().await {
							Ok(true) => "marked=ok:recorded".to_owned(),
							Ok(false) => "marked=ok:nothing".to_owned(),
							Err(error) => format!("marked=err:{}", class(&error)),
						}
					}
					other => received_line(other),
				};
				send(STEP, line.as_bytes()).await;
			}
		}
		other => report(PANICKED, format!("unknown scenario {other}").as_bytes()),
	}
}

/// One received event as a stable line.
fn received_line(result: Result<Received, ReceiveError>) -> String {
	let id = |id: Option<uuid::Uuid>| id.map_or("-".to_owned(), |id| id.to_string());
	let json = |data: &serde_json::Map<String, serde_json::Value>| {
		serde_json::Value::Object(data.clone()).to_string()
	};

	match result {
		Ok(Received::Opened(opened)) => {
			format!(
				"opened:{}:{}:{}:{}:{}",
				id(opened.notification),
				opened.route("/inbox"),
				opened.action.as_deref().unwrap_or("-"),
				if opened.launched {
					"launched"
				} else {
					"running"
				},
				json(&opened.data),
			)
		}
		Ok(Received::Shown(shown)) => {
			format!("shown:{}:{}", id(shown.notification), json(&shown.data))
		}
		Ok(Received::Data(message)) => {
			format!(
				"data:{}:{}:{}",
				id(message.notification),
				message
					.badge
					.map_or("-".to_owned(), |badge| badge.to_string()),
				json(&message.data),
			)
		}
		Err(ReceiveError::Unavailable) => "received=err:unavailable".into(),
		Err(ReceiveError::NoPlugin) => "received=err:no-plugin".into(),
		Err(ReceiveError::Host(reason)) => format!("received=err:host:{reason}"),
	}
}

/// Disabling with the module's own error shape reported as a line.
async fn disable(enrollment: &Enrollment) -> Result<(), String> {
	native::disable(enrollment)
		.await
		.map_err(|error| error.to_string())
}

async fn report_step(name: &str, result: &Result<impl Steps, EnrollError>) {
	send(STEP, step(name, result).as_bytes()).await;
}

async fn report_unit(name: &str, result: &Result<(), String>) {
	let line = match result {
		Ok(()) => format!("{name}=ok:done"),
		Err(error) => format!("{name}=err:{error}"),
	};
	send(STEP, line.as_bytes()).await;
}

/// The host calls this after each plugin reply, through `WasmHost`.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_poll() {
	POOL.with_borrow_mut(LocalPool::run_until_stalled);
}

#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_event(_id: i64, _kind: i32, _ptr: i32, _len: i32) -> i32 {
	leptoskit_poll();
	DONE.with(|done| i32::from(done.get()))
}

struct Executor;

impl any_spawner::CustomExecutor for Executor {
	fn spawn(&self, task: any_spawner::PinnedFuture<()>) {
		self.spawn_local(task);
	}

	fn spawn_local(&self, task: any_spawner::PinnedLocalFuture<()>) {
		SPAWNER
			.with(|spawner| spawner.spawn_local(task))
			.expect("spawn task");
	}

	fn poll_local(&self) {}
}
