//! The reserved timer plugin: a host schedules real sleeps, cancels them
//! when the guest drops a call, and never pretends an unsupported host
//! completed at once.

use std::time::Duration;
use std::time::Instant;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::CallId;
use leptoskit_bridge::timer::SLEEP;
use leptoskit_bridge::timer::TIMER_PLUGIN;
use leptoskit_bridge::timer::encode;
use leptoskit_native::EventOutcome;
use leptoskit_wasm::EventKind;
use leptoskit_wasm::WasmHost;

/// The smallest guest the ABI accepts. The test plays the app that submits
/// timer calls through the bridge.
const IDLE: &str = r#"
(module
  (func (export "leptoskit_main"))
  (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) (i32.const 0)))
"#;

fn host() -> WasmHost<leptoskit_native::HeadlessBackend> {
	let mut host = WasmHost::headless(IDLE.as_bytes()).expect("compile wat");
	host.enable_timers().expect("enable timers");
	host.start(100.0, 100.0).expect("start");
	host
}

fn submit(host: &mut WasmHost<leptoskit_native::HeadlessBackend>, millis: u64) -> CallId {
	let timer = host.plugins().resolve(TIMER_PLUGIN).expect("timer plugin");
	let payload = encode(Duration::from_millis(millis)).expect("encode");
	host.plugins()
		.submit(timer, SLEEP, Bytes::copy_from_slice(&payload))
		.expect("submit")
}

fn pump_until_due(
	host: &mut WasmHost<leptoskit_native::HeadlessBackend>,
	deadline: Instant,
) -> bool {
	loop {
		let woke = host.pump_timers().expect("pump");

		if woke || Instant::now() >= deadline {
			return woke;
		}

		std::thread::sleep(Duration::from_millis(2));
	}
}

#[test]
fn a_timer_request_is_scheduled_off_the_platform_and_completes_when_due() {
	let mut host = host();
	let id = submit(&mut host, 10);
	host.pump_timers().expect("route");
	assert!(
		host.plugins().next_request().is_none(),
		"a timer never reaches the platform"
	);
	assert!(host.next_timer_deadline().is_some());

	let started = Instant::now();
	assert!(pump_until_due(
		&mut host,
		Instant::now() + Duration::from_secs(1)
	));
	assert!(
		started.elapsed() >= Duration::from_millis(5),
		"it really waited"
	);
	assert!(host.next_timer_deadline().is_none());

	let reply = host
		.plugins()
		.take_result(id)
		.expect("take")
		.expect("a reply")
		.expect("success");
	assert!(reply.is_empty());
}

#[test]
fn dropping_or_cancelling_a_timer_stops_it_before_its_deadline() {
	let mut host = host();
	let id = submit(&mut host, 30);
	host.pump_timers().expect("route");
	assert!(host.cancel_timer(id));
	assert!(!host.cancel_timer(id));
	std::thread::sleep(Duration::from_millis(50));
	assert!(!host.pump_timers().expect("pump"));
	assert!(host.next_timer_deadline().is_none());
}

#[test]
fn a_bridge_cancellation_reaches_the_host_timer() {
	let mut host = host();
	let id = submit(&mut host, 30);
	host.pump_timers().expect("route");
	host.plugins().cancel(id).expect("cancel the running call");
	host.pump_timers().expect("drain the cancellation");
	assert!(host.next_timer_deadline().is_none());
}

#[test]
fn replacing_the_guest_drops_old_timers_and_keeps_the_capability() {
	let mut host = host();
	let old = submit(&mut host, 10);
	host.pump_timers().expect("route");
	host.replace_guest(IDLE.as_bytes()).expect("replace");
	assert!(host.timers_enabled());
	assert!(host.next_timer_deadline().is_none());
	std::thread::sleep(Duration::from_millis(30));
	assert!(!host.pump_timers().expect("pump"));
	assert!(
		host.plugins().take_result(old).is_err(),
		"the old call cannot be revived"
	);
}

#[test]
fn a_host_without_timers_reports_unavailable() {
	let mut host = WasmHost::headless(IDLE.as_bytes()).expect("compile wat");
	assert!(!host.timers_enabled());
	assert!(host.plugins().resolve(TIMER_PLUGIN).is_none());
	host.start(100.0, 100.0).expect("start");
	assert!(
		host.plugins().next_request().is_none(),
		"an old host queues no timer work"
	);

	// A guest that resolves the timer sees the explicit error code.
	let guest = r#"
	(module
	  (import "leptoskit_plugins_v1" "resolve" (func $resolve (param i32 i32) (result i64)))
	  (memory (export "memory") 1)
	  (data (i32.const 0) "leptoskit.timer")
	  (func (export "leptoskit_main")
	    (if (i64.ne (call $resolve (i32.const 0) (i32.const 15)) (i64.const -2))
	      (then unreachable)))
	  (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) (i32.const 0)))
	"#;
	let mut old = WasmHost::headless(guest.as_bytes()).expect("compile wat");
	old.start(100.0, 100.0)
		.expect("resolve reports Unavailable");
}

#[test]
fn a_timer_only_guest_sees_the_capability_after_enable() {
	let guest = r#"
	(module
	  (import "leptoskit_plugins_v1" "resolve" (func $resolve (param i32 i32) (result i64)))
	  (memory (export "memory") 1)
	  (data (i32.const 0) "leptoskit.timer")
	  (func (export "leptoskit_main")
	    (if (i64.lt_s (call $resolve (i32.const 0) (i32.const 15)) (i64.const 0))
	      (then unreachable)))
	  (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) (i32.const 0)))
	"#;
	let mut host = WasmHost::headless(guest.as_bytes()).expect("compile wat");
	host.enable_timers().expect("enable timers");
	host.start(100.0, 100.0).expect("resolve the timer");
}

#[test]
fn a_rejected_timer_wakes_the_guest_without_waiting_for_a_deadline() {
	let guest = r#"
	(module
	  (global $woken (mut i32) (i32.const 0))
	  (func (export "leptoskit_main"))
	  (func (export "leptoskit_plugins_ready") (global.set $woken (i32.const 1)))
	  (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32)
	    (global.get $woken)))
	"#;
	let mut host = WasmHost::headless(guest.as_bytes()).expect("compile wat");
	host.enable_timers().expect("enable timers");
	let wakes = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
	let counted = wakes.clone();
	host.set_timer_wake(move || {
		counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
	});
	host.start(100.0, 100.0).expect("start");
	let timer = host.plugins().resolve(TIMER_PLUGIN).expect("timer plugin");
	let id = host
		.plugins()
		.submit(timer, SLEEP, Bytes::new())
		.expect("submit");

	assert!(
		host.next_host_request().is_none(),
		"timer stays in the host"
	);
	assert_eq!(wakes.load(std::sync::atomic::Ordering::SeqCst), 1);
	assert_eq!(host.next_timer_deadline(), Some(Duration::ZERO));
	assert!(host.poll_due_timers().expect("deliver the rejection"));
	assert_eq!(
		host.plugins()
			.take_result(id)
			.expect("result")
			.expect("reply"),
		Err(leptoskit_bridge::Error::InvalidArgument)
	);
	assert_eq!(
		host.dispatch_wire(0, EventKind::Press, None)
			.expect("inspect wake"),
		EventOutcome::Handled
	);
	assert!(host.next_timer_deadline().is_none());
	assert!(!host.pump_timers().expect("already delivered"));
}

/// Run by `devenv shell test:plugins:guest`, which compiles the Rust guest.
/// The fixture sleeps for real, drops an abandoned sleep, and reports both.
#[test]
#[ignore = "run by test:plugins:guest, which compiles the Rust wasm fixture"]
fn compiled_rust_guest_sleeps_and_cancels() {
	let path = std::env::var("LEPTOSKIT_TIMER_GUEST").expect("fixture wasm path");
	let wasm = std::fs::read(path).expect("read fixture");
	let mut host = WasmHost::headless(&wasm).expect("compile fixture");
	register_panic_report(&mut host);
	host.enable_timers().expect("enable timers");
	let started = Instant::now();
	let result = host.start(100.0, 100.0);
	guest_result(&mut host, result);

	// The guest starts a sleep in `main`; route it and wait for its deadline.
	let deadline = Instant::now() + Duration::from_secs(2);
	let mut done = false;

	while Instant::now() < deadline {
		let result = host.pump_timers();
		guest_result(&mut host, result);
		done = host
			.dispatch_wire(0, EventKind::Press, None)
			.expect("poll the guest")
			== EventOutcome::Handled;

		if done {
			break;
		}

		std::thread::sleep(Duration::from_millis(5));
	}

	assert!(done, "the guest's sleep completed and its assertion ran");
	assert!(
		started.elapsed() >= Duration::from_millis(5),
		"the guest really waited"
	);
	assert!(
		host.next_timer_deadline().is_none(),
		"the abandoned one-second sleep was cancelled"
	);
	assert!(host.plugins().next_request().is_none());
	assert_eq!(host.plugins().buffered_bytes(), 0);
}

#[test]
#[ignore = "run by test:plugins:guest, which compiles the Rust wasm fixture"]
fn compiled_rust_guest_reports_missing_timer_as_an_error() {
	let path = std::env::var("LEPTOSKIT_TIMER_GUEST").expect("fixture wasm path");
	let wasm = std::fs::read(path).expect("read fixture");
	let mut host = WasmHost::headless(&wasm).expect("compile fixture");
	register_panic_report(&mut host);
	let result = host.start(100.0, 100.0);
	guest_result(&mut host, result);
	assert_eq!(
		host.dispatch_wire(0, EventKind::Press, None)
			.expect("inspect result"),
		EventOutcome::Handled
	);
	assert!(host.plugins().next_request().is_none());
	assert!(host.next_timer_deadline().is_none());
}

fn register_panic_report(host: &mut WasmHost<leptoskit_native::HeadlessBackend>) {
	let method = leptoskit_bridge::MethodId(0);
	let plugin = host
		.plugins()
		.register("timer-test-panic", [method])
		.expect("panic reporter");
	host.plugins()
		.grant(plugin, method, true)
		.expect("grant panic reporter");
}

fn guest_result<T>(
	host: &mut WasmHost<leptoskit_native::HeadlessBackend>,
	result: Result<T, leptoskit_wasm::WasmError>,
) -> T {
	result.unwrap_or_else(|error| {
		let details: Vec<_> = std::iter::from_fn(|| host.plugins().next_request())
			.map(|request| String::from_utf8_lossy(&request.payload).into_owned())
			.collect();
		panic!("guest failed: {error}; pending reports: {details:?}");
	})
}

#[test]
fn a_full_request_batch_does_not_delay_later_short_timers() {
	let mut host = host();
	for _ in 0..99 {
		submit(&mut host, 60_000);
	}
	let short = submit(&mut host, 0);
	let woke = host.pump_timers().expect("route all admitted timers");
	assert!(
		host.plugins().next_request().is_none(),
		"later timers must not wait for the first long deadline"
	);
	assert!(
		woke || host
			.next_timer_deadline()
			.is_some_and(|delay| delay <= Duration::from_millis(10))
	);
	if !woke {
		assert!(pump_until_due(
			&mut host,
			Instant::now() + Duration::from_secs(1)
		));
	}
	assert!(
		host.plugins()
			.take_result(short)
			.expect("call")
			.expect("reply")
			.is_ok()
	);
}

#[test]
fn a_full_cancellation_batch_does_not_leave_long_timers_waiting() {
	let mut host = host();
	let calls: Vec<_> = (0..100).map(|_| submit(&mut host, 60_000)).collect();
	host.pump_timers().expect("route");
	for id in calls {
		host.plugins().cancel(id).expect("cancel");
	}
	host.pump_timers().expect("drain every cancellation");
	assert!(host.next_timer_deadline().is_none());
	assert!(host.plugins().next_cancelled().is_none());
}

#[test]
fn exhausting_the_pump_budget_requests_one_more_turn_then_returns_to_the_deadline() {
	let mut host = WasmHost::headless(IDLE.as_bytes()).expect("compile wat");
	*host.plugins() = leptoskit_bridge::Bridge::new(leptoskit_bridge::Limits {
		calls: 600,
		..Default::default()
	});
	host.enable_timers().expect("timers");
	host.start(100.0, 100.0).expect("start");
	let mut last = None;
	for _ in 0..600 {
		last = Some(submit(&mut host, 60_000));
	}
	host.pump_timers().expect("bounded first turn");
	assert_eq!(host.next_timer_deadline(), Some(Duration::ZERO));
	host.pump_timers().expect("remaining queued work");
	assert!(host.plugins().next_request().is_none());
	assert!(
		host.next_timer_deadline()
			.is_some_and(|delay| delay > Duration::from_secs(1))
	);
	assert_eq!(
		host.plugins()
			.take_result(last.unwrap())
			.expect("last call")
			.expect("completed"),
		Err(leptoskit_bridge::Error::Busy)
	);
}
